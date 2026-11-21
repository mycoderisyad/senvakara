//! Video packet framing: a 12-byte header (u64 BE PTS whose two top bits are
//! the config and key-frame flags, then u32 BE size) before each packet.
//! Config packets (SPS/PPS) are merged into the next packet so every key
//! frame is self-contained Annex-B, as WebCodecs needs without `description`.

use anyhow::{bail, Result};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

pub const CODEC_H264: u32 = 0x6832_3634; // "h264"
const FLAG_CONFIG: u64 = 1 << 63;
const FLAG_KEY: u64 = 1 << 62;
const PTS_MASK: u64 = FLAG_KEY - 1;
const MAX_PACKET: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub key: bool,
    /// Microseconds.
    pub pts: u64,
    /// Annex-B data (config prepended to the first packet after it).
    pub data: Vec<u8>,
    /// WebCodecs codec string when this packet carries new SPS (e.g. after rotation).
    pub codec: Option<String>,
}

pub struct VideoReader {
    stream: TcpStream,
    pending_config: Option<Vec<u8>>,
}

impl VideoReader {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            pending_config: None,
        }
    }

    /// Next decodable packet. `Err` when the stream ends.
    pub async fn next(&mut self) -> Result<Packet> {
        loop {
            let header = self.stream.read_u64().await?;
            let size = self.stream.read_u32().await? as usize;
            if size > MAX_PACKET {
                bail!("video packet too large ({size} bytes)");
            }
            let mut data = vec![0u8; size];
            self.stream.read_exact(&mut data).await?;
            if header & FLAG_CONFIG != 0 {
                self.pending_config = Some(data);
                continue;
            }
            let mut codec = None;
            if let Some(mut cfg) = self.pending_config.take() {
                codec = h264_codec_string(&cfg);
                cfg.extend_from_slice(&data);
                data = cfg;
            }
            return Ok(Packet {
                key: header & FLAG_KEY != 0,
                pts: header & PTS_MASK,
                data,
                codec,
            });
        }
    }
}

/// Build `avc1.PPCCLL` from the first SPS NAL unit in Annex-B data.
pub fn h264_codec_string(annexb: &[u8]) -> Option<String> {
    let mut i = 0;
    while i + 4 < annexb.len() {
        let start = if annexb[i..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if annexb[i..].starts_with(&[0, 0, 1]) {
            3
        } else {
            i += 1;
            continue;
        };
        let nal = i + start;
        if nal + 3 < annexb.len() && annexb[nal] & 0x1f == 7 {
            return Some(format!(
                "avc1.{:02X}{:02X}{:02X}",
                annexb[nal + 1],
                annexb[nal + 2],
                annexb[nal + 3]
            ));
        }
        i = nal;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_string_from_sps() {
        let data = [
            0, 0, 0, 1, 0x67, 0x42, 0xC0, 0x1F, 0xAA, 0, 0, 0, 1, 0x68, 0xCE,
        ];
        assert_eq!(h264_codec_string(&data).as_deref(), Some("avc1.42C01F"));
        assert_eq!(h264_codec_string(&[0, 0, 1, 0x68, 1, 2, 3]), None);
    }

    #[tokio::test]
    async fn merges_config_into_next_packet() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let (mut s, _) = listener.accept().await.unwrap();
            let cfg = [0u8, 0, 0, 1, 0x67, 0x64, 0x00, 0x28];
            s.write_u64(FLAG_CONFIG).await.unwrap();
            s.write_u32(cfg.len() as u32).await.unwrap();
            s.write_all(&cfg).await.unwrap();
            s.write_u64(FLAG_KEY | 1234).await.unwrap();
            s.write_u32(3).await.unwrap();
            s.write_all(&[9, 9, 9]).await.unwrap();
        });
        let mut r = VideoReader::new(TcpStream::connect(addr).await.unwrap());
        let p = r.next().await.unwrap();
        assert!(p.key);
        assert_eq!(p.pts, 1234);
        assert_eq!(p.codec.as_deref(), Some("avc1.640028"));
        assert_eq!(&p.data[p.data.len() - 3..], &[9, 9, 9]);
        assert_eq!(p.data.len(), 8 + 3);
    }
}
