//! What a run learns about and changes on a host besides running scripts:
//! the system it runs (modules refuse other systems, ADR-029) and, after a
//! guarded SSH port change, the port Hosts reaches it on.

use super::remote::HostRun;
use super::RunCtx;
use crate::state::CmdResult;
use senvakara_core::{CoreError, ErrorCode};
use senvakara_workflow::{parse_os_release, CheckedNode, Guard, Module, ParamValue};
use std::time::Duration;

fn err(code: ErrorCode, message: impl Into<String>) -> CoreError {
    CoreError::new(code, message)
}

/// The SSH port a guarded module moves to, when it differs from the port in
/// use.
pub(super) fn moved_port(node: &CheckedNode, guard: Option<&Guard>, current: u16) -> Option<u16> {
    let name = guard?.port_param.as_ref()?;
    match node.params.get(name) {
        Some(ParamValue::Int(n)) => u16::try_from(*n).ok().filter(|p| *p != current),
        _ => None,
    }
}

/// sshd moved and Senvakara signed in on the new port: Hosts follows (the
/// run's confirmation named the change), and later steps see the new port.
pub(super) fn follow_port(ctx: &RunCtx, host_id: &str, host: &mut HostRun, port: u16) {
    let from = host.session.port;
    match ctx
        .app
        .state
        .db
        .lock()
        .unwrap()
        .move_ssh_port(host_id, from, port)
    {
        Ok(_) => ctx.log(format!(
            "Hosts now reaches {} on port {port} (was {from}).",
            host.name
        )),
        Err(e) => ctx.log(format!(
            "SSH on {} moved to port {port}, but Hosts could not be updated ({e}); change the port in Hosts.",
            host.name
        )),
    }
    host.session.port = port;
}

/// Refuses a module on a system it was not written for (ADR-029).
pub(super) async fn check_os(host: &mut HostRun, module: &Module) -> CmdResult<()> {
    if module.manifest.os.is_empty() {
        return Ok(());
    }
    if host.os.is_none() {
        let out = tokio::time::timeout(
            Duration::from_secs(20),
            host.conn.exec(&["cat", "/etc/os-release"]),
        )
        .await
        .map_err(|_| {
            err(
                ErrorCode::Timeout,
                format!("{} did not say which system it runs", host.name),
            )
        })?
        .map_err(|e| crate::state::to_core("read /etc/os-release", e))?;
        host.os = parse_os_release(&out.stdout_str());
    }
    let Some((id, version)) = &host.os else {
        return Err(err(
            ErrorCode::UnsupportedCapability,
            format!(
                "{} has no readable /etc/os-release, so it is not known to be a supported system",
                host.name
            ),
        ));
    };
    if module.manifest.supports_os(id, version) {
        Ok(())
    } else {
        Err(err(
            ErrorCode::UnsupportedCapability,
            format!(
                "{} is written for {}; {} runs {id} {version}",
                module.manifest.name,
                module.manifest.os_text(),
                host.name
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use senvakara_workflow::Binding;

    #[test]
    fn only_a_port_that_differs_from_the_one_in_use_counts_as_moved() {
        let mut node = CheckedNode {
            id: "n1".into(),
            module: "security.ssh_hardening".into(),
            target: Binding::Host {
                host_id: "h".into(),
            },
            params: Default::default(),
            needs_root: true,
        };
        let guard = Guard {
            rollback_secs: 180,
            port_param: Some("port".into()),
        };
        assert_eq!(moved_port(&node, Some(&guard), 22), None);
        node.params.insert("port".into(), ParamValue::Int(2222));
        assert_eq!(moved_port(&node, Some(&guard), 22), Some(2222));
        assert_eq!(moved_port(&node, Some(&guard), 2222), None);
        assert_eq!(moved_port(&node, None, 22), None);
        let firewall = Guard {
            rollback_secs: 180,
            port_param: None,
        };
        assert_eq!(moved_port(&node, Some(&firewall), 22), None);
    }
}
