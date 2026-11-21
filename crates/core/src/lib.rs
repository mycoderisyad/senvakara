//! Senvakara core domain types.
//!
//! Host, device and route identities are deliberately distinct types
//! (see docs/ARCHITECTURE.md §3): a device is *what* we control, a route is
//! *how* we reach it from a particular host. None of them is derived from an
//! IP address, USB transport id or display name.

pub mod error;
pub mod ids;
pub mod target;

pub use error::{CoreError, ErrorCode};
pub use ids::{DeviceId, HostId, RouteId};
pub use target::{AndroidTarget, ObservedAdbDevice};
