//! Opaque identifiers. Constructed only from stored/generated ids, never from
//! IP addresses, ADB serials or display names (docs/ARCHITECTURE.md §3).

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Wrap an existing persisted id. Empty ids are rejected.
            pub fn new(id: impl Into<String>) -> Option<Self> {
                let id = id.into();
                if id.trim().is_empty() {
                    None
                } else {
                    Some(Self(id))
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

opaque_id!(HostId);
opaque_id!(DeviceId);
opaque_id!(RouteId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_ids_are_rejected() {
        assert!(HostId::new("").is_none());
        assert!(DeviceId::new("   ").is_none());
        assert_eq!(RouteId::new("route-1").unwrap().as_str(), "route-1");
    }
}
