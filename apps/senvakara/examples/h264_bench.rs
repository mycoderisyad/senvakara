//! Spike UI-MIG-03 (offline part): how much CPU does software H.264 decoding cost?
//!
//! Encodes a synthetic phone-screen clip with OpenH264, then times decoding each frame and
//! converting it to RGBA, which is the work the mirror pane does per frame. Synthetic content
//! is only a proxy for a real scrcpy stream; repeat with a phone before closing the task.
//!
//! Run: cargo run --release -p senvakara --example h264_bench [width height frames]

use std::time::{Duration, Instant};

use openh264::decoder::Decoder;
use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate};
use openh264::formats::{RgbSliceU8, YUVBuffer};
use openh264::OpenH264API;

/// One frame of a scrolling list with a moving highlight: mostly flat areas with sharp text-like
/// edges, and the whole frame shifts every frame (the expensive case for a screen stream).
fn frame(width: usize, height: usize, index: usize) -> Vec<u8> {
    let mut rgb = vec![0u8; width * height * 3];
    let scroll = index * 12;
    for y in 0..height {
        let row = (y + scroll) / 96;
        let in_row = (y + scroll) % 96;
        for x in 0..width {
            let mut pixel = if row.is_multiple_of(2) {
                [24u8, 28, 34]
            } else {
                [30, 35, 43]
            };
            if in_row < 2 {
                pixel = [52, 59, 71];
            }
            // "Text": short dark-on-light bars whose pattern depends on the row.
            let glyph = !(x / 6 + row * 7).is_multiple_of(5) && (x / 6 + row) % 11 < 8;
            if (28..44).contains(&in_row) && x > 96 && x < width - 64 && glyph && (x % 6) < 4 {
                pixel = [228, 231, 236];
            }
            // An avatar-like coloured block per row.
            if (20..76).contains(&in_row) && (24..80).contains(&x) {
                let noise = ((x * 31 + (y + scroll) * 17 + row * 13) % 64) as u8;
                pixel = [
                    (row * 53 % 160) as u8 + 30 + noise,
                    (row * 97 % 160) as u8 + 30 + noise,
                    180,
                ];
            }
            let at = (y * width + x) * 3;
            rgb[at..at + 3].copy_from_slice(&pixel);
        }
    }
    rgb
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn main() -> anyhow::Result<()> {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let width = args.first().copied().unwrap_or(720);
    let height = args.get(1).copied().unwrap_or(1280);
    let frames = args.get(2).copied().unwrap_or(150);

    // scrcpy is started with video_bit_rate=4000000 and max_fps=30 (adapters-scrcpy).
    let config = EncoderConfig::new()
        .bitrate(BitRate::from_bps(4_000_000))
        .max_frame_rate(FrameRate::from_hz(30.0))
        .skip_frames(false);
    let mut encoder = Encoder::with_api_config(OpenH264API::from_source(), config)?;
    let mut packets = Vec::with_capacity(frames);
    let mut bytes = 0usize;
    for index in 0..frames {
        let rgb = frame(width, height, index);
        let yuv = YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb, (width, height)));
        let packet = encoder.encode(&yuv)?.to_vec();
        bytes += packet.len();
        packets.push(packet);
    }

    let mut decoder = Decoder::new()?;
    let mut rgba = vec![0u8; width * height * 4];
    let mut decode_times = Vec::with_capacity(frames);
    let mut convert_times = Vec::with_capacity(frames);
    for packet in &packets {
        let start = Instant::now();
        let decoded = decoder.decode(packet)?;
        let decoded_at = Instant::now();
        if let Some(yuv) = decoded {
            yuv.write_rgba8(&mut rgba);
            decode_times.push(decoded_at - start);
            convert_times.push(decoded_at.elapsed());
        }
    }

    let total: Duration = decode_times.iter().chain(&convert_times).sum();
    let count = decode_times.len();
    let per_frame = total / count as u32;
    decode_times.sort();
    convert_times.sort();
    println!(
        "clip: {width}x{height}, {count} frames decoded of {frames}, {:.0} kbit/s at 30 fps",
        bytes as f64 * 8.0 / frames as f64 * 30.0 / 1000.0
    );
    println!(
        "decode  p50 {:?}  p95 {:?}\nto RGBA p50 {:?}  p95 {:?}",
        percentile(&decode_times, 0.5),
        percentile(&decode_times, 0.95),
        percentile(&convert_times, 0.5),
        percentile(&convert_times, 0.95),
    );
    println!(
        "mean per frame {per_frame:?} -> {:.1} % of one core at 30 fps for one mirror",
        per_frame.as_secs_f64() * 30.0 * 100.0
    );
    Ok(())
}
