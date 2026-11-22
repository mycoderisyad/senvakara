//! The VNC protocol loop of one desktop session, run on the backend runtime.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use senvakara_app::VncTarget;
use slint::SharedPixelBuffer;
use tokio::sync::mpsc;
use vnc::{PixelFormat, VncConnector, VncEncoding, VncEvent, X11Event};

use super::framebuffer::Framebuffer;
use super::{Cmd, Phase, Shared, FRAME_GAP};

/// Runs the VNC session until the pane closes (Ok) or the connection ends (Err: why, in words).
pub(super) async fn serve(
    target: &VncTarget,
    shared: &Shared,
    mut commands: mpsc::UnboundedReceiver<Cmd>,
) -> Result<(), String> {
    let lost = |error: &dyn std::fmt::Display| {
        format!("The connection to the desktop was lost. ({error})")
    };
    // The session is over on the other side: x11vnc stopped or the SSH connection went away.
    let ended = |error: vnc::VncError| match error {
        vnc::VncError::ClientNotRunning => {
            "The desktop session ended: the host closed it or the SSH connection was lost."
                .to_string()
        }
        other => lost(&other),
    };
    let stream = target
        .conn
        .open_tcp("127.0.0.1", target.vnc_port)
        .await
        .map_err(|error| format!("The SSH tunnel to the desktop could not be opened. ({error})"))?;
    let password = target.password.clone();
    let start =
        |error: vnc::VncError| format!("The desktop did not accept the connection. ({error})");
    let vnc = VncConnector::new(stream)
        .set_auth_method(async move { Ok(password) })
        .add_encoding(VncEncoding::Tight)
        .add_encoding(VncEncoding::Zrle)
        .add_encoding(VncEncoding::CopyRect)
        .add_encoding(VncEncoding::Raw)
        .allow_shared(true)
        .set_pixel_format(PixelFormat::rgba())
        .build()
        .map_err(start)?
        .try_start()
        .await
        .map_err(start)?
        .finish()
        .map_err(start)?;
    shared.set_phase(Phase::Connected);

    let mut screen = Framebuffer::default();
    let mut changed = false;
    let mut asked = Instant::now() - FRAME_GAP;
    let mut copied = asked;
    let outcome = loop {
        if shared.closed.load(Ordering::Acquire) {
            break Ok(());
        }
        // Apply what the server sent, without letting a flood starve the input.
        let burst = Instant::now();
        let failed = loop {
            match vnc.poll_event().await {
                Ok(Some(event)) => match event {
                    VncEvent::SetResolution(size) => {
                        screen.resize(size.width as usize, size.height as usize);
                        changed = true;
                    }
                    VncEvent::RawImage(rect, data) => changed |= screen.draw(rect, &data),
                    VncEvent::Copy(dst, src) => changed |= screen.copy(dst, src),
                    VncEvent::Error(message) => break Some(lost(&message)),
                    _ => {}
                },
                Ok(None) => break None,
                Err(error) => break Some(ended(error)),
            }
            if burst.elapsed() > Duration::from_millis(12) {
                break None;
            }
        };
        if let Some(reason) = failed {
            break Err(reason);
        }

        let visible = shared.visible.load(Ordering::Acquire);
        if visible && asked.elapsed() >= FRAME_GAP {
            if let Err(error) = vnc.input(X11Event::Refresh).await {
                break Err(ended(error));
            }
            asked = Instant::now();
        }
        if visible && screen.width > 0 && copied.elapsed() >= FRAME_GAP {
            let wanted = shared.want_frame.load(Ordering::Acquire);
            let mut slot = shared.frame.lock().unwrap();
            // One copy waits at most: while the UI has not taken it, the screen stays here.
            if (changed || wanted) && slot.is_none() {
                *slot = Some(SharedPixelBuffer::clone_from_slice(
                    &screen.rgba,
                    screen.width as u32,
                    screen.height as u32,
                ));
                drop(slot);
                shared.want_frame.store(false, Ordering::Release);
                changed = false;
                copied = Instant::now();
                shared.mark();
            }
        }

        // Sleep until the user does something, but not for long: the server is polled.
        let nap = Duration::from_millis(if visible { 8 } else { 100 });
        let first = match tokio::time::timeout(nap, commands.recv()).await {
            Ok(Some(command)) => command,
            Ok(None) => break Ok(()),
            Err(_) => continue,
        };
        let mut batch = vec![first];
        while let Ok(command) = commands.try_recv() {
            // Of several pointer moves in a row only the last position matters.
            match (batch.last_mut(), command) {
                (
                    Some(Cmd::Pointer { x, y, mask }),
                    Cmd::Pointer {
                        x: new_x,
                        y: new_y,
                        mask: new_mask,
                    },
                ) if *mask == new_mask => (*x, *y) = (new_x, new_y),
                (_, command) => batch.push(command),
            }
        }
        let mut failed = None;
        for command in batch {
            let event = match command {
                Cmd::Pointer { x, y, mask } => X11Event::PointerEvent((x, y, mask).into()),
                Cmd::Key { sym, down } => X11Event::KeyEvent((sym, down).into()),
            };
            if let Err(error) = vnc.input(event).await {
                failed = Some(ended(error));
                break;
            }
        }
        if let Some(reason) = failed {
            break Err(reason);
        }
    };
    let _ = vnc.close().await;
    outcome
}
