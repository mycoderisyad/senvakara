//! SSH adapter: verified connection, interactive PTY, structured exec, SFTP
//! and loopback-only local port forwarding (used for remote ADB, ADR-014).
//!
//! Requirements TERM-02, FILE-01, AND-02 / tasks POC-02, POC-04, POC-05.
//!
//! Security rules applied here (docs/SECURITY.md §3):
//! - The server host key is always checked. Unknown keys are rejected unless
//!   the caller explicitly opts in to trust-on-first-use for this connection
//!   (the UI must show the fingerprint and get user approval first).
//! - Public-key auth only in this adapter; no password handling.
//! - Forwarded ports bind to 127.0.0.1 only.

pub mod pty;
pub mod sftp;

pub use pty::{SshPtyEvent, SshPtyReader, SshPtyWriter};

use anyhow::{Context, Result};
use russh::client::{self, Handle};
use russh::keys::{load_secret_key, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect};
use russh_sftp::client::SftpSession;
use senvakara_core::{CoreError, ErrorCode};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// How the server host key must be treated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyPolicy {
    /// Only accept a key whose SHA-256 fingerprint (`SHA256:...`) matches.
    Pinned(String),
    /// Accept an unknown key once. The caller must persist the observed
    /// fingerprint and only use this after explicit user approval.
    TrustOnFirstUse,
}

#[derive(Debug, Clone)]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub private_key: PathBuf,
    pub host_key: HostKeyPolicy,
    pub connect_timeout: Duration,
    /// Passphrase for an encrypted private key (from the OS credential store).
    pub passphrase: Option<String>,
}

/// Errors from this adapter carry a [`CoreError`] so callers can
/// `downcast_ref::<CoreError>()` and show a structured state.
fn core_err(code: ErrorCode, message: impl Into<String>) -> anyhow::Error {
    CoreError::new(code, message).into()
}

struct VerifyingHandler {
    policy: HostKeyPolicy,
    observed: Arc<Mutex<Option<String>>>,
}

impl client::Handler for VerifyingHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // Certificates need a CA trust model we have not designed yet.
        let PublicKeyOrCertificate::PublicKey { key, .. } = server_public_key else {
            return Ok(false);
        };
        let fp = key.fingerprint(HashAlg::Sha256).to_string();
        *self.observed.lock().unwrap() = Some(fp.clone());
        Ok(match &self.policy {
            HostKeyPolicy::Pinned(expected) => expected == &fp,
            HostKeyPolicy::TrustOnFirstUse => true,
        })
    }
}

/// An authenticated SSH connection. Cheap to share: clone the `Arc`.
pub struct SshConnection {
    handle: Handle<VerifyingHandler>,
    host_key_fingerprint: String,
}

impl SshConnection {
    pub async fn connect(p: &ConnectParams) -> Result<Arc<Self>> {
        let observed = Arc::new(Mutex::new(None));
        let handler = VerifyingHandler {
            policy: p.host_key.clone(),
            observed: observed.clone(),
        };
        let config = Arc::new(client::Config {
            inactivity_timeout: Some(Duration::from_secs(300)),
            keepalive_interval: Some(Duration::from_secs(15)),
            ..Default::default()
        });

        let connecting = client::connect(config, (p.host.as_str(), p.port), handler);
        let mut handle = match tokio::time::timeout(p.connect_timeout, connecting).await {
            Err(_) => {
                return Err(core_err(
                    ErrorCode::HostOffline,
                    format!("connection to {}:{} timed out", p.host, p.port),
                ))
            }
            Ok(Err(e)) => {
                if let Some(seen) = observed.lock().unwrap().clone() {
                    return Err(core_err(
                        ErrorCode::Unauthenticated,
                        format!("host key rejected: server presented {seen}, which does not match the trusted key"),
                    ));
                }
                return Err(core_err(
                    ErrorCode::HostOffline,
                    format!("cannot reach {}:{}: {e}", p.host, p.port),
                ));
            }
            Ok(Ok(h)) => h,
        };
        let host_key_fingerprint = observed
            .lock()
            .unwrap()
            .clone()
            .context("server presented no host key")?;

        let key = match load_secret_key(&p.private_key, p.passphrase.as_deref()) {
            Ok(k) => k,
            Err(russh::keys::Error::KeyIsEncrypted) => {
                return Err(core_err(
                    ErrorCode::Unauthenticated,
                    "private key is encrypted: save its passphrase for this host",
                ))
            }
            // Do not echo the key path or key material back to the UI/logs.
            Err(e) => {
                return Err(core_err(
                    ErrorCode::Unauthenticated,
                    format!("cannot load private key: {e}"),
                ))
            }
        };
        let hash = handle.best_supported_rsa_hash().await?.flatten();
        let auth = handle
            .authenticate_publickey(
                p.user.clone(),
                PrivateKeyWithHashAlg::new(Arc::new(key), hash),
            )
            .await?;
        if !auth.success() {
            return Err(core_err(
                ErrorCode::Unauthenticated,
                format!("public key rejected for user {}", p.user),
            ));
        }
        Ok(Arc::new(Self {
            handle,
            host_key_fingerprint,
        }))
    }

    /// Connect only far enough to read the server's host-key fingerprint, then
    /// abort before authenticating. Used to show the key for user approval.
    pub async fn fetch_host_key(host: &str, port: u16, timeout: Duration) -> Result<String> {
        struct Recorder(Arc<Mutex<Option<String>>>);
        impl client::Handler for Recorder {
            type Error = russh::Error;
            async fn check_server_key(
                &mut self,
                k: &PublicKeyOrCertificate,
            ) -> Result<bool, Self::Error> {
                if let PublicKeyOrCertificate::PublicKey { key, .. } = k {
                    *self.0.lock().unwrap() = Some(key.fingerprint(HashAlg::Sha256).to_string());
                }
                Ok(false)
            }
        }
        let seen = Arc::new(Mutex::new(None));
        let connecting = client::connect(
            Arc::new(client::Config::default()),
            (host, port),
            Recorder(seen.clone()),
        );
        match tokio::time::timeout(timeout, connecting).await {
            Err(_) => Err(core_err(
                ErrorCode::HostOffline,
                format!("connection to {host}:{port} timed out"),
            )),
            Ok(result) => {
                let fp = seen.lock().unwrap().clone();
                match (fp, result) {
                    (Some(fp), _) => Ok(fp),
                    (None, Err(e)) => Err(core_err(
                        ErrorCode::HostOffline,
                        format!("cannot reach {host}:{port}: {e}"),
                    )),
                    (None, Ok(_)) => Err(core_err(
                        ErrorCode::InternalError,
                        "server presented no host key",
                    )),
                }
            }
        }
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Fingerprint the server actually presented (persist it after TOFU).
    pub fn host_key_fingerprint(&self) -> &str {
        &self.host_key_fingerprint
    }

    /// Run a program with discrete arguments; each argument is POSIX-quoted
    /// so no shell interpolation of caller data happens on the remote side.
    pub async fn exec(&self, argv: &[&str]) -> Result<ExecOutput> {
        let command = argv
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ");
        let mut ch = self.handle.channel_open_session().await?;
        ch.exec(true, command).await?;
        let mut out = ExecOutput::default();
        while let Some(msg) = ch.wait().await {
            match msg {
                ChannelMsg::Data { data } => out.stdout.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, .. } => out.stderr.extend_from_slice(&data),
                ChannelMsg::ExitStatus { exit_status } => out.exit_status = Some(exit_status),
                _ => {}
            }
        }
        Ok(out)
    }

    /// Open an interactive login shell on a remote PTY.
    pub async fn open_pty_shell(
        &self,
        cols: u32,
        rows: u32,
    ) -> Result<russh::Channel<client::Msg>> {
        let ch = self.handle.channel_open_session().await?;
        ch.request_pty(true, "xterm-256color", cols, rows, 0, 0, &[])
            .await?;
        ch.request_shell(true).await?;
        Ok(ch)
    }

    /// Open an interactive login shell, split for concurrent read/write.
    pub async fn open_pty(&self, cols: u32, rows: u32) -> Result<(SshPtyWriter, SshPtyReader)> {
        Ok(pty::split(self.open_pty_shell(cols, rows).await?))
    }

    /// Start a long-running program (discrete, POSIX-quoted arguments) and
    /// keep its channel. Closing the writer ends the channel.
    pub async fn spawn(&self, argv: &[&str]) -> Result<(SshPtyWriter, SshPtyReader)> {
        let command = argv
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ");
        let ch = self.handle.channel_open_session().await?;
        ch.exec(true, command).await?;
        Ok(pty::split(ch))
    }

    /// Byte stream to a Unix socket on the host (`direct-streamlocal`), so a
    /// service bound to a user-only socket is reachable without a TCP port.
    pub async fn open_unix_socket(&self, path: &str) -> Result<russh::ChannelStream<client::Msg>> {
        let ch = self.handle.channel_open_direct_streamlocal(path).await?;
        Ok(ch.into_stream())
    }

    /// Byte stream to `host:port` as seen from the SSH server (`direct-tcpip`),
    /// e.g. a service bound to the host's loopback interface only.
    pub async fn open_tcp(
        &self,
        host: &str,
        port: u16,
    ) -> Result<russh::ChannelStream<client::Msg>> {
        let ch = self
            .handle
            .channel_open_direct_tcpip(host, port.into(), "127.0.0.1", 0)
            .await?;
        Ok(ch.into_stream())
    }

    pub async fn sftp(&self) -> Result<SftpSession> {
        let ch = self.handle.channel_open_session().await?;
        ch.request_subsystem(true, "sftp").await?;
        SftpSession::new(ch.into_stream())
            .await
            .context("start sftp")
    }

    /// Forward `127.0.0.1:<random>` on this machine to `remote_host:remote_port`
    /// as seen from the SSH server. Abort the returned task to stop forwarding.
    pub async fn forward_local(
        self: &Arc<Self>,
        remote_host: &str,
        remote_port: u16,
    ) -> Result<(SocketAddr, JoinHandle<()>)> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let local = listener.local_addr()?;
        let conn = Arc::clone(self);
        let remote_host = remote_host.to_string();
        let task = tokio::spawn(async move {
            while let Ok((mut tcp, peer)) = listener.accept().await {
                let conn = Arc::clone(&conn);
                let remote_host = remote_host.clone();
                tokio::spawn(async move {
                    let ch = match conn
                        .handle
                        .channel_open_direct_tcpip(
                            remote_host,
                            remote_port.into(),
                            peer.ip().to_string(),
                            peer.port().into(),
                        )
                        .await
                    {
                        Ok(ch) => ch,
                        Err(_) => return,
                    };
                    let mut stream = ch.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut tcp, &mut stream).await;
                });
            }
        });
        Ok((local, task))
    }

    pub async fn close(&self) -> Result<()> {
        self.handle
            .disconnect(Disconnect::ByApplication, "", "en")
            .await?;
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct ExecOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_status: Option<u32>,
}

impl ExecOutput {
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
    pub fn success(&self) -> bool {
        self.exit_status == Some(0)
    }
}

/// POSIX single-quote an argument for transmission in an SSH exec request.
pub fn shell_quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./=:@,+%".contains(&b))
    {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::shell_quote;

    #[test]
    fn plain_args_are_untouched() {
        assert_eq!(shell_quote("adb"), "adb");
        assert_eq!(shell_quote("ro.product.model"), "ro.product.model");
    }

    #[test]
    fn metacharacters_are_neutralised() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("$(reboot)"), "'$(reboot)'");
        assert_eq!(shell_quote("x; rm -rf ~"), "'x; rm -rf ~'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
    }
}
