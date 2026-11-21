//! Spike UI-MIG-04: a native VNC client (`vnc-rs`) over the SSH `direct-tcpip` channel, with no
//! WebSocket bridge and no web view.
//!
//! Starts x11vnc on the first trusted remote host exactly as the Desktop button does (loopback
//! only, one-time password, one client), receives the screen for a few seconds, moves the
//! pointer and presses Shift once, writes the framebuffer to a PNG and stops x11vnc.
//! Works on a copy of the app database so a running Senvakara is not disturbed.
//!
//! Run: cargo run -p senvakara-app --example vnc_probe -- <out.png>

use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use senvakara_app::desktop::{start_vnc, stop_x11vnc};
use senvakara_app::AppState;
use vnc::{PixelFormat, VncConnector, VncEncoding, VncEvent, X11Event};

struct Screen {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl Screen {
    fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.rgba = vec![0; width * height * 4];
    }

    fn draw(&mut self, rect: vnc::Rect, data: &[u8]) {
        let (x, y, w, h) = (
            rect.x as usize,
            rect.y as usize,
            rect.width as usize,
            rect.height as usize,
        );
        if x + w > self.width || y + h > self.height || data.len() < w * h * 4 {
            return;
        }
        for row in 0..h {
            let from = row * w * 4;
            let to = ((y + row) * self.width + x) * 4;
            self.rgba[to..to + w * 4].copy_from_slice(&data[from..from + w * 4]);
        }
    }

    fn copy(&mut self, dst: vnc::Rect, src: vnc::Rect) {
        let (w, h) = (src.width as usize, src.height as usize);
        let mut block = Vec::with_capacity(w * h * 4);
        for row in 0..h {
            let from = ((src.y as usize + row) * self.width + src.x as usize) * 4;
            block.extend_from_slice(&self.rgba[from..from + w * 4]);
        }
        self.draw(
            vnc::Rect {
                x: dst.x,
                y: dst.y,
                width: src.width,
                height: src.height,
            },
            &block,
        );
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let out = std::env::args()
        .nth(1)
        .context("usage: vnc_probe <out.png>")?;

    let source = std::path::PathBuf::from(std::env::var("APPDATA")?)
        .join("dev.senvakara.desktop/senvakara.db");
    let copy = std::env::temp_dir().join("senvakara-vnc-probe.db");
    std::fs::copy(&source, &copy).context("copy app database")?;
    // The database runs in WAL mode: recent rows live in the side files.
    for suffix in ["-wal", "-shm"] {
        let side = |path: &std::path::Path| {
            std::path::PathBuf::from(format!("{}{suffix}", path.display()))
        };
        if side(&source).exists() {
            std::fs::copy(side(&source), side(&copy)).context("copy app database side file")?;
        }
    }
    let state = AppState::open(copy.clone())?;
    let host = state
        .db
        .lock()
        .unwrap()
        .list_hosts()?
        .into_iter()
        .find(|host| !host.is_local && host.ssh.is_some())
        .context("no remote host registered")?;

    let started = Instant::now();
    let conn = state
        .connection(&host.id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    println!("ssh connected in {:?}", started.elapsed());
    let target = start_vnc(&conn).await.map_err(|e| anyhow!("{e:?}"))?;
    println!(
        "x11vnc up after {:?}: display {} desktop {:?}, loopback port verified",
        started.elapsed(),
        target.display,
        target.desktop
    );
    let vnc_port = target.vnc_port;

    let result = async {
        let stream = conn.open_tcp("127.0.0.1", vnc_port).await?;
        let password = target.password.clone();
        let vnc = VncConnector::new(stream)
            .set_auth_method(async move { Ok(password) })
            .add_encoding(VncEncoding::Tight)
            .add_encoding(VncEncoding::Zrle)
            .add_encoding(VncEncoding::CopyRect)
            .add_encoding(VncEncoding::Raw)
            .allow_shared(true)
            .set_pixel_format(PixelFormat::rgba())
            .build()?
            .try_start()
            .await?
            .finish()?;
        println!("vnc handshake done after {:?}", started.elapsed());

        let mut screen = Screen { width: 0, height: 0, rgba: Vec::new() };
        let (mut updates, mut bytes, mut jpeg) = (0u32, 0usize, 0u32);
        let mut first_frame = None;
        let begin = Instant::now();
        let mut last_refresh = Instant::now();
        let mut sent_input = false;
        while begin.elapsed() < Duration::from_secs(6) {
            match tokio::time::timeout(Duration::from_millis(20), vnc.poll_event()).await {
                Ok(Ok(Some(event))) => match event {
                    VncEvent::SetResolution(s) => screen.resize(s.width as usize, s.height as usize),
                    VncEvent::RawImage(rect, data) => {
                        updates += 1;
                        bytes += data.len();
                        screen.draw(rect, &data);
                        first_frame.get_or_insert_with(|| started.elapsed());
                    }
                    VncEvent::Copy(dst, src) => screen.copy(dst, src),
                    VncEvent::JpegImage(..) => jpeg += 1,
                    _ => {}
                },
                Ok(Ok(None)) | Err(_) => {}
                Ok(Err(error)) => return Err(anyhow!("vnc: {error}")),
            }
            if last_refresh.elapsed() > Duration::from_millis(33) {
                vnc.input(X11Event::Refresh).await?;
                last_refresh = Instant::now();
            }
            // Harmless input: nudge the pointer near the centre and tap Shift.
            if !sent_input && begin.elapsed() > Duration::from_secs(2) && screen.width > 0 {
                let (x, y) = ((screen.width / 2) as u16, (screen.height / 2) as u16);
                vnc.input(X11Event::PointerEvent((x, y, 0).into())).await?;
                vnc.input(X11Event::PointerEvent((x + 40, y + 40, 0).into())).await?;
                vnc.input(X11Event::KeyEvent((0xffe1, true).into())).await?;
                vnc.input(X11Event::KeyEvent((0xffe1, false).into())).await?;
                sent_input = true;
                println!("input sent: pointer to ({}, {}), Shift tap", x + 40, y + 40);
            }
        }
        let _ = vnc.close().await;

        let seconds = begin.elapsed().as_secs_f64();
        println!(
            "screen {}x{}, first pixels after {:?}, {updates} raw rectangles ({:.1}/s), {:.1} MB decoded, {jpeg} JPEG rectangles ignored",
            screen.width,
            screen.height,
            first_frame.unwrap_or_default(),
            updates as f64 / seconds,
            bytes as f64 / 1e6,
        );
        let filled = screen.rgba.chunks_exact(4).filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0).count();
        println!("non-black pixels: {:.1} %", filled as f64 * 100.0 / (screen.width * screen.height).max(1) as f64);

        for pixel in screen.rgba.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        let file = std::io::BufWriter::new(std::fs::File::create(&out)?);
        let mut encoder = png::Encoder::new(file, screen.width as u32, screen.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&screen.rgba)?;
        println!("saved {out}");
        Ok(())
    }
    .await;

    // Always stop x11vnc, whatever happened above.
    let _ = target.child_writer.close().await;
    stop_x11vnc(&conn, vnc_port).await;
    let left = conn
        .exec(&[
            "sh",
            "-c",
            &format!("ss -ltnH | grep -c ':{vnc_port} ' || true"),
        ])
        .await;
    println!(
        "listeners left on the VNC port: {}",
        left.map(|o| o.stdout_str().trim().to_string())
            .unwrap_or_default()
    );
    drop(state);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", copy.display()));
    }
    result
}
