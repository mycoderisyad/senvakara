//! POC-03 spike: prove Windows local PTY (PowerShell) and WSL PTY work,
//! including interactive input, resize and explicit termination.
//!
//! Run: cargo run -p senvakara-adapters-pty --example poc03_pty -- [wsl-distro]

use senvakara_adapters_pty::{PtySession, ShellKind};
use std::io::Read;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Read output until `needle` appears (`None` = just drain for `timeout`).
/// Acts like a minimal terminal: ConPTY sends a cursor-position query
/// (DSR, `ESC[6n`) at startup and blocks until the terminal answers.
/// xterm.js answers automatically in the real app.
fn collect_until(
    s: &mut PtySession,
    rx: &mpsc::Receiver<Vec<u8>>,
    needle: Option<&str>,
    timeout: Duration,
) -> (bool, String) {
    let start = Instant::now();
    let mut buf = String::new();
    while start.elapsed() < timeout {
        if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(100)) {
            let text = String::from_utf8_lossy(&chunk);
            for _ in 0..text.matches("\x1b[6n").count() {
                s.write_input(b"\x1b[1;1R").expect("answer DSR");
            }
            buf.push_str(&text);
            if needle.is_some_and(|n| buf.contains(n)) {
                return (true, buf);
            }
        }
    }
    (false, buf)
}

fn run_case(name: &str, kind: ShellKind, input: &str, needle: &str) -> bool {
    println!("== {name} ==");
    let mut s = match PtySession::spawn(&kind, &[], 120, 30) {
        Ok(s) => s,
        Err(e) => {
            println!("FAIL spawn: {e:#}");
            return false;
        }
    };
    let mut reader = s.reader().expect("reader");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut b = [0u8; 4096];
        while let Ok(n) = reader.read(&mut b) {
            if n == 0 || tx.send(b[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    // Let the shell finish starting (and answer the DSR) before typing.
    let _ = collect_until(&mut s, &rx, None, Duration::from_secs(3));
    s.resize(100, 40).expect("resize");
    s.write_input(input.as_bytes()).expect("write");
    let (ok, out) = collect_until(&mut s, &rx, Some(needle), Duration::from_secs(20));
    let tail: String = out
        .chars()
        .rev()
        .take(300)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    println!("output tail: {:?}", tail);

    s.kill().expect("kill");
    let exited = (0..50).any(|_| {
        thread::sleep(Duration::from_millis(100));
        matches!(s.try_exit_code(), Ok(Some(_)))
    });
    println!("interactive marker seen: {ok}; terminated after kill: {exited}");
    ok && exited
}

fn main() {
    let distro = std::env::args().nth(1);
    let ps = run_case(
        "PowerShell",
        ShellKind::PowerShell,
        "Write-Output (\"SVK_\" + \"PS_OK \" + $PSVersionTable.PSVersion)\r\n",
        "SVK_PS_OK",
    );
    let wsl = run_case(
        "WSL",
        ShellKind::Wsl { distro },
        "echo SVK_$((40+2))_WSL_OK $(uname -sr); stty size\r",
        "SVK_42_WSL_OK",
    );
    println!(
        "\nRESULT PowerShell={} WSL={}",
        if ps { "PASS" } else { "FAIL" },
        if wsl { "PASS" } else { "FAIL" }
    );
    std::process::exit(if ps && wsl { 0 } else { 1 });
}
