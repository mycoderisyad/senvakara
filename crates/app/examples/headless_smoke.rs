//! Headless smoke test of the service layer, for use where no window can be
//! opened: starts an [`App`] on a throw-away data folder and exercises the
//! parts that need no remote host or phone (lists, local metrics, a local
//! PowerShell terminal with ordered input, resize, exit and "viewer gone").
//!
//! Every call is awaited from a second runtime that has no timer and no I/O
//! driver, the way a UI event loop would: it only works because `App` runs
//! its work on its own runtime.
//!
//! Run: cargo run -p senvakara-app --example headless_smoke

use anyhow::{bail, ensure, Result};
use senvakara_app::{App, AppPaths, ShellRequest, Sink, TermEvent};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("senvakara-smoke-{}", std::process::id()));
    let paths = AppPaths::in_dir(&dir);
    let _instance = senvakara_app::single_instance(&paths)?;
    ensure!(
        senvakara_app::single_instance(&paths).is_err(),
        "a second instance on the same data folder must be refused"
    );

    let backend = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let app = App::start(paths.clone(), backend.handle().clone())?;
    // Stands in for a UI thread: no tokio context of its own.
    let ui = tokio::runtime::Builder::new_current_thread().build()?;

    let hosts = app.hosts_list()?;
    ensure!(
        hosts.iter().any(|h| h.is_local),
        "the local host is registered"
    );
    let local = hosts.iter().find(|h| h.is_local).unwrap().id.clone();
    println!(
        "hosts: {}, devices: {}",
        hosts.len(),
        app.devices_list()?.len()
    );
    println!("adb: {:?}", app.adb_status().local_adb_path);

    let first = ui.block_on(app.host_metrics(local.clone()))?;
    std::thread::sleep(Duration::from_millis(300));
    let second = ui.block_on(app.host_metrics(local))?;
    ensure!(first.cpu_percent.is_none() && second.cpu_percent.is_some());
    println!(
        "metrics: cpu {:.1} %, mem {} / {} MB",
        second.cpu_percent.unwrap_or(0.0),
        second.mem_used >> 20,
        second.mem_total >> 20
    );
    println!("wsl: {:?}", ui.block_on(app.wsl_distros()));

    terminal(&app, &ui)?;
    viewer_gone(&app, &ui)?;

    drop(app);
    backend.shutdown_timeout(Duration::from_secs(2));
    let _ = std::fs::remove_dir_all(&dir);
    println!("ok");
    Ok(())
}

/// Wait for output containing `needle`, answering ConPTY's cursor-position
/// query on the way (the shell does not start until it gets an answer).
fn wait_for(
    app: &App,
    id: u32,
    events: &mpsc::Receiver<TermEvent>,
    seen: &mut String,
    needle: &str,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !seen.contains(needle) {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(TermEvent::Data(bytes)) => {
                let text = String::from_utf8_lossy(&bytes);
                if text.contains("\x1b[6n") {
                    app.term_write(id, b"\x1b[1;1R".to_vec())?;
                }
                seen.push_str(&text);
            }
            Ok(TermEvent::Exit(code)) => bail!("shell exited early ({code:?}) before {needle:?}"),
            Err(_) => bail!("no {needle:?} within 30 s; output so far: {seen:?}"),
        }
    }
    Ok(())
}

fn terminal(app: &Arc<App>, ui: &tokio::runtime::Runtime) -> Result<()> {
    let (tx, events) = mpsc::channel();
    let sink: Sink<TermEvent> = Arc::new(move |e| tx.send(e).is_ok());
    let id = ui.block_on(app.term_open(ShellRequest::PowerShell, 100, 30, sink))?;
    let mut seen = String::new();

    // Many small writes with a resize in between must arrive in order.
    for piece in ["Write-Output ('smo' + ", "'ke-' + ", "(6 * 7))", "\r"] {
        app.term_write(id, piece.as_bytes().to_vec())?;
    }
    app.term_resize(id, 120, 40)?;
    wait_for(app, id, &events, &mut seen, "smoke-42")?;
    app.term_write(
        id,
        b"Write-Output $Host.UI.RawUI.WindowSize.Width\r".to_vec(),
    )?;
    wait_for(app, id, &events, &mut seen, "120")?;
    println!("terminal {id}: ordered input and resize ok");

    app.term_write(id, b"exit\r".to_vec())?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let exit = loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(TermEvent::Exit(code)) => break Some(code),
            Ok(TermEvent::Data(_)) => {}
            Err(_) => break None,
        }
    };
    match exit {
        Some(code) => println!("terminal {id}: exit event received ({code:?})"),
        // ConPTY keeps the output pipe open after the shell exits; the end
        // then only comes with the close below.
        None => println!("terminal {id}: no exit event before close (ConPTY keeps the pipe open)"),
    }
    app.term_close(id)?;
    ensure!(
        app.term_write(id, b"x".to_vec()).is_err(),
        "closed session refuses input"
    );
    Ok(())
}

/// A sink that reports "viewer gone" must end the session by itself.
fn viewer_gone(app: &Arc<App>, ui: &tokio::runtime::Runtime) -> Result<()> {
    let sink: Sink<TermEvent> = Arc::new(|_| false);
    let id = ui.block_on(app.term_open(ShellRequest::PowerShell, 80, 24, sink))?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while app.term_write(id, Vec::new()).is_ok() {
        ensure!(
            Instant::now() < deadline,
            "session {id} was not closed after its viewer left"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("terminal {id}: closed after its viewer left");
    Ok(())
}
