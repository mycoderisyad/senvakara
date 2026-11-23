//! The newest picture decoded while the pane was hidden, kept as video planes.
//!
//! The decoder must keep decoding hidden frames, because each one depends on the one before,
//! and the scrcpy adapter has no way to ask the phone for a new key frame. Turning a picture
//! into RGBA and uploading it is the costly part, so a hidden pane copies the planes instead
//! and converts the newest copy once, when the pane comes back on screen.

use openh264::formats::YUVSource;
use slint::{Rgba8Pixel, SharedPixelBuffer};

// Limited-range BT.601, the same constants OpenH264's own RGBA conversion uses, so a picture
// looks the same whichever way it was converted.
const Y_MUL: f32 = 255.0 / 219.0;
const RV_MUL: f32 = 255.0 / 224.0 * 1.402;
const GV_MUL: f32 = -255.0 / 224.0 * 1.402 * 0.299 / 0.587;
const GU_MUL: f32 = -255.0 / 224.0 * 1.772 * 0.114 / 0.587;
const BU_MUL: f32 = 255.0 / 224.0 * 1.772;

/// One I420 picture: the planes with their strides.
#[derive(Default)]
pub struct Held {
    width: usize,
    height: usize,
    strides: (usize, usize, usize),
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl Held {
    /// Keep `picture` in `slot`, reusing the memory of the picture that was there.
    pub fn keep(slot: &mut Option<Held>, picture: &impl YUVSource) {
        let held = slot.get_or_insert_with(Held::default);
        (held.width, held.height) = picture.dimensions();
        held.strides = picture.strides();
        for (plane, source) in [
            (&mut held.y, picture.y()),
            (&mut held.u, picture.u()),
            (&mut held.v, picture.v()),
        ] {
            plane.clear();
            plane.extend_from_slice(source);
        }
    }

    /// The picture as RGBA.
    pub fn to_rgba(&self) -> SharedPixelBuffer<Rgba8Pixel> {
        let mut buffer =
            SharedPixelBuffer::<Rgba8Pixel>::new(self.width as u32, self.height as u32);
        let out = buffer.make_mut_bytes();
        for row in 0..self.height {
            for col in 0..self.width {
                let y = Y_MUL * (f32::from(self.y[row * self.strides.0 + col]) - 16.0);
                let u = f32::from(self.u[(row / 2) * self.strides.1 + col / 2]) - 128.0;
                let v = f32::from(self.v[(row / 2) * self.strides.2 + col / 2]) - 128.0;
                let pixel = &mut out[(row * self.width + col) * 4..][..4];
                pixel[0] = channel(RV_MUL.mul_add(v, y));
                pixel[1] = channel(GV_MUL.mul_add(v, GU_MUL.mul_add(u, y)));
                pixel[2] = channel(BU_MUL.mul_add(u, y));
                pixel[3] = 255;
            }
        }
        buffer
    }
}

fn channel(value: f32) -> u8 {
    value.clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Planes {
        y: Vec<u8>,
        u: Vec<u8>,
        v: Vec<u8>,
    }

    impl YUVSource for Planes {
        fn dimensions(&self) -> (usize, usize) {
            (2, 2)
        }
        fn strides(&self) -> (usize, usize, usize) {
            (2, 1, 1)
        }
        fn y(&self) -> &[u8] {
            &self.y
        }
        fn u(&self) -> &[u8] {
            &self.u
        }
        fn v(&self) -> &[u8] {
            &self.v
        }
    }

    fn pixels(held: &Held) -> Vec<u8> {
        held.to_rgba().as_bytes().to_vec()
    }

    #[test]
    fn a_kept_picture_converts_to_the_colours_it_was_decoded_with() {
        let mut slot = None;
        // Video black and video white, over neutral colour.
        let picture = Planes {
            y: vec![16, 235, 235, 16],
            u: vec![128],
            v: vec![128],
        };
        Held::keep(&mut slot, &picture);
        let rgba = pixels(slot.as_ref().unwrap());
        assert_eq!(
            rgba,
            [0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255]
        );
    }

    #[test]
    fn keeping_a_newer_picture_replaces_the_older_one() {
        let mut slot = None;
        let dark = Planes {
            y: vec![16; 4],
            u: vec![128],
            v: vec![128],
        };
        let bright = Planes {
            y: vec![235; 4],
            u: vec![128],
            v: vec![128],
        };
        Held::keep(&mut slot, &dark);
        Held::keep(&mut slot, &bright);
        assert!(pixels(slot.as_ref().unwrap())
            .chunks(4)
            .all(|pixel| pixel == [255, 255, 255, 255]));
    }
}
