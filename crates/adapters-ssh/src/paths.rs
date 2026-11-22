//! Route fallback across the addresses of one host (NET-03/NET-04).
//!
//! A host may be reachable at its LAN address and at one or more VPN
//! addresses. They are *paths*, not identities: every attempt uses the same
//! pinned host key, which russh checks before any credential is sent, so a
//! different machine answering at an old address is refused and the next
//! path is tried.

use crate::{ConnectParams, SshConnection};
use anyhow::Result;
use senvakara_core::{CoreError, ErrorCode};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// SSH handshake budget for a path while other paths are still untried.
pub const HANDSHAKE_WITH_FALLBACK: Duration = Duration::from_secs(5);

/// One address a host may be reached at, in preference order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// `lan` (the main SSH address) or `vpn`.
    pub transport: &'static str,
    pub address: String,
    pub port: u16,
}

/// A connection plus which candidate it went through and why more preferred
/// candidates were skipped.
pub struct Connected {
    pub conn: Arc<SshConnection>,
    pub index: usize,
    pub notes: Vec<String>,
}

/// Probe the candidates in `idxs` at once and return the most preferred one
/// that accepts a TCP connection — as soon as every more preferred one has
/// failed, without waiting for less preferred probes. Also returns the
/// candidates known to be unreachable by then. Bounded by `timeout`, so an
/// unreachable LAN address costs seconds, not a full SSH timeout.
pub async fn first_reachable(
    cands: &[Candidate],
    idxs: &[usize],
    timeout: Duration,
) -> (Option<usize>, Vec<usize>) {
    let mut status: HashMap<usize, bool> = HashMap::new();
    let mut set = tokio::task::JoinSet::new();
    for &i in idxs {
        let (addr, port) = (cands[i].address.clone(), cands[i].port);
        set.spawn(async move {
            let ok = tokio::time::timeout(
                timeout,
                tokio::net::TcpStream::connect((addr.as_str(), port)),
            )
            .await
            .is_ok_and(|r| r.is_ok());
            (i, ok)
        });
    }
    // The best index whose predecessors are all known-unreachable, if decided.
    let decide = |status: &HashMap<usize, bool>| {
        for &i in idxs {
            match status.get(&i) {
                Some(true) => return Some(Some(i)),
                Some(false) => continue,
                None => return None,
            }
        }
        Some(None)
    };
    while let Some(Ok((i, ok))) = set.join_next().await {
        status.insert(i, ok);
        if let Some(best) = decide(&status) {
            let down = idxs
                .iter()
                .copied()
                .filter(|i| status.get(i) == Some(&false))
                .collect();
            return (best, down);
        }
    }
    let down = idxs
        .iter()
        .copied()
        .filter(|i| status.get(i) != Some(&true))
        .collect();
    (
        idxs.iter().copied().find(|i| status.get(i) == Some(&true)),
        down,
    )
}

/// Connect through the most preferred working candidate. `base` supplies
/// user, sign-in method and host-key policy; its host/port are ignored.
///
/// On failure the error carries a [`CoreError`]: a host-key or login
/// problem wins over "unreachable", because it is the one the user must act
/// on.
pub async fn connect_any(
    cands: &[Candidate],
    base: &ConnectParams,
    probe_timeout: Duration,
) -> Result<Connected> {
    let mut remaining: Vec<usize> = (0..cands.len()).collect();
    let mut notes: Vec<String> = Vec::new();
    let mut key_err: Option<CoreError> = None;
    let mut last_err: Option<CoreError> = None;
    loop {
        let (best, down) = first_reachable(cands, &remaining, probe_timeout).await;
        for &d in &down {
            notes.push(format!(
                "{} {} unreachable",
                cands[d].transport, cands[d].address
            ));
        }
        remaining.retain(|i| !down.contains(i) && Some(*i) != best);
        let Some(i) = best else { break };
        let c = &cands[i];
        // A path that accepts TCP but never finishes the SSH handshake (for
        // example a stale route right after a network change) must not hold
        // up the remaining paths for the full timeout.
        let connect_timeout = if remaining.is_empty() {
            base.connect_timeout
        } else {
            base.connect_timeout.min(HANDSHAKE_WITH_FALLBACK)
        };
        let params = ConnectParams {
            host: c.address.clone(),
            port: c.port,
            connect_timeout,
            ..base.clone()
        };
        match SshConnection::connect(&params).await {
            Ok(conn) => {
                return Ok(Connected {
                    conn,
                    index: i,
                    notes,
                })
            }
            Err(e) => {
                let ce = e
                    .downcast_ref::<CoreError>()
                    .cloned()
                    .unwrap_or_else(|| CoreError::new(ErrorCode::HostOffline, format!("{e:#}")));
                notes.push(format!("{} {}: {}", c.transport, c.address, ce.message));
                if ce.code == ErrorCode::Unauthenticated {
                    key_err.get_or_insert(ce);
                } else {
                    last_err = Some(ce);
                }
            }
        }
    }
    let err = key_err.or(last_err).unwrap_or_else(|| {
        CoreError::new(
            ErrorCode::HostOffline,
            format!(
                "none of the host's addresses answered ({})",
                cands
                    .iter()
                    .map(|c| format!("{}:{}", c.address, c.port))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
    });
    Err(err.into())
}
