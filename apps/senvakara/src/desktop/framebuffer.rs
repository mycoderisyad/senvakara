//! The framebuffer one pane keeps of the host's screen.

/// The host's screen as RGBA, kept up to date from the rectangles the server sends.
#[derive(Default)]
pub struct Framebuffer {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl Framebuffer {
    pub fn resize(&mut self, width: usize, height: usize) {
        if (width, height) == (self.width, self.height) {
            return;
        }
        self.width = width;
        self.height = height;
        self.rgba.clear();
        // Opaque black until the server has painted it.
        self.rgba.resize(width * height * 4, 0);
        for pixel in self.rgba.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    }

    /// Paint `data` (RGBX rows) at `rect`. Returns false when it does not fit the screen.
    pub fn draw(&mut self, rect: vnc::Rect, data: &[u8]) -> bool {
        let (x, y, w, h) = (
            rect.x as usize,
            rect.y as usize,
            rect.width as usize,
            rect.height as usize,
        );
        if x + w > self.width || y + h > self.height || data.len() < w * h * 4 {
            return false;
        }
        for row in 0..h {
            let from = row * w * 4;
            let to = ((y + row) * self.width + x) * 4;
            let line = &mut self.rgba[to..to + w * 4];
            line.copy_from_slice(&data[from..from + w * 4]);
            // The fourth byte is padding on the wire; the picture is opaque.
            for pixel in line.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
        }
        true
    }

    /// Move the pixels of `src` to `dst` (same size), as CopyRect asks.
    pub fn copy(&mut self, dst: vnc::Rect, src: vnc::Rect) -> bool {
        let (w, h) = (src.width as usize, src.height as usize);
        let (sx, sy, dx, dy) = (
            src.x as usize,
            src.y as usize,
            dst.x as usize,
            dst.y as usize,
        );
        if sx + w > self.width
            || sy + h > self.height
            || dx + w > self.width
            || dy + h > self.height
        {
            return false;
        }
        // Rows are moved in the order that never overwrites a row still to be read.
        let rows: Box<dyn Iterator<Item = usize>> = if dy > sy {
            Box::new((0..h).rev())
        } else {
            Box::new(0..h)
        };
        for row in rows {
            let from = ((sy + row) * self.width + sx) * 4;
            let to = ((dy + row) * self.width + dx) * 4;
            self.rgba.copy_within(from..from + w * 4, to);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u16, y: u16, width: u16, height: u16) -> vnc::Rect {
        vnc::Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn rectangles_are_painted_opaque() {
        let mut screen = Framebuffer::default();
        screen.resize(4, 3);
        assert_eq!(screen.rgba.len(), 48);
        assert!(screen.rgba.chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
        // Two pixels with a zero padding byte, at (1, 1).
        assert!(screen.draw(rect(1, 1, 2, 1), &[10, 20, 30, 0, 40, 50, 60, 0]));
        let at = |screen: &Framebuffer, x: usize, y: usize| {
            let start = (y * screen.width + x) * 4;
            screen.rgba[start..start + 4].to_vec()
        };
        assert_eq!(at(&screen, 1, 1), [10, 20, 30, 255]);
        assert_eq!(at(&screen, 2, 1), [40, 50, 60, 255]);
        assert_eq!(at(&screen, 0, 1), [0, 0, 0, 255]);
        // Out of bounds or short data is refused, not painted.
        assert!(!screen.draw(rect(3, 0, 2, 1), &[0; 8]));
        assert!(!screen.draw(rect(0, 0, 2, 1), &[0; 4]));
        // CopyRect, overlapping downwards.
        assert!(screen.copy(rect(1, 2, 2, 1), rect(1, 1, 2, 1)));
        assert_eq!(at(&screen, 2, 2), [40, 50, 60, 255]);
        assert!(!screen.copy(rect(3, 2, 2, 1), rect(1, 1, 2, 1)));
        // The same size keeps the picture.
        screen.resize(4, 3);
        assert_eq!(at(&screen, 1, 1), [10, 20, 30, 255]);
    }
}
