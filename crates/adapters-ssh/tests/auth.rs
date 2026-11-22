//! Sign-in methods against an in-process SSH server (CRED-01..03): password,
//! keyboard-interactive fallback, key text, certificate, and the rule that
//! nothing is offered to a server whose host key is not the pinned one.

use russh::keys::ssh_key::certificate::{Builder, CertType};
use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, Certificate, HashAlg, PrivateKey};
use russh::server::{self, Server as _};
use russh::{MethodKind, MethodSet};
use senvakara_adapters_ssh::{Auth, ConnectParams, HostKeyPolicy, Secret, SshConnection};
use senvakara_core::{CoreError, ErrorCode};
use std::borrow::Cow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;

const USER: &str = "tester";
const PASSWORD: &str = "correct horse battery";

/// What the fake server accepts.
#[derive(Clone, Default)]
struct Rules {
    /// Accept this password through the `password` method.
    password: Option<&'static str>,
    /// Accept this password through one hidden keyboard-interactive prompt.
    interactive: Option<&'static str>,
    /// Accept this public key.
    key: Option<russh::keys::PublicKey>,
    /// Accept certificates signed by this authority.
    authority: Option<russh::keys::PublicKey>,
}

#[derive(Clone)]
struct Fake {
    rules: Rules,
    /// Every sign-in attempt of any kind the server saw.
    attempts: Arc<AtomicUsize>,
}

impl Fake {
    fn seen(&self) {
        self.attempts.fetch_add(1, Ordering::SeqCst);
    }

    fn verdict(&self, accepted: bool) -> server::Auth {
        if accepted {
            return server::Auth::Accept;
        }
        let mut methods = MethodSet::empty();
        if self.rules.interactive.is_some() {
            methods.push(MethodKind::KeyboardInteractive);
        }
        server::Auth::Reject {
            proceed_with_methods: Some(methods),
            partial_success: false,
        }
    }
}

impl server::Server for Fake {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for Fake {
    type Error = russh::Error;

    async fn auth_none(&mut self, _: &str) -> Result<server::Auth, Self::Error> {
        self.seen();
        Ok(self.verdict(false))
    }

    async fn auth_password(
        &mut self,
        _: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        self.seen();
        Ok(self.verdict(self.rules.password == Some(password)))
    }

    async fn auth_publickey_offered(
        &mut self,
        _: &str,
        _: server::PublicKeyOffer<'_>,
    ) -> Result<server::Auth, Self::Error> {
        self.seen();
        Ok(server::Auth::Accept)
    }

    async fn auth_publickey(
        &mut self,
        _: &str,
        key: server::VerifiedPublicKey<'_>,
    ) -> Result<server::Auth, Self::Error> {
        self.seen();
        let known = self.rules.key.as_ref().map(|k| k.key_data());
        Ok(self.verdict(known == Some(key.public_key.key_data())))
    }

    async fn auth_openssh_certificate(
        &mut self,
        _: &str,
        certificate: &Certificate,
    ) -> Result<server::Auth, Self::Error> {
        self.seen();
        let authority = self.rules.authority.as_ref().map(|k| k.key_data());
        Ok(self.verdict(authority == Some(certificate.signature_key())))
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _: &str,
        _: &str,
        response: Option<server::Response<'a>>,
    ) -> Result<server::Auth, Self::Error> {
        self.seen();
        let Some(mut answers) = response else {
            return Ok(server::Auth::Partial {
                name: Cow::Borrowed(""),
                instructions: Cow::Borrowed(""),
                prompts: Cow::Borrowed(&[(Cow::Borrowed("Password: "), false)]),
            });
        };
        let answer = answers.next().unwrap_or_default();
        let expected = self.rules.interactive.map(str::as_bytes);
        Ok(self.verdict(expected == Some(&answer[..])))
    }
}

struct Server {
    port: u16,
    fingerprint: String,
    attempts: Arc<AtomicUsize>,
}

async fn serve(rules: Rules) -> Server {
    let host_key = key();
    let fingerprint = host_key
        .public_key()
        .fingerprint(HashAlg::Sha256)
        .to_string();
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    let config = Arc::new(server::Config {
        keys: vec![host_key],
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let attempts = Arc::new(AtomicUsize::new(0));
    let mut fake = Fake {
        rules,
        attempts: attempts.clone(),
    };
    tokio::spawn(async move {
        let _ = fake.run_on_socket(config, &socket).await;
    });
    Server {
        port,
        fingerprint,
        attempts,
    }
}

fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn text(key: &PrivateKey) -> Secret {
    Secret::new(key.to_openssh(LineEnding::LF).unwrap().to_string())
}

fn password(value: &str) -> Auth {
    Auth::Password(Secret::new(value.to_string()))
}

async fn connect(server: &Server, auth: Auth) -> anyhow::Result<Arc<SshConnection>> {
    SshConnection::connect(&ConnectParams {
        host: "127.0.0.1".into(),
        port: server.port,
        user: USER.into(),
        auth,
        host_key: HostKeyPolicy::Pinned(server.fingerprint.clone()),
        connect_timeout: Duration::from_secs(5),
    })
    .await
}

fn refusal(result: anyhow::Result<Arc<SshConnection>>) -> CoreError {
    let error = result.err().expect("the sign-in must fail");
    error
        .downcast_ref::<CoreError>()
        .expect("CoreError")
        .clone()
}

#[tokio::test]
async fn the_right_password_signs_in() {
    let server = serve(Rules {
        password: Some(PASSWORD),
        ..Default::default()
    })
    .await;
    connect(&server, password(PASSWORD)).await.unwrap();
}

#[tokio::test]
async fn a_wrong_password_is_unauthenticated_and_not_echoed() {
    let server = serve(Rules {
        password: Some(PASSWORD),
        ..Default::default()
    })
    .await;
    let error = refusal(connect(&server, password("not-the-password")).await);
    assert_eq!(error.code, ErrorCode::Unauthenticated);
    assert!(!error.message.contains("not-the-password"), "{error}");
    // One try, no loop: the server offered nothing else to fall back to.
    assert_eq!(server.attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_server_that_only_prompts_gets_the_password_typed_into_the_prompt() {
    let server = serve(Rules {
        interactive: Some(PASSWORD),
        ..Default::default()
    })
    .await;
    connect(&server, password(PASSWORD)).await.unwrap();

    let error = refusal(connect(&server, password("not-the-password")).await);
    assert_eq!(error.code, ErrorCode::Unauthenticated);
}

#[tokio::test]
async fn a_key_given_as_text_signs_in() {
    let user_key = key();
    let server = serve(Rules {
        key: Some(user_key.public_key().clone()),
        ..Default::default()
    })
    .await;
    let auth = Auth::KeyText {
        private_key: text(&user_key),
        passphrase: None,
        certificate: None,
    };
    connect(&server, auth).await.unwrap();

    let stranger = Auth::KeyText {
        private_key: text(&key()),
        passphrase: None,
        certificate: None,
    };
    assert_eq!(
        refusal(connect(&server, stranger).await).code,
        ErrorCode::Unauthenticated
    );
}

#[tokio::test]
async fn an_encrypted_key_needs_its_passphrase() {
    let user_key = key();
    let server = serve(Rules {
        key: Some(user_key.public_key().clone()),
        ..Default::default()
    })
    .await;
    let locked = text(&user_key.encrypt(&mut rand::rng(), "open sesame").unwrap());
    let auth = |passphrase: Option<&str>| Auth::KeyText {
        private_key: locked.clone(),
        passphrase: passphrase.map(|p| Secret::new(p.to_string())),
        certificate: None,
    };

    let error = refusal(connect(&server, auth(None)).await);
    assert_eq!(error.code, ErrorCode::Unauthenticated);
    assert!(error.message.contains("save its passphrase"), "{error}");
    // The key never unlocked, so nothing was offered to the server.
    assert_eq!(server.attempts.load(Ordering::SeqCst), 0);

    connect(&server, auth(Some("open sesame"))).await.unwrap();
}

/// A user certificate for `subject`, signed by `authority`, valid from
/// `from` to `until` (Unix seconds).
fn certificate(subject: &PrivateKey, authority: &PrivateKey, from: u64, until: u64) -> String {
    let mut builder = Builder::new(
        [7u8; 16].to_vec(),
        subject.public_key().key_data().clone(),
        from,
        until,
    )
    .unwrap();
    builder.cert_type(CertType::User).unwrap();
    builder.valid_principal(USER).unwrap();
    builder.sign(authority).unwrap().to_openssh().unwrap()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::test]
async fn a_certificate_from_a_trusted_authority_signs_in() {
    let (user_key, authority) = (key(), key());
    let server = serve(Rules {
        authority: Some(authority.public_key().clone()),
        ..Default::default()
    })
    .await;
    let auth = |certificate: String| Auth::KeyText {
        private_key: text(&user_key),
        passphrase: None,
        certificate: Some(certificate),
    };
    let valid = certificate(&user_key, &authority, now() - 60, now() + 3600);
    connect(&server, auth(valid)).await.unwrap();

    // Signed by an authority the server does not trust.
    let foreign = certificate(&user_key, &key(), now() - 60, now() + 3600);
    assert_eq!(
        refusal(connect(&server, auth(foreign)).await).code,
        ErrorCode::Unauthenticated
    );
}

#[tokio::test]
async fn an_expired_or_mismatched_certificate_is_refused_before_it_is_sent() {
    let (user_key, authority) = (key(), key());
    let server = serve(Rules {
        authority: Some(authority.public_key().clone()),
        ..Default::default()
    })
    .await;
    let auth = |certificate: String| Auth::KeyText {
        private_key: text(&user_key),
        passphrase: None,
        certificate: Some(certificate),
    };

    let expired = certificate(&user_key, &authority, now() - 7200, now() - 3600);
    let error = refusal(connect(&server, auth(expired)).await);
    assert_eq!(error.code, ErrorCode::Unauthenticated);
    assert!(error.message.contains("expired"), "{error}");

    let other = certificate(&key(), &authority, now() - 60, now() + 3600);
    let error = refusal(connect(&server, auth(other)).await);
    assert!(error.message.contains("does not belong"), "{error}");

    let error = refusal(connect(&server, auth("not a certificate".into())).await);
    assert_eq!(error.code, ErrorCode::Unauthenticated);
    assert_eq!(server.attempts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_server_with_the_wrong_host_key_never_sees_a_credential() {
    let server = serve(Rules {
        password: Some(PASSWORD),
        ..Default::default()
    })
    .await;
    let other = key().public_key().fingerprint(HashAlg::Sha256).to_string();
    for auth in [
        password(PASSWORD),
        Auth::KeyText {
            private_key: text(&key()),
            passphrase: None,
            certificate: None,
        },
    ] {
        let result = SshConnection::connect(&ConnectParams {
            host: "127.0.0.1".into(),
            port: server.port,
            user: USER.into(),
            auth,
            host_key: HostKeyPolicy::Pinned(other.clone()),
            connect_timeout: Duration::from_secs(5),
        })
        .await;
        let error = refusal(result);
        assert_eq!(error.code, ErrorCode::Unauthenticated);
        assert!(error.message.contains("host key rejected"), "{error}");
    }
    assert_eq!(server.attempts.load(Ordering::SeqCst), 0);
}
