//! Control message serialization (big-endian), mirroring scrcpy v3.3.3
//! `ControlMessageReader.java`.

use anyhow::{bail, Result};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::OwnedWriteHalf;

pub const TYPE_INJECT_KEYCODE: u8 = 0;
pub const TYPE_INJECT_TEXT: u8 = 1;
pub const TYPE_INJECT_TOUCH_EVENT: u8 = 2;
pub const TYPE_INJECT_SCROLL_EVENT: u8 = 3;
pub const TYPE_BACK_OR_SCREEN_ON: u8 = 4;
pub const TYPE_EXPAND_NOTIFICATION_PANEL: u8 = 5;
pub const TYPE_COLLAPSE_PANELS: u8 = 7;
pub const TYPE_ROTATE_DEVICE: u8 = 11;

/// `INJECT_TEXT_MAX_LENGTH` on the server.
pub const TEXT_MAX: usize = 300;

/// scrcpy's pointer id for a generic finger (`POINTER_ID_GENERIC_FINGER` = -2).
pub const POINTER_GENERIC_FINGER: u64 = u64::MAX - 1;

/// Android `MotionEvent` / `KeyEvent` action codes used here.
pub const ACTION_DOWN: u8 = 0;
pub const ACTION_UP: u8 = 1;
pub const ACTION_MOVE: u8 = 2;

/// Input requested by the UI. Coordinates are in video-frame pixels and
/// `width`/`height` must be the current frame size, otherwise the server
/// ignores the event.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Input {
    Touch {
        action: u8,
        pointer_id: u64,
        x: i32,
        y: i32,
        width: u16,
        height: u16,
        pressure: f32,
    },
    Scroll {
        x: i32,
        y: i32,
        width: u16,
        height: u16,
        h: f32,
        v: f32,
    },
    Key {
        action: u8,
        keycode: i32,
        meta_state: i32,
    },
    Text {
        text: String,
    },
    BackOrScreenOn {
        action: u8,
    },
    ExpandNotifications,
    CollapsePanels,
    Rotate,
}

fn u16_fixed(v: f32) -> u16 {
    // [0, 1] → u16, 1.0 maps to 0xffff (scrcpy `Binary.floatToU16FixedPoint`).
    let v = v.clamp(0.0, 1.0);
    if v >= 1.0 {
        0xffff
    } else {
        (v * 65536.0) as u16
    }
}

fn i16_fixed(v: f32) -> i16 {
    // [-1, 1] → i16 (scrcpy `Binary.floatToI16FixedPoint`).
    let v = v.clamp(-1.0, 1.0);
    if v >= 1.0 {
        0x7fff
    } else {
        (v * 32768.0) as i16
    }
}

/// Serialize one message. Rejects values the server would refuse.
pub fn encode(input: &Input) -> Result<Vec<u8>> {
    let mut b = Vec::with_capacity(32);
    match input {
        Input::Touch {
            action,
            pointer_id,
            x,
            y,
            width,
            height,
            pressure,
        } => {
            if ![ACTION_DOWN, ACTION_UP, ACTION_MOVE].contains(action) {
                bail!("unsupported touch action");
            }
            b.push(TYPE_INJECT_TOUCH_EVENT);
            b.push(*action);
            b.extend_from_slice(&pointer_id.to_be_bytes());
            b.extend_from_slice(&x.to_be_bytes());
            b.extend_from_slice(&y.to_be_bytes());
            b.extend_from_slice(&width.to_be_bytes());
            b.extend_from_slice(&height.to_be_bytes());
            b.extend_from_slice(&u16_fixed(*pressure).to_be_bytes());
            // Injected as a finger (no mouse buttons): actionButton = buttons = 0.
            b.extend_from_slice(&0i32.to_be_bytes());
            b.extend_from_slice(&0i32.to_be_bytes());
        }
        Input::Scroll {
            x,
            y,
            width,
            height,
            h,
            v,
        } => {
            b.push(TYPE_INJECT_SCROLL_EVENT);
            b.extend_from_slice(&x.to_be_bytes());
            b.extend_from_slice(&y.to_be_bytes());
            b.extend_from_slice(&width.to_be_bytes());
            b.extend_from_slice(&height.to_be_bytes());
            // Server multiplies by 16, so the wire carries value / 16.
            b.extend_from_slice(&i16_fixed(h / 16.0).to_be_bytes());
            b.extend_from_slice(&i16_fixed(v / 16.0).to_be_bytes());
            b.extend_from_slice(&0i32.to_be_bytes());
        }
        Input::Key {
            action,
            keycode,
            meta_state,
        } => {
            if ![ACTION_DOWN, ACTION_UP].contains(action) || !(0..=400).contains(keycode) {
                bail!("unsupported key event");
            }
            b.push(TYPE_INJECT_KEYCODE);
            b.push(*action);
            b.extend_from_slice(&keycode.to_be_bytes());
            b.extend_from_slice(&0i32.to_be_bytes()); // repeat
            b.extend_from_slice(&meta_state.to_be_bytes());
        }
        Input::Text { text } => {
            let bytes = text.as_bytes();
            if bytes.is_empty() || bytes.len() > TEXT_MAX {
                bail!("text must be 1-{TEXT_MAX} bytes");
            }
            b.push(TYPE_INJECT_TEXT);
            b.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            b.extend_from_slice(bytes);
        }
        Input::BackOrScreenOn { action } => {
            b.push(TYPE_BACK_OR_SCREEN_ON);
            b.push(*action);
        }
        Input::ExpandNotifications => b.push(TYPE_EXPAND_NOTIFICATION_PANEL),
        Input::CollapsePanels => b.push(TYPE_COLLAPSE_PANELS),
        Input::Rotate => b.push(TYPE_ROTATE_DEVICE),
    }
    Ok(b)
}

pub struct ControlWriter {
    tx: OwnedWriteHalf,
}

impl ControlWriter {
    pub fn new(tx: OwnedWriteHalf) -> Self {
        Self { tx }
    }

    pub async fn send(&mut self, input: &Input) -> Result<()> {
        let bytes = encode(input)?;
        self.tx.write_all(&bytes).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_layout_is_32_bytes() {
        let b = encode(&Input::Touch {
            action: ACTION_DOWN,
            pointer_id: u64::MAX - 1,
            x: 100,
            y: 200,
            width: 720,
            height: 1280,
            pressure: 1.0,
        })
        .unwrap();
        assert_eq!(b.len(), 32);
        assert_eq!(b[0], TYPE_INJECT_TOUCH_EVENT);
        assert_eq!(b[1], ACTION_DOWN);
        assert_eq!(&b[10..14], &100i32.to_be_bytes());
        assert_eq!(&b[14..18], &200i32.to_be_bytes());
        assert_eq!(&b[18..20], &720u16.to_be_bytes());
        assert_eq!(&b[22..24], &[0xff, 0xff]);
    }

    #[test]
    fn key_and_text_layouts() {
        let k = encode(&Input::Key {
            action: ACTION_UP,
            keycode: 4,
            meta_state: 0,
        })
        .unwrap();
        assert_eq!(k, [0, 1, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0]);
        let t = encode(&Input::Text { text: "hi".into() }).unwrap();
        assert_eq!(t, [1, 0, 0, 0, 2, b'h', b'i']);
        assert!(encode(&Input::Text {
            text: "x".repeat(301)
        })
        .is_err());
        assert!(encode(&Input::Key {
            action: 9,
            keycode: 4,
            meta_state: 0
        })
        .is_err());
    }

    #[test]
    fn scroll_and_simple_messages() {
        let s = encode(&Input::Scroll {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
            h: 0.0,
            v: -1.0,
        })
        .unwrap();
        assert_eq!(s.len(), 1 + 4 + 4 + 2 + 2 + 2 + 2 + 4);
        assert_eq!(
            encode(&Input::BackOrScreenOn {
                action: ACTION_DOWN
            })
            .unwrap(),
            [4, 0]
        );
        assert_eq!(encode(&Input::Rotate).unwrap(), [11]);
    }
}
