//! Spike: start scrcpy on the first device of a server, read video packets,
//! send harmless control messages (wake screen, collapse panels).
//! Run: cargo run -p senvakara-adapters-scrcpy --example mirror_probe [server-addr]

use senvakara_adapters_adb::AdbServer;
use senvakara_adapters_scrcpy::{control::Input, start, stop, verified_server, MirrorOptions};
use std::time::{Duration, Instant};

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
    let t = dev.transport_id.unwrap();
    let jar = verified_server()?;
    let t0 = Instant::now();
    let mut m = start(server, t, jar, &MirrorOptions::default()).await?;
    println!(
        "started in {:?}: device={:?} codec=0x{:08x} {}x{}",
        t0.elapsed(),
        m.device_name,
        m.codec_id,
        m.width,
        m.height
    );
    m.control
        .send(&Input::Key {
            action: 0,
            keycode: 224,
            meta_state: 0,
        })
        .await?;
    m.control
        .send(&Input::Key {
            action: 1,
            keycode: 224,
            meta_state: 0,
        })
        .await?;
    m.control.send(&Input::CollapsePanels).await?;
    let (mut n, mut keys, mut bytes, mut codec) = (0, 0, 0usize, None);
    let first = Instant::now();
    while first.elapsed() < Duration::from_secs(4) {
        match tokio::time::timeout(Duration::from_secs(2), m.video.next()).await {
            Ok(Ok(p)) => {
                n += 1;
                bytes += p.data.len();
                if p.key {
                    keys += 1;
                }
                if p.codec.is_some() {
                    codec = p.codec;
                }
            }
            Ok(Err(e)) => {
                println!("stream error: {e:#}");
                break;
            }
            Err(_) => {
                println!("(no new frame within 2 s — static screen)");
            }
        }
    }
    println!("packets={n} key={keys} bytes={bytes} codec={codec:?}");
    stop(m).await;
    let wake = server
        .shell(
            t,
            "dumpsys power | grep -m1 mWakefulness=",
            Duration::from_secs(5),
            4096,
        )
        .await?;
    println!("after wake key: {}", wake.output.trim());
    Ok(())
}
