//! Spike: exercise the host protocol against a real ADB server.
//! Run: cargo run -p senvakara-adapters-adb --example adb_probe [127.0.0.1:5037]

use senvakara_adapters_adb::{ensure_local_server, find_local_adb, AdbServer};
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = match std::env::args().nth(1) {
        Some(a) => AdbServer { addr: a.parse()? },
        None => {
            let adb = find_local_adb().expect("adb not found");
            println!("adb: {}", adb.display());
            ensure_local_server(&adb).await?
        }
    };
    println!("server {} version {}", server.addr, server.version().await?);
    let devices = server.devices().await?;
    println!("devices: {devices:#?}");
    for d in &devices {
        if let (Some(t), "device") = (d.transport_id, d.state.as_str()) {
            println!(
                "transport {t}: model={:?} sdk={:?}",
                server.getprop(t, "ro.product.model").await?,
                server.getprop(t, "ro.build.version.sdk").await?
            );
            let out = server
                .shell(t, "echo SVK_$((40+2)); id -u", Duration::from_secs(5), 1024)
                .await?;
            println!("shell: {out:?}");
        }
    }
    // A transport id that does not exist must fail as stale, never fall back.
    match server
        .shell(999_999, "echo should-not-run", Duration::from_secs(5), 64)
        .await
    {
        Ok(o) => println!("STALE CHECK FAILED: {o:?}"),
        Err(e) => println!("stale transport rejected: {e:#}"),
    }
    let mut tracker = server.track().await?;
    let first = tokio::time::timeout(Duration::from_secs(3), tracker.next()).await;
    println!(
        "track-devices first snapshot: {:?}",
        first.map(|r| r.map(|v| v.len()))
    );
    Ok(())
}
