//! Sign-in methods for an SSH connection (CRED-01..04): a key file, key text
//! from the keychain (optionally with an OpenSSH user certificate), a
//! password, or a key held by the Windows OpenSSH agent.
//!
//! Everything here runs after the host key was accepted, so no credential
//! reaches a machine that failed the host-key check. Error messages never
//! contain a password, a passphrase, key material or a key path.

use crate::core_err;
use anyhow::Result;
use russh::client::{Handle, Handler, KeyboardInteractiveAuthResponse};
use russh::keys::{decode_secret_key, load_secret_key, Certificate, PrivateKey};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg};
use russh::MethodKind;
use senvakara_core::ErrorCode;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

/// Secret text that is wiped from memory when dropped.
pub type Secret = Zeroizing<String>;

/// Named pipe of the Windows OpenSSH agent service (ADR-027).
#[cfg(windows)]
const AGENT_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";

/// Most keyboard-interactive rounds answered before giving up, so a server
/// cannot keep the sign-in going forever.
const MAX_INTERACTIVE_ROUNDS: usize = 4;

/// How to sign in once the host key has been accepted.
#[derive(Clone)]
pub enum Auth {
    /// A private key file on this PC.
    KeyFile {
        path: PathBuf,
        passphrase: Option<Secret>,
    },
    /// Private key text (OpenSSH, PEM or PuTTY format), with the OpenSSH user
    /// certificate line that goes with it, if any.
    KeyText {
        private_key: Secret,
        passphrase: Option<Secret>,
        certificate: Option<String>,
    },
    /// A password; also used to answer a keyboard-interactive password
    /// prompt when the server offers that after refusing `password`.
    Password(Secret),
    /// Any key the Windows OpenSSH agent holds, for example a FIDO2 security
    /// key (candidate: not tested with hardware yet, CRED-04).
    Agent,
}

/// Only the method name: secrets and key paths must not reach a log.
impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Auth::KeyFile { .. } => "Auth::KeyFile",
            Auth::KeyText { .. } => "Auth::KeyText",
            Auth::Password(_) => "Auth::Password",
            Auth::Agent => "Auth::Agent",
        })
    }
}

fn rejected(what: &str, user: &str) -> anyhow::Error {
    core_err(
        ErrorCode::Unauthenticated,
        format!("{what} rejected for user {user}"),
    )
}

/// Sign `user` in on a connection whose host key was already accepted.
pub(crate) async fn authenticate<H: Handler>(
    handle: &mut Handle<H>,
    user: &str,
    auth: &Auth,
) -> Result<()> {
    match auth {
        Auth::KeyFile { path, passphrase } => {
            let (path, passphrase) = (path.clone(), passphrase.clone());
            let key =
                unlock(move || load_secret_key(&path, passphrase.as_ref().map(|p| p.as_str())))
                    .await?;
            with_key(handle, user, key).await
        }
        Auth::KeyText {
            private_key,
            passphrase,
            certificate,
        } => {
            let (text, passphrase) = (private_key.clone(), passphrase.clone());
            let key =
                unlock(move || decode_secret_key(&text, passphrase.as_ref().map(|p| p.as_str())))
                    .await?;
            match certificate.as_deref().map(str::trim) {
                Some(line) if !line.is_empty() => with_certificate(handle, user, key, line).await,
                _ => with_key(handle, user, key).await,
            }
        }
        Auth::Password(password) => with_password(handle, user, password).await,
        Auth::Agent => with_agent(handle, user).await,
    }
}

/// Read and decrypt a private key off the async workers (decrypting an
/// OpenSSH key runs a deliberately slow key derivation).
async fn unlock<F>(load: F) -> Result<PrivateKey>
where
    F: FnOnce() -> Result<PrivateKey, russh::keys::Error> + Send + 'static,
{
    match tokio::task::spawn_blocking(load).await {
        Ok(Ok(key)) => Ok(key),
        Ok(Err(russh::keys::Error::KeyIsEncrypted)) => Err(core_err(
            ErrorCode::Unauthenticated,
            "private key is encrypted: save its passphrase",
        )),
        // Do not echo the key path or key material back to the UI/logs.
        Ok(Err(e)) => Err(core_err(
            ErrorCode::Unauthenticated,
            format!("cannot load private key: {e}"),
        )),
        Err(_) => Err(core_err(
            ErrorCode::InternalError,
            "reading the private key was interrupted",
        )),
    }
}

async fn with_key<H: Handler>(handle: &mut Handle<H>, user: &str, key: PrivateKey) -> Result<()> {
    let hash = handle.best_supported_rsa_hash().await?.flatten();
    let done = handle
        .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
        .await?;
    if done.success() {
        Ok(())
    } else {
        Err(rejected("public key", user))
    }
}

async fn with_certificate<H: Handler>(
    handle: &mut Handle<H>,
    user: &str,
    key: PrivateKey,
    line: &str,
) -> Result<()> {
    let certificate = Certificate::from_openssh(line).map_err(|_| {
        core_err(
            ErrorCode::Unauthenticated,
            "the certificate saved with this key cannot be read",
        )
    })?;
    if certificate.public_key() != key.public_key().key_data() {
        return Err(core_err(
            ErrorCode::Unauthenticated,
            "the certificate does not belong to this private key",
        ));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if certificate.valid_before() <= now {
        return Err(core_err(
            ErrorCode::Unauthenticated,
            "the certificate has expired: ask for a new one and save it with the key",
        ));
    }
    let done = handle
        .authenticate_openssh_cert(user, Arc::new(key), certificate)
        .await?;
    if done.success() {
        Ok(())
    } else {
        Err(rejected("certificate", user))
    }
}

async fn with_password<H: Handler>(
    handle: &mut Handle<H>,
    user: &str,
    password: &Secret,
) -> Result<()> {
    let first = handle
        .authenticate_password(user, password.as_str())
        .await?;
    let russh::client::AuthResult::Failure {
        remaining_methods, ..
    } = first
    else {
        return Ok(());
    };
    // Many servers (PAM setups) refuse `password` and only take the same
    // password through a keyboard-interactive prompt.
    if remaining_methods.contains(&MethodKind::KeyboardInteractive)
        && interactive_password(handle, user, password).await?
    {
        return Ok(());
    }
    Err(rejected("password", user))
}

/// Answer one hidden keyboard-interactive prompt with the password. Anything
/// else the server asks (a one-time code, several questions, an echoed
/// prompt) is not answered: the password is typed into a password prompt
/// only, and only once.
async fn interactive_password<H: Handler>(
    handle: &mut Handle<H>,
    user: &str,
    password: &Secret,
) -> Result<bool> {
    let mut reply = handle
        .authenticate_keyboard_interactive_start(user, None)
        .await?;
    let mut answered = false;
    for _ in 0..MAX_INTERACTIVE_ROUNDS {
        let prompts = match reply {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => prompts,
        };
        let answers = match prompts.as_slice() {
            // An empty request only carries a message; acknowledge it.
            [] => Vec::new(),
            [only] if !only.echo && !answered => {
                answered = true;
                vec![password.as_str().to_string()]
            }
            _ => return Ok(false),
        };
        reply = handle
            .authenticate_keyboard_interactive_respond(answers)
            .await?;
    }
    Ok(matches!(reply, KeyboardInteractiveAuthResponse::Success))
}

/// Offer every plain key the Windows OpenSSH agent holds until the server
/// takes one. The private key never leaves the agent (or the security key).
#[cfg(windows)]
async fn with_agent<H: Handler>(handle: &mut Handle<H>, user: &str) -> Result<()> {
    use russh::keys::agent::client::AgentClient;
    use russh::keys::agent::AgentIdentity;
    use russh::keys::Algorithm;

    let unavailable = || {
        core_err(
            ErrorCode::Unauthenticated,
            "the Windows OpenSSH agent did not answer: start the \"OpenSSH Authentication Agent\" service and add the key to it",
        )
    };
    let mut agent = AgentClient::connect_named_pipe(AGENT_PIPE)
        .await
        .map_err(|_| unavailable())?;
    let identities = agent
        .request_identities()
        .await
        .map_err(|_| unavailable())?;
    let keys: Vec<_> = identities
        .into_iter()
        .filter_map(|identity| match identity {
            AgentIdentity::PublicKey { key, .. } => Some(key),
            AgentIdentity::Certificate { .. } => None,
        })
        .collect();
    if keys.is_empty() {
        return Err(core_err(
            ErrorCode::Unauthenticated,
            "the Windows OpenSSH agent holds no key: add one with ssh-add",
        ));
    }
    let rsa_hash: Option<HashAlg> = handle.best_supported_rsa_hash().await?.flatten();
    for key in keys {
        let hash = matches!(key.algorithm(), Algorithm::Rsa { .. })
            .then_some(rsa_hash)
            .flatten();
        // A key the agent cannot sign with (a security key that was not
        // touched, say) is skipped; the next one is tried.
        if let Ok(done) = handle
            .authenticate_publickey_with(user, key, hash, &mut agent)
            .await
        {
            if done.success() {
                return Ok(());
            }
        }
        if handle.is_closed() {
            break;
        }
    }
    Err(rejected("every key in the Windows OpenSSH agent was", user))
}

#[cfg(not(windows))]
async fn with_agent<H: Handler>(_: &mut Handle<H>, _: &str) -> Result<()> {
    Err(core_err(
        ErrorCode::UnsupportedCapability,
        "signing in through the OpenSSH agent is only wired up on Windows",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_names_the_method_and_nothing_else() {
        let password = Auth::Password(Secret::new("hunter2-secret".into()));
        assert_eq!(format!("{password:?}"), "Auth::Password");
        let key = Auth::KeyFile {
            path: PathBuf::from(r"C:\Users\someone\.ssh\id_ed25519"),
            passphrase: Some(Secret::new("open-sesame".into())),
        };
        let text = format!("{key:?}");
        assert!(
            !text.contains("someone") && !text.contains("sesame"),
            "{text}"
        );
    }
}
