//! ADB host wire format: a request is `<4 hex digits length><payload>`; the
//! server answers `OKAY` or `FAIL<4 hex length><message>`. List-style replies
//! are a `<4 hex length><text>` block.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Encode a host request with its 4-hex-digit length prefix.
pub fn encode(payload: &str) -> Result<Vec<u8>> {
    if payload.len() > 0xffff {
        bail!("adb request too long");
    }
    Ok(format!("{:04x}{payload}", payload.len()).into_bytes())
}

/// Send a request and consume the OKAY/FAIL status.
pub async fn request<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S, payload: &str) -> Result<()> {
    s.write_all(&encode(payload)?).await?;
    let mut status = [0u8; 4];
    s.read_exact(&mut status).await.context("read adb status")?;
    match &status {
        b"OKAY" => Ok(()),
        b"FAIL" => {
            let msg = read_hex_block(s).await.unwrap_or_default();
            bail!("adb: {msg}")
        }
        other => bail!("unexpected adb status {:?}", String::from_utf8_lossy(other)),
    }
}

/// Read a `<4 hex length><bytes>` block as text.
pub async fn read_hex_block<S: AsyncRead + Unpin>(s: &mut S) -> Result<String> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).await.context("adb stream closed")?;
    let n =
        usize::from_str_radix(std::str::from_utf8(&len)?, 16).context("bad adb length prefix")?;
    let mut body = vec![0u8; n];
    s.read_exact(&mut body).await?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// One line of `devices -l` output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdbDevice {
    pub serial: String,
    /// `device`, `unauthorized`, `offline`, `recovery`, ...
    pub state: String,
    pub transport_id: Option<u64>,
    pub model: Option<String>,
    pub product: Option<String>,
    pub device: Option<String>,
    /// USB port path, e.g. `1-1.1`; absent for network transports.
    pub usb: Option<String>,
}

impl AdbDevice {
    /// Wireless/TCP transports have `host:port` style serials.
    pub fn is_network(&self) -> bool {
        self.usb.is_none()
            && (self.serial.contains(':') || self.serial.contains("._adb-tls-connect."))
    }
}

/// Parse `devices -l` text. Unknown fields are ignored; `model` keeps ADB's
/// underscore form (`Redmi_5A`) converted to spaces for display.
pub fn parse_devices_l(text: &str) -> Vec<AdbDevice> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let serial = parts.next()?.to_string();
            if serial == "List" || serial.starts_with('*') {
                return None;
            }
            let state = parts.next()?.to_string();
            let mut d = AdbDevice {
                serial,
                state,
                transport_id: None,
                model: None,
                product: None,
                device: None,
                usb: None,
            };
            for kv in parts {
                if let Some((k, v)) = kv.split_once(':') {
                    match k {
                        "transport_id" => d.transport_id = v.parse().ok(),
                        "model" => d.model = Some(v.replace('_', " ")),
                        "product" => d.product = Some(v.to_string()),
                        "device" => d.device = Some(v.to_string()),
                        "usb" => d.usb = Some(v.to_string()),
                        _ => {}
                    }
                }
            }
            Some(d)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_length_prefix() {
        assert_eq!(encode("host:version").unwrap(), b"000chost:version");
    }

    #[test]
    fn parses_devices_l() {
        let text = "LNFQ8TY9VS9HPJEI       device product:fleur_p_id model:2201117PG device:fleur transport_id:1\n\
                    43bbf22b7d94           device usb:1-1.1 product:rova model:Redmi_5A device:riva transport_id:3\n\
                    0123456789             unauthorized usb:2-1 transport_id:4\n\
                    192.168.1.50:5555      device product:x model:Pixel_7 device:panther transport_id:7\n";
        let d = parse_devices_l(text);
        assert_eq!(d.len(), 4);
        assert_eq!(d[0].serial, "LNFQ8TY9VS9HPJEI");
        assert_eq!(d[0].transport_id, Some(1));
        assert_eq!(d[1].model.as_deref(), Some("Redmi 5A"));
        assert_eq!(d[1].usb.as_deref(), Some("1-1.1"));
        assert_eq!(d[2].state, "unauthorized");
        assert_eq!(d[2].model, None);
        assert!(d[3].is_network());
        assert!(!d[1].is_network());
    }

    #[test]
    fn ignores_headers_and_blank_lines() {
        assert!(parse_devices_l("List of devices attached\n\n* daemon started\n").is_empty());
    }

    #[tokio::test]
    async fn request_reports_fail_message() {
        let (mut client, mut server) = tokio::io::duplex(64);
        tokio::spawn(async move {
            let mut buf = [0u8; 64];
            let _ = server.read(&mut buf).await;
            server.write_all(b"FAIL0010device not found").await.unwrap();
        });
        let err = request(&mut client, "host:transport-id:99")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("device not found"));
    }
}
