//! M0 spike against a Linux host: POC-02 (SSH + host-key pinning + PTY),
//! POC-04 (SFTP round-trip + cancellation) and POC-05 (Android on Windows
//! USB and on Linux USB listed together, commands isolated, remote ADB server
//! reached only through an SSH tunnel).
//!
//! Run:
//!   cargo run -p senvakara-adapters-ssh --example poc_linux -- \
//!       <host> <user> <SHA256:pinned-fingerprint> [private-key-path]
//!
//! Env: SVK_ADB = path to adb.exe (default: %LOCALAPPDATA%\Android\Sdk\platform-tools\adb.exe)

use anyhow::{Context, Result};
use senvakara_adapters_ssh::{Auth, ConnectParams, HostKeyPolicy, SshConnection};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Report(Vec<(&'static str, bool, String)>);

impl Report {
    fn add(&mut self, name: &'static str, ok: bool, detail: impl Into<String>) {
        let detail = detail.into();
        println!("[{}] {name}: {detail}", if ok { "PASS" } else { "FAIL" });
        self.0.push((name, ok, detail));
    }
}

fn adb_path() -> String {
    std::env::var("SVK_ADB").unwrap_or_else(|_| {
        let p = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default())
            .join(r"Android\Sdk\platform-tools\adb.exe");
        if p.exists() {
            p.display().to_string()
        } else {
            "adb".into()
        }
    })
}

/// Run local adb.exe with explicit server selection. `server` = None means the
/// local Windows ADB server; Some(port) means the tunnelled Linux ADB server.
fn adb(server: Option<u16>, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new(adb_path());
    if let Some(port) = server {
        cmd.args(["-H", "127.0.0.1", "-P", &port.to_string()]);
    }
    let out = cmd.args(args).output().context("run adb")?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr))
}

/// Parse `adb devices -l` into (serial, state) pairs.
fn parse_devices(out: &str) -> Vec<(String, String)> {
    out.lines()
        .skip_while(|l| !l.starts_with("List of devices"))
        .skip(1)
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.to_string(), it.next()?.to_string()))
        })
        .collect()
}

fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed;
    (0..len)
        .map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 33) as u8
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [host, user, fp] = [&args[0], &args[1], &args[2]];
    let key = args.get(3).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
            .join(r".ssh\senvakara_test_ed25519")
    });
    let mut r = Report(Vec::new());
    let params = |policy| ConnectParams {
        host: host.clone(),
        port: 22,
        user: user.clone(),
        auth: Auth::KeyFile {
            path: key.clone(),
            passphrase: None,
        },
        host_key: policy,
        connect_timeout: Duration::from_secs(8),
    };

    // ---------- POC-02: host key pinning ----------
    let wrong = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string();
    match SshConnection::connect(&params(HostKeyPolicy::Pinned(wrong))).await {
        Ok(_) => r.add(
            "POC-02 wrong host key rejected",
            false,
            "connection was accepted!",
        ),
        Err(e) => r.add(
            "POC-02 wrong host key rejected",
            format!("{e:#}").contains("host key rejected"),
            format!("{e:#}"),
        ),
    }

    let conn: Arc<SshConnection> =
        match SshConnection::connect(&params(HostKeyPolicy::Pinned(fp.clone()))).await {
            Ok(c) => c,
            Err(e) => {
                r.add("POC-02 connect with pinned key", false, format!("{e:#}"));
                return finish(r);
            }
        };
    r.add(
        "POC-02 connect with pinned key",
        true,
        conn.host_key_fingerprint().to_string(),
    );

    let os = conn.exec(&["cat", "/etc/os-release"]).await?.stdout_str();
    let pretty = os
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .unwrap_or("?")
        .trim_matches('"')
        .to_string();
    let uname = conn.exec(&["uname", "-srm"]).await?.stdout_str();
    r.add(
        "POC-02 structured exec",
        !uname.is_empty(),
        format!("{pretty} | {}", uname.trim()),
    );

    // Interactive PTY with resize.
    {
        let mut ch = conn.open_pty_shell(120, 30).await?;
        ch.window_change(100, 40, 0, 0).await?;
        ch.data(&b"echo SVK_$((40+2))_SSH_OK; stty size; exit\n"[..])
            .await?;
        let mut buf = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while let Ok(Some(msg)) = tokio::time::timeout_at(deadline, ch.wait()).await {
            if let russh::ChannelMsg::Data { data } = msg {
                buf.push_str(&String::from_utf8_lossy(&data));
            }
        }
        let ok = buf.contains("SVK_42_SSH_OK") && buf.contains("40 100");
        r.add(
            "POC-02 interactive PTY + resize",
            ok,
            if ok {
                "marker + stty 40 100 seen".into()
            } else {
                buf.chars().take(300).collect::<String>()
            },
        );
    }

    // ---------- POC-04: SFTP ----------
    match sftp_checks(&conn).await {
        Ok(lines) => {
            for (n, ok, d) in lines {
                r.add(n, ok, d)
            }
        }
        Err(e) => r.add("POC-04 SFTP", false, format!("{e:#}")),
    }

    // ---------- POC-05: multi-host ADB ----------
    let remote_adb = conn.exec(&["sh", "-c", "command -v adb"]).await?;
    if !remote_adb.success() {
        r.add(
            "POC-05 adb on Linux host",
            false,
            "adb not found on Linux host PATH",
        );
        return finish(r);
    }
    r.add(
        "POC-05 adb on Linux host",
        true,
        remote_adb.stdout_str().trim().to_string(),
    );
    conn.exec(&["adb", "start-server"]).await?;

    let ss = conn
        .exec(&["sh", "-c", "ss -ltn 2>/dev/null || netstat -ltn"])
        .await?
        .stdout_str();
    let listeners: Vec<&str> = ss.lines().filter(|l| l.contains(":5037")).collect();
    let loopback_only = !listeners.is_empty()
        && listeners
            .iter()
            .all(|l| l.contains("127.0.0.1:5037") || l.contains("[::1]:5037"));
    r.add(
        "POC-05 Linux ADB server bound to loopback only",
        loopback_only,
        listeners.join(" | "),
    );

    let local = parse_devices(&adb(None, &["devices", "-l"])?);
    r.add(
        "POC-05 Windows USB devices (Android A)",
        !local.is_empty(),
        format!("{local:?}"),
    );

    let (fwd, task) = conn.forward_local("127.0.0.1", 5037).await?;
    let remote = parse_devices(&adb(Some(fwd.port()), &["devices", "-l"])?);
    r.add(
        "POC-05 Linux USB devices via SSH tunnel (Android B)",
        !remote.is_empty(),
        format!("tunnel {fwd} -> {remote:?}"),
    );

    for (serial, state) in &remote {
        if state != "device" {
            r.add(
                "POC-05 routed command to B",
                false,
                format!("{serial} state={state}"),
            );
            continue;
        }
        let model = adb(
            Some(fwd.port()),
            &["-s", serial, "shell", "getprop", "ro.product.model"],
        )?;
        r.add(
            "POC-05 routed command to B",
            !model.trim().is_empty(),
            format!("{serial}: {}", model.trim()),
        );
        let leaked = local.iter().any(|(s, _)| s == serial);
        r.add(
            "POC-05 B not visible on Windows server",
            !leaked,
            format!("{serial} present locally: {leaked}"),
        );
    }
    for (serial, state) in &local {
        if state == "device" {
            let model = adb(
                None,
                &["-s", serial, "shell", "getprop", "ro.product.model"],
            )?;
            r.add(
                "POC-05 routed command to A",
                !model.trim().is_empty(),
                format!("{serial}: {}", model.trim()),
            );
        }
    }
    task.abort();
    conn.close().await?;
    finish(r)
}

async fn sftp_checks(conn: &SshConnection) -> Result<Vec<(&'static str, bool, String)>> {
    let mut out = Vec::new();
    let sftp = conn.sftp().await?;
    let dir = format!("/tmp/senvakara-poc-{}", std::process::id());
    sftp.create_dir(&dir).await?;
    let path = format!("{dir}/payload.bin");

    let data = pseudo_random(4 * 1024 * 1024, 42);
    let t = std::time::Instant::now();
    let mut f = sftp.create(&path).await?;
    f.write_all(&data).await?;
    f.shutdown().await?;
    let up = t.elapsed();
    let t = std::time::Instant::now();
    let mut back = Vec::new();
    sftp.open(&path).await?.read_to_end(&mut back).await?;
    let down = t.elapsed();
    out.push((
        "POC-04 SFTP upload/download content match",
        back == data,
        format!("4 MiB up {:.2?}, down {:.2?}", up, down),
    ));

    let listed: Vec<String> = sftp.read_dir(&dir).await?.map(|e| e.file_name()).collect();
    out.push((
        "POC-04 SFTP directory listing",
        listed.contains(&"payload.bin".to_string()),
        format!("{listed:?}"),
    ));

    // Cancellation: abandon a large upload mid-way; the session must survive.
    let big = pseudo_random(64 * 1024 * 1024, 7);
    let partial = format!("{dir}/cancelled.bin");
    let cancelled = tokio::time::timeout(Duration::from_millis(300), async {
        let mut f = sftp.create(&partial).await?;
        f.write_all(&big).await?;
        f.shutdown().await?;
        anyhow::Ok(())
    })
    .await
    .is_err();
    let alive = sftp.metadata(&dir).await.is_ok();
    out.push((
        "POC-04 SFTP cancel mid-transfer, session survives",
        cancelled && alive,
        format!("cancelled={cancelled} session_alive={alive}"),
    ));

    let _ = sftp.remove_file(&partial).await;
    sftp.remove_file(&path).await?;
    sftp.remove_dir(&dir).await?;
    Ok(out)
}

fn finish(r: Report) -> Result<()> {
    let failed = r.0.iter().filter(|(_, ok, _)| !ok).count();
    println!("\nRESULT: {} passed, {} failed", r.0.len() - failed, failed);
    std::process::exit(if failed == 0 { 0 } else { 1 });
}
