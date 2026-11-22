//! Route fallback against real (in-process) SSH servers (NET-IMPL-02/03):
//! an unreachable address and a server with the wrong host key are skipped,
//! and the pinned key decides which machine is accepted.

use russh::keys::{ssh_key::LineEnding, Algorithm, HashAlg, PrivateKey};
use russh::server::{self, Server as _};
use senvakara_adapters_ssh::{connect_any, Auth, Candidate, ConnectParams, HostKeyPolicy};
use senvakara_core::{CoreError, ErrorCode};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

#[derive(Clone)]
struct Fake;

impl server::Server for Fake {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        Fake
    }
}

impl server::Handler for Fake {
    type Error = russh::Error;
    async fn auth_publickey(
        &mut self,
        _: &str,
        _: server::VerifiedPublicKey<'_>,
    ) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Accept)
    }
}

/// Start an SSH server on 127.0.0.1 with `key`; returns its port.
async fn serve(key: PrivateKey) -> u16 {
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    tokio::spawn(async move {
        let _ = Fake.run_on_socket(config, &socket).await;
    });
    port
}

fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn fingerprint(k: &PrivateKey) -> String {
    k.public_key().fingerprint(HashAlg::Sha256).to_string()
}

/// A port that was free a moment ago: connections are refused.
fn closed_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn cand(transport: &'static str, port: u16) -> Candidate {
    Candidate {
        transport,
        address: "127.0.0.1".into(),
        port,
    }
}

struct Client {
    dir: std::path::PathBuf,
    params: ConnectParams,
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn client(pinned: &str) -> Client {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("svk-fallback-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("id_test");
    std::fs::write(&path, key().to_openssh(LineEnding::LF).unwrap().as_bytes()).unwrap();
    Client {
        dir,
        params: ConnectParams {
            host: String::new(),
            port: 0,
            user: "tester".into(),
            auth: Auth::KeyFile {
                path,
                passphrase: None,
            },
            host_key: HostKeyPolicy::Pinned(pinned.to_string()),
            connect_timeout: Duration::from_secs(5),
        },
    }
}

fn code(e: &anyhow::Error) -> ErrorCode {
    e.downcast_ref::<CoreError>().expect("CoreError").code
}

#[tokio::test]
async fn skips_unreachable_and_wrong_key_paths() {
    let real = key();
    let real_port = serve(real.clone()).await;
    let impostor_port = serve(key()).await;
    let c = client(&fingerprint(&real));

    // LAN down, first VPN address is a different machine, second is the host.
    let cands = vec![
        cand("lan", closed_port()),
        cand("vpn", impostor_port),
        cand("vpn", real_port),
    ];
    let done = connect_any(&cands, &c.params, Duration::from_secs(3))
        .await
        .expect("falls back to the real host");
    assert_eq!(done.index, 2);
    assert_eq!(done.conn.host_key_fingerprint(), fingerprint(&real));
    assert!(
        done.notes[0].contains("lan 127.0.0.1 unreachable"),
        "{:?}",
        done.notes
    );
    assert!(
        done.notes.iter().any(|n| n.contains("host key rejected")),
        "{:?}",
        done.notes
    );

    // The preferred path wins when it works.
    let cands = vec![cand("lan", real_port), cand("vpn", impostor_port)];
    let done = connect_any(&cands, &c.params, Duration::from_secs(3))
        .await
        .unwrap();
    assert_eq!(done.index, 0);
    assert!(done.notes.is_empty());
}

#[tokio::test]
async fn reports_key_problem_before_unreachable() {
    let real = key();
    let impostor_port = serve(key()).await;
    let c = client(&fingerprint(&real));
    let cands = vec![cand("lan", closed_port()), cand("vpn", impostor_port)];
    let err = connect_any(&cands, &c.params, Duration::from_secs(3))
        .await
        .err()
        .expect("only an impostor answers");
    assert_eq!(code(&err), ErrorCode::Unauthenticated);

    let cands = vec![cand("lan", closed_port())];
    let err = connect_any(&cands, &c.params, Duration::from_secs(3))
        .await
        .err()
        .expect("nothing answers");
    assert_eq!(code(&err), ErrorCode::HostOffline);
}

#[tokio::test]
async fn a_silent_path_does_not_hold_up_the_next_one() {
    // Accepts TCP but never speaks SSH (like a stale route after a network change).
    let silent = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let silent_port = silent.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = silent.accept().await {
            held.push(s);
        }
    });
    let real = key();
    let real_port = serve(real.clone()).await;
    let mut c = client(&fingerprint(&real));
    c.params.connect_timeout = Duration::from_secs(30);
    let t = std::time::Instant::now();
    let cands = vec![cand("lan", silent_port), cand("vpn", real_port)];
    let done = connect_any(&cands, &c.params, Duration::from_secs(3))
        .await
        .unwrap();
    assert_eq!(done.index, 1);
    assert!(t.elapsed() < Duration::from_secs(9), "{:?}", t.elapsed());
    assert!(done.notes[0].contains("timed out"), "{:?}", done.notes);
}
