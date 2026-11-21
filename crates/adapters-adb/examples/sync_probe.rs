//! Spike: sync protocol + exec against a real device.
//! Run: cargo run -p senvakara-adapters-adb --example sync_probe [server-addr]

use senvakara_adapters_adb::{sync, AdbServer};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = std::env::args()
        .nth(1)
        .map(|a| AdbServer {
            addr: a.parse().unwrap(),
        })
        .unwrap_or_else(AdbServer::local_default);
    let dev = server
        .devices()
        .await?
        .into_iter()
        .find(|d| d.state == "device")
        .expect("no device");
    let t = dev.transport_id.expect("transport id");
    println!("device {} transport {t}", dev.serial);

    let mut s = server.sync(t).await?;
    let listing = sync::list(&mut s, "/sdcard/").await?;
    println!(
        "LIST /sdcard/: {} entries, first: {:?}",
        listing.len(),
        listing.iter().take(4).map(|e| &e.name).collect::<Vec<_>>()
    );

    let payload: Vec<u8> = (0..300_000u32).map(|i| (i * 31 % 251) as u8).collect();
    let path = "/data/local/tmp/svk_sync_probe.bin";
    let no = AtomicBool::new(false);
    sync::send(
        &mut s,
        &mut payload.as_slice(),
        path,
        0o644,
        1_700_000_000,
        &no,
        |_| {},
    )
    .await?;
    let st = sync::stat(&mut s, path).await?;
    println!("STAT after SEND: size={} mode={:o}", st.size, st.mode);
    let mut back = Vec::new();
    sync::recv(&mut s, path, &mut back, &no, |_| {}).await?;
    println!("RECV round trip identical: {}", back == payload);
    let missing = sync::recv(
        &mut s,
        "/data/local/tmp/does-not-exist",
        &mut Vec::new(),
        &no,
        |_| {},
    )
    .await;
    println!(
        "RECV missing file -> {:?}",
        missing.map_err(|e| e.to_string())
    );
    sync::quit(&mut s).await;
    let rm = server
        .shell(
            t,
            &format!("rm -f {path}; ls {path} 2>&1"),
            Duration::from_secs(5),
            1024,
        )
        .await?;
    println!("cleanup: {}", rm.output.trim());

    let png = server
        .exec_bytes(t, "screencap -p", Duration::from_secs(15), 32 * 1024 * 1024)
        .await?;
    println!(
        "screencap: {} bytes, PNG signature ok: {}",
        png.len(),
        png.starts_with(b"\x89PNG\r\n\x1a\n")
    );
    Ok(())
}
