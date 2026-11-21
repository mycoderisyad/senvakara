//! Android Wireless Debugging (AND-05, task AND-IMPL-07): pair with a
//! one-time code shown on the phone, connect, and list mDNS-advertised
//! devices. Uses the ADB server's own TLS pairing; Senvakara never stores
//! the pairing code (docs/SECURITY.md §3.6).

use crate::protocol;
use crate::AdbServer;
use anyhow::{bail, Result};
use senvakara_core::{CoreError, ErrorCode};
use serde::Serialize;
use tokio::io::AsyncReadExt;

fn core(code: ErrorCode, msg: impl Into<String>) -> anyhow::Error {
    CoreError::new(code, msg).into()
}

/// `host:port` where host is an IPv4 address, `[IPv6]` or a DNS-style name.
pub fn valid_address(addr: &str) -> bool {
    let Some((host, port)) = addr.rsplit_once(':') else {
        return false;
    };
    let port_ok = port.parse::<u16>().is_ok_and(|p| p > 0);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let host_ok = !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-:_".contains(c));
    port_ok && host_ok
}

/// Android shows a 6-digit pairing code.
pub fn valid_code(code: &str) -> bool {
    code.len() == 6 && code.chars().all(|c| c.is_ascii_digit())
}

/// Read the reply used by `host:pair` / `host:connect` after OKAY: either
/// `FAIL<len><msg>` or `<len><msg>`.
async fn read_reply(s: &mut tokio::net::TcpStream) -> Result<std::result::Result<String, String>> {
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    if &head == b"FAIL" {
        return Ok(Err(protocol::read_hex_block(s).await.unwrap_or_default()));
    }
    let n = usize::from_str_radix(std::str::from_utf8(&head)?, 16)?;
    let mut body = vec![0u8; n];
    s.read_exact(&mut body).await?;
    Ok(Ok(String::from_utf8_lossy(&body).into_owned()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MdnsService {
    pub name: String,
    /// `_adb-tls-pairing._tcp` (pairing screen open) or `_adb-tls-connect._tcp`.
    pub service: String,
    pub address: String,
}

pub fn parse_mdns(text: &str) -> Vec<MdnsService> {
    text.lines()
        .filter_map(|l| {
            let mut p = l.split('\t');
            let (name, service, address) = (p.next()?.trim(), p.next()?.trim(), p.next()?.trim());
            (!name.is_empty() && service.starts_with("_adb") && valid_address(address)).then(|| {
                MdnsService {
                    name: name.to_string(),
                    service: service.trim_end_matches('.').to_string(),
                    address: address.to_string(),
                }
            })
        })
        .collect()
}

impl AdbServer {
    /// Pair with a phone showing "Pair device with pairing code".
    pub async fn pair(&self, address: &str, code: &str) -> Result<String> {
        if !valid_address(address) {
            bail!(core(
                ErrorCode::InvalidInput,
                "address must look like 192.0.2.20:37123"
            ));
        }
        if !valid_code(code) {
            bail!(core(
                ErrorCode::InvalidInput,
                "pairing code must be the 6 digits shown on the phone"
            ));
        }
        let mut s = self.connect_raw().await?;
        protocol::request(&mut s, &format!("host:pair:{code}:{address}")).await?;
        match read_reply(&mut s).await? {
            Ok(msg) if msg.starts_with("Successfully paired") => Ok(msg),
            Ok(msg) | Err(msg) => Err(core(
                ErrorCode::Unauthorized,
                format!("pairing failed: {msg}"),
            )),
        }
    }

    /// Connect to a paired phone's Wireless debugging address.
    pub async fn connect_device(&self, address: &str) -> Result<String> {
        if !valid_address(address) {
            bail!(core(
                ErrorCode::InvalidInput,
                "address must look like 192.0.2.20:41234"
            ));
        }
        let mut s = self.connect_raw().await?;
        protocol::request(&mut s, &format!("host:connect:{address}")).await?;
        match read_reply(&mut s).await? {
            Ok(msg)
                if msg.starts_with("connected to") || msg.starts_with("already connected to") =>
            {
                Ok(msg)
            }
            Ok(msg) | Err(msg) => Err(core(ErrorCode::HostOffline, msg)),
        }
    }

    pub async fn disconnect_device(&self, address: &str) -> Result<String> {
        if !valid_address(address) {
            bail!(core(ErrorCode::InvalidInput, "invalid address"));
        }
        let mut s = self.connect_raw().await?;
        protocol::request(&mut s, &format!("host:disconnect:{address}")).await?;
        Ok(read_reply(&mut s).await?.unwrap_or_else(|e| e))
    }

    /// Devices advertising Wireless debugging on the server's network.
    pub async fn mdns_services(&self) -> Result<Vec<MdnsService>> {
        let mut s = self.connect_raw().await?;
        protocol::request(&mut s, "host:mdns:services").await?;
        Ok(parse_mdns(&protocol::read_hex_block(&mut s).await?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_inputs() {
        assert!(valid_address("192.168.1.20:37123"));
        assert!(valid_address("[fe80::1]:5555"));
        assert!(valid_address("adb-ABC._adb-tls-connect._tcp:5555"));
        for bad in [
            "192.168.1.20",
            "x:0",
            ":5555",
            "a b:1",
            "h:99999",
            "h:1;reboot",
        ] {
            assert!(!valid_address(bad), "{bad}");
        }
        assert!(valid_code("123456"));
        assert!(!valid_code("12345") && !valid_code("12345a") && !valid_code("1234567"));
    }

    #[test]
    fn parses_mdns_services() {
        let text = "adb-LNFQ8TY9-AbCdEf\t_adb-tls-connect._tcp.\t192.168.1.20:41234\n\
                    adb-LNFQ8TY9-AbCdEf\t_adb-tls-pairing._tcp.\t192.168.1.20:37123\n\
                    junk line\n";
        let s = parse_mdns(text);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].service, "_adb-tls-connect._tcp");
        assert_eq!(s[1].address, "192.168.1.20:37123");
    }
}
