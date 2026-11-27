//! Phase 0 spike 2 (WF): prove the remote script path over russh =0.65.2.
//!
//! Reads `creds.local` (written by setup-wsl.sh, gitignored) and runs five
//! checks against that host. Prints a plain-text log; the test password is
//! never printed (every output line is scrubbed of it).
//!
//! Runs on a current-thread tokio runtime with a 50 ms ticker task: if any
//! part blocks the runtime, the tick count falls behind wall time.

use anyhow::{bail, Context, Result};
use russh::client::{self, Handle, Msg};
use russh::keys::{HashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, ChannelWriteHalf, Sig};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, OpenFlags};
use senvakara_adapters_ssh::{
    shell_quote, Auth, ConnectParams, HostKeyPolicy, Secret, SshConnection, SshPtyEvent,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

// ---------------------------------------------------------------- creds

struct Creds {
    host: String,
    port: u16,
    user: String,
    password: Secret,
}

fn load_creds() -> Result<Creds> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/creds.local");
    let text = std::fs::read_to_string(path).context("read creds.local (run setup-wsl.sh)")?;
    let mut get = |k: &str| -> Result<String> {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .map(|v| v.trim().to_string())
            .with_context(|| format!("creds.local: missing {k}"))
    };
    Ok(Creds {
        host: get("host")?,
        port: get("port")?.parse()?,
        user: get("user")?,
        password: Secret::new(get("password")?),
    })
}

static PW: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Print a log line with the test password removed.
fn log(line: impl AsRef<str>) {
    let mut s = line.as_ref().to_string();
    if let Some(pw) = PW.get() {
        s = s.replace(pw.as_str(), "[REDACTED]");
    }
    println!("{s}");
}

// ---------------------------------------------------------------- raw client

struct Recorder;
impl client::Handler for Recorder {
    type Error = russh::Error;
    async fn check_server_key(&mut self, k: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        if let PublicKeyOrCertificate::PublicKey { key, .. } = k {
            log(format!(
                "  raw client: host key {}",
                key.fingerprint(HashAlg::Sha256)
            ));
        }
        // Spike only (loopback test host). Production pins the key.
        Ok(true)
    }
}

async fn raw_connect(c: &Creds) -> Result<Handle<Recorder>> {
    let cfg = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(300)),
        keepalive_interval: Some(Duration::from_secs(15)),
        ..Default::default()
    });
    let mut h = client::connect(cfg, (c.host.as_str(), c.port), Recorder).await?;
    let r = h.authenticate_password(&c.user, c.password.as_str()).await?;
    if !r.success() {
        bail!("password auth refused");
    }
    Ok(h)
}

// ---------------------------------------------------------------- exec_stream prototype

#[derive(Debug)]
enum Event {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
}

#[derive(Debug, Default)]
struct ExecResult {
    exit_status: Option<u32>,
    exit_signal: Option<String>,
    stdout_total: u64,
    stderr_total: u64,
    stdout_kept: Vec<u8>,
    stderr_kept: Vec<u8>,
    truncated: bool,
    timed_out: bool,
    cancelled: bool,
    saw_eof: bool,
    saw_close: bool,
    /// Ordered (stream, len) of every chunk, to show interleaving.
    order: Vec<(&'static str, usize)>,
}

enum Stop {
    Timeout,
    Cancel,
}

/// Prototype of the proposed `SshConnection::exec_stream`:
/// open a session channel, exec `argv` (POSIX-quoted), write `stdin` then
/// send EOF, stream stdout (Data) and stderr (ExtendedData ext=1) separately,
/// keep at most `cap` bytes per stream (counting the rest), stop at
/// `timeout` or when `cancel` flips. On stop it calls `on_stop` (the caller's
/// remote kill), then keeps draining for `drain` to collect the exit status.
#[allow(clippy::too_many_arguments)]
async fn exec_stream<H, F, Fut>(
    h: &Handle<H>,
    argv: &[&str],
    stdin: Option<&[u8]>,
    timeout: Duration,
    cap: usize,
    mut cancel: watch::Receiver<bool>,
    mut on_event: impl FnMut(&Event),
    on_stop: F,
) -> Result<ExecResult>
where
    H: client::Handler,
    F: FnOnce(ChannelWriteHalf<Msg>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let command = argv.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ");
    let ch = h.channel_open_session().await?;
    ch.exec(true, command).await?;
    let (mut rd, wr) = ch.split();
    if let Some(bytes) = stdin {
        wr.data_bytes(bytes.to_vec()).await?;
    }
    wr.eof().await?;

    let mut res = ExecResult::default();
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut on_stop = Some((on_stop, wr));
    let mut drain_until: Option<tokio::time::Instant> = None;

    loop {
        let msg = if let Some(until) = drain_until {
            match tokio::time::timeout_at(until, rd.wait()).await {
                Ok(m) => m,
                Err(_) => break, // gave up waiting for the channel to close
            }
        } else {
            let stop = tokio::select! {
                m = rd.wait() => { if let Some(m) = m { handle_msg(m, cap, &mut res, &mut on_event); continue; } else { break; } }
                _ = &mut deadline => Stop::Timeout,
                r = cancel.changed() => { if r.is_err() || !*cancel.borrow() { continue; } Stop::Cancel }
            };
            match stop {
                Stop::Timeout => res.timed_out = true,
                Stop::Cancel => res.cancelled = true,
            }
            if let Some((f, wr)) = on_stop.take() {
                f(wr).await;
            }
            drain_until = Some(tokio::time::Instant::now() + Duration::from_secs(10));
            continue;
        };
        match msg {
            Some(m) => handle_msg(m, cap, &mut res, &mut on_event),
            None => break,
        }
    }
    Ok(res)
}

fn handle_msg(m: ChannelMsg, cap: usize, res: &mut ExecResult, on_event: &mut impl FnMut(&Event)) {
    match m {
        ChannelMsg::Data { data } => {
            res.stdout_total += data.len() as u64;
            res.order.push(("stdout", data.len()));
            keep(&mut res.stdout_kept, &data, cap, &mut res.truncated);
            on_event(&Event::Stdout(data.to_vec()));
        }
        ChannelMsg::ExtendedData { data, ext } => {
            if ext != 1 {
                log(format!("  unexpected ext={ext}"));
            }
            res.stderr_total += data.len() as u64;
            res.order.push(("stderr", data.len()));
            keep(&mut res.stderr_kept, &data, cap, &mut res.truncated);
            on_event(&Event::Stderr(data.to_vec()));
        }
        ChannelMsg::ExitStatus { exit_status } => res.exit_status = Some(exit_status),
        ChannelMsg::ExitSignal { signal_name, .. } => {
            res.exit_signal = Some(format!("{signal_name:?}"))
        }
        ChannelMsg::Eof => res.saw_eof = true,
        ChannelMsg::Close => res.saw_close = true,
        _ => {}
    }
}

fn keep(buf: &mut Vec<u8>, data: &[u8], cap: usize, truncated: &mut bool) {
    let room = cap.saturating_sub(buf.len());
    if data.len() > room {
        *truncated = true;
    }
    buf.extend_from_slice(&data[..data.len().min(room)]);
}

/// Simple buffered exec on the raw handle (for checks around the tests).
async fn sh<H: client::Handler>(h: &Handle<H>, argv: &[&str], stdin: Option<&[u8]>) -> Result<ExecResult> {
    let (_tx, rx) = watch::channel(false);
    exec_stream(h, argv, stdin, Duration::from_secs(30), 1 << 20, rx, |_| {}, |_w| async {}).await
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).trim_end().to_string()
}

fn show(name: &str, r: &ExecResult, t0: Instant) {
    log(format!(
        "  [{name}] exit_status={:?} exit_signal={:?} eof={} close={} timed_out={} cancelled={} truncated={} stdout_total={} stderr_total={} elapsed={:.2}s",
        r.exit_status, r.exit_signal, r.saw_eof, r.saw_close, r.timed_out, r.cancelled, r.truncated,
        r.stdout_total, r.stderr_total, t0.elapsed().as_secs_f64()
    ));
    if r.stdout_total < 4096 {
        for l in text(&r.stdout_kept).lines() {
            log(format!("    stdout| {l}"));
        }
    }
    if r.stderr_total < 4096 {
        for l in text(&r.stderr_kept).lines() {
            log(format!("    stderr| {l}"));
        }
    }
}

// ---------------------------------------------------------------- scripts

const S1_SUDO_EOF: &str = r#"echo "uid=$(id -u) user=$(id -un)"
if IFS= read -r line; then echo "stdin-line-len=${#line}"; else echo "stdin=eof"; fi
echo "tty=$(tty 2>&1 || true)"
echo done
"#;

const S1_CLOSE_MARK: &str = r#"touch "$1"
echo "uid=$(id -u)"
sleep 1
echo after-sleep
"#;

const S2_STREAMS: &str = r#"echo out-1
echo err-1 >&2
sleep 0.2
echo out-2
echo err-2 >&2
exit 7
"#;

/// apt-like: a foreground loop plus children, one of which ignores TERM.
const S3_LONG: &str = r#"echo "start pid=$$ pgid=$(ps -o pgid= -p $$ | tr -d ' ') sid=$(ps -o sid= -p $$ | tr -d ' ')"
sleep 1000 &
( trap '' TERM; while :; do sleep 1; done ) &
for i in $(seq 1 1000); do echo "tick $i"; sleep 1; done
"#;

const S4_FLOOD: &str = r#"head -c 5000000 /dev/zero | tr '\0' 'x'
echo
echo err-line >&2
sleep 30
echo never
"#;

/// Launcher: new session (pgid = pid of this bash), record it, then exec the
/// script. Arguments: $1 pidfile, $2 script.
const LAUNCH: &str = r#"echo $$ > "$1"; exec bash -euo pipefail "$2""#;

/// Same launcher, but the script's stdin is /dev/null: whatever sudo did not
/// consume (e.g. the password when sudo did not ask for one) never reaches it.
const LAUNCH_NULL: &str = r#"echo $$ > "$1"; exec bash -euo pipefail "$2" </dev/null"#;

/// Kill a run's process group: TERM, wait 3 s, then KILL. Refuses a pgid
/// that is not a plain number > 1 (`kill -- -1` would hit every process).
const KILL_GROUP: &str = r#"pg=$(cat -- "$1")
case $pg in ''|*[!0-9]*) echo "bad pgid" >&2; exit 2;; esac
[ "$pg" -gt 1 ] || { echo "bad pgid" >&2; exit 2; }
kill -TERM -- "-$pg" 2>/dev/null || { echo "group $pg already gone"; exit 0; }
for i in $(seq 1 30); do
  kill -0 -- "-$pg" 2>/dev/null || { echo "group $pg gone after TERM (${i}x100ms)"; exit 0; }
  sleep 0.1
done
kill -KILL -- "-$pg" 2>/dev/null || true
sleep 0.2
if kill -0 -- "-$pg" 2>/dev/null; then echo "group $pg STILL ALIVE"; exit 1; fi
echo "group $pg killed with KILL after TERM grace"
"#;

// ---------------------------------------------------------------- main

fn main() -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(run())
}

async fn run() -> Result<()> {
    let creds = load_creds()?;
    PW.set(creds.password.to_string()).ok();
    let pw_line = Secret::new(format!("{}\n", creds.password.as_str()));

    // Ticker: proves the runtime is never blocked.
    let ticks = Arc::new(AtomicU64::new(0));
    {
        let ticks = ticks.clone();
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_millis(50));
            loop {
                iv.tick().await;
                ticks.fetch_add(1, Ordering::Relaxed);
            }
        });
    }
    let t_all = Instant::now();

    log(format!("== connect {}@{}:{}", creds.user, creds.host, creds.port));
    let raw = raw_connect(&creds).await?;
    let conn = SshConnection::connect(&ConnectParams {
        host: creds.host.clone(),
        port: creds.port,
        user: creds.user.clone(),
        auth: Auth::Password(creds.password.clone()),
        host_key: HostKeyPolicy::TrustOnFirstUse,
        connect_timeout: Duration::from_secs(10),
    })
    .await?;
    log(format!("  adapter SshConnection: host key {}", conn.host_key_fingerprint()));
    let info = sh(&raw, &["sh", "-c", ". /etc/os-release; echo \"$PRETTY_NAME\"; uname -r; sudo -V | head -1; ssh -V 2>&1; umask"], None).await?;
    show("host", &info, Instant::now());

    // ------------------------------------------------------------ 5 (setup): SFTP run dir
    log("\n== 5. SFTP upload to ~/.cache/senvakara/run-<id>/ (dir 0700, file 0700)");
    let sftp = conn.sftp().await?;
    let home = sftp.canonicalize(".").await?;
    let run_id = format!("{}", std::process::id());
    let cache = format!("{home}/.cache");
    let base = format!("{cache}/senvakara");
    let dir = format!("{base}/run-{run_id}");
    for d in [&cache, &base, &dir] {
        if !sftp.try_exists(d.as_str()).await? {
            sftp.create_dir(d.as_str()).await?;
            log(format!("  mkdir {d}"));
        }
    }
    for d in [&base, &dir] {
        senvakara_adapters_ssh::sftp::chmod(&sftp, d, 0o700).await?;
    }
    let mut paths = std::collections::BTreeMap::new();
    for (name, body) in [
        ("s1.sh", S1_SUDO_EOF),
        ("s1close.sh", S1_CLOSE_MARK),
        ("s2.sh", S2_STREAMS),
        ("s3.sh", S3_LONG),
        ("s4.sh", S4_FLOOD),
    ] {
        let p = format!("{dir}/{name}");
        upload_bytes(&sftp, &p, body.as_bytes(), 0o700).await?;
        paths.insert(name, p);
    }
    let st = sh(&raw, &["sh", "-c", "stat -c '%a %U:%G %s %n' \"$1\" \"$1\"/* \"$2\"", "sv", &dir, &base], None).await?;
    show("stat", &st, Instant::now());
    // Exclusive create refuses to clobber an existing file.
    let again = upload_bytes(&sftp, &paths["s1.sh"], b"x", 0o700).await;
    log(format!("  re-upload same path with CREATE|EXCLUDE -> {}", match &again { Ok(_) => "ok (UNEXPECTED)".into(), Err(e) => format!("error as expected: {e:#}") }));

    let sudo = |script: &str| -> Vec<String> {
        ["sudo", "-k", "-S", "-p", "", "--", "bash", "-euo", "pipefail", script]
            .iter()
            .map(|s| s.to_string())
            .collect()
    };

    // ------------------------------------------------------------ 1
    log("\n== 1a. sudo -S: password + EOF on stdin, script must see EOF");
    let t0 = Instant::now();
    let a = sudo(&paths["s1.sh"]);
    let r = sh(&raw, &a.iter().map(String::as_str).collect::<Vec<_>>(), Some(pw_line.as_bytes())).await?;
    show("1a", &r, t0);

    log("\n== 1b. sudo -S: password + one more line, then EOF (does sudo leave the rest for the script?)");
    let t0 = Instant::now();
    let mut more = pw_line.as_bytes().to_vec();
    more.extend_from_slice(b"hello\n");
    let r = sh(&raw, &a.iter().map(String::as_str).collect::<Vec<_>>(), Some(&more)).await?;
    more.iter_mut().for_each(|b| *b = 0);
    show("1b", &r, t0);

    log("\n== 1c. sudo -S with a wrong password, then EOF (script must not run)");
    let t0 = Instant::now();
    let r = sh(&raw, &a.iter().map(String::as_str).collect::<Vec<_>>(), Some(b"definitely-wrong\n")).await?;
    show("1c", &r, t0);

    log("\n== 1e. production launcher: sudo -k -S + setsid + pidfile + script stdin </dev/null; send password + extra line");
    let t0 = Instant::now();
    let pf1 = format!("{dir}/pgid1e");
    let r = sh(
        &raw,
        &["sudo", "-k", "-S", "-p", "", "--", "setsid", "-w", "bash", "-c", LAUNCH_NULL, "sv-run", &pf1, &paths["s1.sh"]],
        Some(&{
            let mut v = pw_line.as_bytes().to_vec();
            v.extend_from_slice(b"hello\n");
            v
        }),
    )
    .await?;
    show("1e", &r, t0);
    log("  same launcher without sudo (as if sudo needed no password): password line goes to stdin");
    let t0 = Instant::now();
    let r = sh(&raw, &["setsid", "-w", "bash", "-c", LAUNCH_NULL, "sv-run", &pf1, &paths["s1.sh"]], Some(pw_line.as_bytes())).await?;
    show("1e no-sudo", &r, t0);

    log("\n== 1d. existing adapter: spawn() + SshPtyWriter::write(pw) + SshPtyWriter::close()");
    let t0 = Instant::now();
    let mark = format!("{dir}/ran-close");
    let (w, mut rdr) = conn
        .spawn(&["sudo", "-k", "-S", "-p", "", "--", "bash", "-euo", "pipefail", &paths["s1close.sh"], &mark])
        .await?;
    w.write(pw_line.as_bytes()).await?;
    w.close().await?;
    let mut evs = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(5), rdr.next()).await {
            Ok(Some(ev)) => {
                let done = matches!(ev, SshPtyEvent::Exit(_));
                evs.push(match ev {
                    SshPtyEvent::Data(d) => format!("Data({:?})", text(&d)),
                    SshPtyEvent::Exit(s) => format!("Exit({s:?})"),
                });
                if done {
                    break;
                }
            }
            Ok(None) => {
                evs.push("reader None (channel gone)".into());
                break;
            }
            Err(_) => {
                evs.push("no event within 5 s".into());
                break;
            }
        }
    }
    log(format!("  [1d] events after close(): {evs:?} elapsed={:.2}s", t0.elapsed().as_secs_f64()));
    tokio::time::sleep(Duration::from_secs(2)).await;
    let m = sh(&raw, &["sh", "-c", "if [ -e \"$1\" ]; then stat -c 'marker exists, owner %U' \"$1\"; else echo 'marker missing'; fi", "sv", &mark], None).await?;
    show("1d marker", &m, Instant::now());

    // ------------------------------------------------------------ 2
    log("\n== 2. stdout vs stderr (Data vs ExtendedData ext=1) + exit status");
    let t0 = Instant::now();
    let r = sh(&raw, &["bash", "-euo", "pipefail", &paths["s2.sh"]], None).await?;
    show("2 plain", &r, t0);
    log(format!("  chunk order: {:?}", r.order));
    let t0 = Instant::now();
    let r = sh(&raw, &sudo(&paths["s2.sh"]).iter().map(String::as_str).collect::<Vec<_>>(), Some(pw_line.as_bytes())).await?;
    show("2 via sudo", &r, t0);
    log(format!("  chunk order: {:?}", r.order));

    // ------------------------------------------------------------ 3
    let pidfile = format!("{dir}/pgid");
    let launch_root = |script: &str| -> Vec<String> {
        ["sudo", "-k", "-S", "-p", "", "--", "setsid", "-w", "bash", "-c", LAUNCH, "sv-run", &pidfile, script]
            .iter()
            .map(|s| s.to_string())
            .collect()
    };
    let kill_root = |pw: &[u8]| {
        let pidfile = pidfile.clone();
        let pw = pw.to_vec();
        let raw = &raw;
        async move {
            let t = Instant::now();
            let r = sh(raw, &["sudo", "-k", "-S", "-p", "", "--", "bash", "-c", KILL_GROUP, "sv-kill", &pidfile], Some(&pw)).await;
            match r {
                Ok(r) => show("kill exec", &r, t),
                Err(e) => log(format!("  kill exec failed: {e:#}")),
            }
        }
    };
    let group_state = |label: &'static str| {
        let pidfile = pidfile.clone();
        let raw = &raw;
        async move {
            let r = sh(raw, &["sh", "-c", "pg=$(cat \"$1\" 2>/dev/null); echo \"pgid=$pg\"; ps -o pid=,user=,stat=,args= -g \"$pg\" 2>/dev/null || echo '(no processes in group)'", "sv", &pidfile], None).await;
            if let Ok(r) = r {
                show(label, &r, Instant::now());
            }
        }
    };

    log("\n== 3a. cancel: setsid + pidfile, kill -TERM/-KILL the group from a second exec");
    let t0 = Instant::now();
    let (tx, rx) = watch::channel(false);
    let a3 = launch_root(&paths["s3.sh"]);
    let a3: Vec<&str> = a3.iter().map(String::as_str).collect();
    let cancel_after = async {
        tokio::time::sleep(Duration::from_secs(3)).await;
        group_state("3a group before cancel").await;
        // Password must not be visible in any process argv.
        let ps = sh(&raw, &["ps", "-eo", "args"], None).await.unwrap();
        let leaked = String::from_utf8_lossy(&ps.stdout_kept).contains(creds.password.as_str());
        log(format!("  password present in `ps -eo args` output: {leaked}"));
        tx.send(true).ok();
    };
    let (r, _) = tokio::join!(
        exec_stream(&raw, &a3, Some(pw_line.as_bytes()), Duration::from_secs(60), 1 << 20, rx, |_| {}, |_w| kill_root(pw_line.as_bytes())),
        cancel_after
    );
    let r = r?;
    show("3a main channel", &r, t0);
    group_state("3a group after cancel").await;

    log("\n== 3b. control: only close the channel (no kill) -> does the script keep running?");
    let t0 = Instant::now();
    let (tx, rx) = watch::channel(false);
    let cancel_after = async {
        tokio::time::sleep(Duration::from_secs(2)).await;
        tx.send(true).ok();
    };
    let (r, _) = tokio::join!(
        exec_stream(&raw, &a3, Some(pw_line.as_bytes()), Duration::from_secs(60), 1 << 20, rx, |_| {}, |w: ChannelWriteHalf<Msg>| async move {
            w.close().await.ok();
        }),
        cancel_after
    );
    show("3b main channel", &r?, t0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    group_state("3b group 2 s after channel close").await;
    kill_root(pw_line.as_bytes()).await;
    group_state("3b group after kill").await;

    log("\n== 3c. control: SSH `signal` request (Sig::TERM) on the channel instead of kill");
    let t0 = Instant::now();
    let (tx, rx) = watch::channel(false);
    let cancel_after = async {
        tokio::time::sleep(Duration::from_secs(2)).await;
        tx.send(true).ok();
    };
    let (r, _) = tokio::join!(
        exec_stream(&raw, &a3, Some(pw_line.as_bytes()), Duration::from_secs(60), 1 << 20, rx, |_| {}, |w: ChannelWriteHalf<Msg>| async move {
            let res = w.signal(Sig::TERM).await;
            log(format!("  signal(TERM) sent: {res:?}"));
            tokio::time::sleep(Duration::from_secs(3)).await;
        }),
        cancel_after
    );
    show("3c main channel (drain gives up after 10 s if nothing ends)", &r?, t0);
    group_state("3c group after signal request").await;
    kill_root(pw_line.as_bytes()).await;
    group_state("3c group after kill").await;

    // ------------------------------------------------------------ 4
    log("\n== 4. timeout 5 s + 2 MB cap on a 5 MB flood that then sleeps 30 s (no sudo)");
    let t0 = Instant::now();
    let ticks0 = ticks.load(Ordering::Relaxed);
    let (_tx, rx) = watch::channel(false);
    let pidfile4 = format!("{dir}/pgid4");
    let a4 = ["setsid", "-w", "bash", "-c", LAUNCH, "sv-run", &pidfile4, &paths["s4.sh"]];
    let mut first_byte: Option<f64> = None;
    let mut last_byte = 0.0f64;
    let r = exec_stream(&raw, &a4, None, Duration::from_secs(5), 2 * 1024 * 1024, rx, |e| {
        let t = t0.elapsed().as_secs_f64();
        if matches!(e, Event::Stdout(_)) {
            first_byte.get_or_insert(t);
            last_byte = t;
        }
    }, |_w| {
        let raw = &raw;
        let pf = pidfile4.clone();
        async move {
            let t = Instant::now();
            if let Ok(r) = sh(raw, &["bash", "-c", KILL_GROUP, "sv-kill", &pf], None).await {
                show("kill exec (no sudo)", &r, t);
            }
        }
    })
    .await?;
    let el = t0.elapsed().as_secs_f64();
    let dt = ticks.load(Ordering::Relaxed) - ticks0;
    show("4 main channel", &r, t0);
    log(format!(
        "  stdout kept={} bytes (cap {}), first byte at {:.2}s, last stdout at {:.2}s, stderr kept={:?}",
        r.stdout_kept.len(), 2 * 1024 * 1024, first_byte.unwrap_or(-1.0), last_byte, text(&r.stderr_kept)
    ));
    log(format!(
        "  runtime ticker: {dt} ticks of 50 ms in {el:.2}s (expected ~{:.0}) -> runtime not blocked",
        el / 0.05
    ));

    // ------------------------------------------------------------ 5 (cleanup)
    log("\n== 5. cleanup run dir with adapter sftp::remove (tree contains root-owned files from sudo runs)");
    let ls = sh(&raw, &["sh", "-c", "ls -ln \"$1\" | tail -n +2", "sv", &dir], None).await?;
    show("run dir before cleanup", &ls, Instant::now());
    let rm = senvakara_adapters_ssh::sftp::remove(&sftp, &dir).await;
    log(format!("  sftp::remove -> {:?}", rm.as_ref().map_err(|e| format!("{e:#}"))));
    let gone = sh(&raw, &["sh", "-c", "[ -e \"$1\" ] && echo still-there || echo gone", "sv", &dir], None).await?;
    show("after cleanup", &gone, Instant::now());

    let total = t_all.elapsed().as_secs_f64();
    let all_ticks = ticks.load(Ordering::Relaxed);
    log(format!("\n== total {total:.1}s, ticker {all_ticks} ticks (expected ~{:.0})", total / 0.05));
    conn.close().await.ok();
    Ok(())
}

/// Prototype of the proposed `sftp::upload_bytes`: exclusive create with the
/// mode set at open time (never a world-readable window), write, close.
async fn upload_bytes(sftp: &SftpSession, path: &str, bytes: &[u8], mode: u32) -> Result<()> {
    let mut attrs = FileAttributes::empty();
    attrs.permissions = Some(mode);
    let mut f = sftp
        .open_with_flags_and_attributes(
            path.to_string(),
            OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            attrs,
        )
        .await
        .context("create remote file")?;
    f.write_all(bytes).await?;
    f.shutdown().await?;
    Ok(())
}
