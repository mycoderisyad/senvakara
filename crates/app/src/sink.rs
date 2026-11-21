//! Where a session's events go. A sink only queues the event for its viewer
//! (an IPC channel, an event-loop callback, a test buffer); it never calls
//! back into [`crate::App`].

use std::sync::Arc;

/// Returns `false` once the viewer is gone; the producing loop then stops.
pub type Sink<T> = Arc<dyn Fn(T) -> bool + Send + Sync>;
