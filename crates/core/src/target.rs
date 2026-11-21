//! Explicit Android action targets.
//!
//! Every Android operation must name a concrete host, route and host-local
//! ADB serial (docs/AGENTS.md rule 6, docs/API.md §5). An action is only
//! executed if the route observed *right now* still matches the target.

use crate::{CoreError, DeviceId, ErrorCode, HostId, RouteId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidTarget {
    pub host_id: HostId,
    pub device_id: DeviceId,
    pub route_id: RouteId,
    /// Host-local ADB serial / transport identifier expected on `host_id`.
    pub expected_serial: String,
}

/// What a host's ADB server reports for one serial at validation time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedAdbDevice {
    pub host_id: HostId,
    pub serial: String,
    /// Raw ADB state, e.g. `device`, `unauthorized`, `offline`.
    pub state: String,
}

impl AndroidTarget {
    /// Re-validate the target against the devices currently observed on the
    /// target's host. Never falls back to another host or serial.
    pub fn validate(&self, observed: &[ObservedAdbDevice]) -> Result<(), CoreError> {
        let hit = observed
            .iter()
            .find(|d| d.host_id == self.host_id && d.serial == self.expected_serial);
        match hit {
            None => Err(CoreError::new(
                ErrorCode::RouteStale,
                format!(
                    "route {} is no longer present on host {}",
                    self.route_id, self.host_id
                ),
            )),
            Some(d) if d.state == "unauthorized" => Err(CoreError::new(
                ErrorCode::AdbUnauthorized,
                "USB debugging not authorized on the device",
            )),
            Some(d) if d.state != "device" => Err(CoreError::new(
                ErrorCode::DeviceOffline,
                format!("device state is '{}'", d.state),
            )),
            Some(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> AndroidTarget {
        AndroidTarget {
            host_id: HostId::new("host-linux").unwrap(),
            device_id: DeviceId::new("android-b").unwrap(),
            route_id: RouteId::new("route-linux-usb-b").unwrap(),
            expected_serial: "SERIAL_B".into(),
        }
    }

    fn obs(host: &str, serial: &str, state: &str) -> ObservedAdbDevice {
        ObservedAdbDevice {
            host_id: HostId::new(host).unwrap(),
            serial: serial.into(),
            state: state.into(),
        }
    }

    #[test]
    fn ok_when_serial_present_on_same_host() {
        assert!(target()
            .validate(&[obs("host-linux", "SERIAL_B", "device")])
            .is_ok());
    }

    #[test]
    fn same_serial_on_other_host_is_stale_not_substituted() {
        let err = target()
            .validate(&[obs("host-windows", "SERIAL_B", "device")])
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::RouteStale);
    }

    #[test]
    fn unauthorized_and_offline_are_distinct() {
        let t = target();
        assert_eq!(
            t.validate(&[obs("host-linux", "SERIAL_B", "unauthorized")])
                .unwrap_err()
                .code,
            ErrorCode::AdbUnauthorized
        );
        assert_eq!(
            t.validate(&[obs("host-linux", "SERIAL_B", "offline")])
                .unwrap_err()
                .code,
            ErrorCode::DeviceOffline
        );
    }
}
