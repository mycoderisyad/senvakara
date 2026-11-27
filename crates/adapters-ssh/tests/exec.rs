//! `exec_stream` against an in-process SSH server (WF-SPIKE-02, ADR-030):
//! stdin then EOF, stdout and stderr apart, exit status, deadline, cancel,
//! and a channel that disappears without a status.

use russh::keys::{Algorithm, HashAlg, PrivateKey};
use russh::server::{self, Server as _};
use russh::{Channel, ChannelId};
use senvakara_adapters_ssh::{
    Auth, ConnectParams, ExecEnd, ExecEvent, ExecLimits, HostKeyPolicy, Secret, SshConnection,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::watch;

#[derive(Clone, Default)]
struct Fake {
    /// Command and stdin bytes per channel.
    channels: HashMap<ChannelId, (String, usize)>,
    /// Never answer a channel open: a stalled connection.
    stall_open: bool,
}

impl server::Server for Fake {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for Fake {
    type Error = russh::Error;

    async fn auth_password(&mut self, _: &str, _: &str) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        _: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if self.stall_open {
            // Hold the open unanswered (dropping `reply` would refuse it).
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).to_string();
        // Never confirms the exec request: a stalled connection.
        if command == "no-reply" {
            return Ok(());
        }
        session.channel_success(channel)?;
        match command.as_str() {
            "out-err" => {
                session.data(channel, b"out-1\n".to_vec())?;
                session.extended_data(channel, 1, b"err-1\n".to_vec())?;
                session.data(channel, b"out-2\n".to_vec())?;
            }
            // Gone without an exit status.
            "vanish" => session.close(channel)?,
            // "hang": never answers.
            _ => {}
        }
        self.channels.insert(channel, (command, 0));
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if let Some(c) = self.channels.get_mut(&channel) {
            c.1 += data.len();
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if let Some((command, stdin)) = self.channels.get(&channel) {
            if command == "out-err" {
                session.data(channel, format!("stdin={stdin}\n").into_bytes())?;
                session.exit_status_request(channel, 7)?;
                session.eof(channel)?;
                session.close(channel)?;
            }
        }
        Ok(())
    }
}

async fn connect() -> Arc<SshConnection> {
    serve(false).await
}

async fn serve(stall_open: bool) -> Arc<SshConnection> {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    let config = Arc::new(server::Config {
        keys: vec![key],
        ..Default::default()
    });
    tokio::spawn(async move {
        let mut fake = Fake {
            stall_open,
            ..Fake::default()
        };
        let _ = fake.run_on_socket(config, &socket).await;
    });
    SshConnection::connect(&ConnectParams {
        host: "127.0.0.1".into(),
        port,
        user: "tester".into(),
        auth: Auth::Password(Secret::new("pw".to_string())),
        host_key: HostKeyPolicy::Pinned(fingerprint),
        connect_timeout: Duration::from_secs(5),
    })
    .await
    .unwrap()
}

fn limits(timeout_ms: u64) -> ExecLimits {
    ExecLimits {
        timeout: Duration::from_millis(timeout_ms),
        drain: Duration::from_millis(300),
    }
}

#[tokio::test]
async fn streams_stay_apart_and_stdin_ends_with_eof() {
    let conn = connect().await;
    let (_tx, rx) = watch::channel(false);
    let mut events = Vec::new();
    let stops = AtomicUsize::new(0);
    let report = conn
        .exec_stream(
            &["out-err"],
            Some(b"secret\n"),
            limits(5_000),
            rx,
            |e| events.push(e),
            || async {
                stops.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await
        .unwrap();
    assert_eq!(report.end, ExecEnd::Exited(7));
    assert_eq!(stops.load(Ordering::SeqCst), 0);
    assert_eq!(
        events,
        [
            ExecEvent::Stdout(b"out-1\n".to_vec()),
            ExecEvent::Stderr(b"err-1\n".to_vec()),
            ExecEvent::Stdout(b"out-2\n".to_vec()),
            // The server only answers after EOF, with what it read.
            ExecEvent::Stdout(b"stdin=7\n".to_vec()),
        ]
    );
    assert_eq!((report.stdout_total, report.stderr_total), (20, 6));
}

#[tokio::test]
async fn the_deadline_stops_once_and_gives_up_after_the_drain() {
    let conn = connect().await;
    let (_tx, rx) = watch::channel(false);
    let stops = AtomicUsize::new(0);
    let started = Instant::now();
    let report = conn
        .exec_stream(
            &["hang"],
            None,
            limits(200),
            rx,
            |_| {},
            || async {
                stops.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await
        .unwrap();
    assert_eq!(report.end, ExecEnd::TimedOut);
    assert_eq!(stops.load(Ordering::SeqCst), 1);
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(450) && took < Duration::from_secs(3),
        "{took:?}"
    );
}

#[tokio::test]
async fn cancel_stops_before_the_deadline() {
    let conn = connect().await;
    let (tx, rx) = watch::channel(false);
    let stops = AtomicUsize::new(0);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        tx.send(true).ok();
        // Keep the sender alive past the stop.
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let started = Instant::now();
    let report = conn
        .exec_stream(
            &["hang"],
            None,
            limits(30_000),
            rx,
            |_| {},
            || async {
                stops.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await
        .unwrap();
    assert_eq!(report.end, ExecEnd::Cancelled);
    assert_eq!(stops.load(Ordering::SeqCst), 1);
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn a_dropped_cancel_switch_neither_cancels_nor_spins() {
    let conn = connect().await;
    let (tx, rx) = watch::channel(false);
    drop(tx);
    let report = conn
        .exec_stream(&["out-err"], None, limits(5_000), rx, |_| {}, || async {})
        .await
        .unwrap();
    assert_eq!(report.end, ExecEnd::Exited(7));
}

#[tokio::test]
async fn a_command_the_host_never_confirms_still_ends_at_the_deadline() {
    let conn = connect().await;
    let (_tx, rx) = watch::channel(false);
    let started = Instant::now();
    let report = conn
        .exec_stream(&["no-reply"], None, limits(300), rx, |_| {}, || async {})
        .await
        .unwrap();
    assert_eq!(report.end, ExecEnd::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn a_stalled_connection_fails_with_a_timeout_instead_of_hanging() {
    let conn = serve(true).await;
    let (_tx, rx) = watch::channel(false);
    let started = Instant::now();
    let error = conn
        .exec_stream(&["out-err"], None, limits(300), rx, |_| {}, || async {})
        .await
        .unwrap_err();
    let code = error
        .downcast_ref::<senvakara_core::CoreError>()
        .map(|e| e.code);
    assert_eq!(code, Some(senvakara_core::ErrorCode::Timeout));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn a_channel_closed_without_a_status_is_lost() {
    let conn = connect().await;
    let (_tx, rx) = watch::channel(false);
    let report = conn
        .exec_stream(&["vanish"], None, limits(5_000), rx, |_| {}, || async {})
        .await
        .unwrap();
    assert_eq!(report.end, ExecEnd::Lost);
}
