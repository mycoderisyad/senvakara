//! Single-instance guard shared by every Senvakara front end. Two instances
//! on one database would overwrite each other's device states (the route
//! states are reset on start), so the second one must not start.

use crate::paths::{AppPaths, APP_ID};
use sha2::{Digest, Sha256};

/// Held for the life of the process; dropping it releases the lock.
#[derive(Debug)]
pub struct InstanceGuard {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

// SAFETY: the handle is only ever closed, once, by `Drop`.
unsafe impl Send for InstanceGuard {}
// SAFETY: no method reads or writes through the handle.
unsafe impl Sync for InstanceGuard {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceError {
    /// Another Senvakara process holds the lock.
    AlreadyRunning,
    /// The lock could not be created (Windows error code).
    Os(u32),
}

impl std::fmt::Display for InstanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning => write!(f, "another Senvakara instance is already running"),
            Self::Os(code) => write!(
                f,
                "could not create the single-instance lock (error {code})"
            ),
        }
    }
}

impl std::error::Error for InstanceError {}

/// Lock name for these paths: `Local\dev.senvakara.desktop` for the default
/// data location (shared by all front ends), and a name derived from the
/// folder when `SENVAKARA_DATA_DIR` points somewhere else.
pub fn instance_name(paths: &AppPaths) -> String {
    if !paths.overridden {
        return format!("Local\\{APP_ID}");
    }
    let key = paths.data_dir.to_string_lossy().to_lowercase();
    let hex: String = Sha256::digest(key.as_bytes())
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("Local\\{APP_ID}.{hex}")
}

/// Take the single-instance lock for `paths`. Keep the guard until exit.
pub fn single_instance(paths: &AppPaths) -> Result<InstanceGuard, InstanceError> {
    acquire(&instance_name(paths))
}

/// Take a named lock (a Windows named mutex in the session namespace).
#[cfg(windows)]
pub fn acquire(name: &str) -> Result<InstanceGuard, InstanceError> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is a valid NUL-terminated UTF-16 string that outlives the
    // call; a null security descriptor means the default one.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
    // SAFETY: reads the calling thread's last-error value set by the call above.
    let last = unsafe { GetLastError() };
    if handle.is_null() {
        return Err(InstanceError::Os(last));
    }
    if last == ERROR_ALREADY_EXISTS {
        // SAFETY: `handle` was just returned by CreateMutexW and is not used again.
        unsafe { CloseHandle(handle) };
        return Err(InstanceError::AlreadyRunning);
    }
    Ok(InstanceGuard { handle })
}

/// Other platforms have no lock yet; the app only ships for Windows.
#[cfg(not(windows))]
pub fn acquire(_name: &str) -> Result<InstanceGuard, InstanceError> {
    Ok(InstanceGuard {})
}

#[cfg(windows)]
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this guard and closed exactly once.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn second_holder_is_refused_until_the_first_lets_go() {
        let name = format!("Local\\senvakara-test-{}", uuid::Uuid::new_v4().simple());
        let first = acquire(&name).unwrap();
        assert_eq!(acquire(&name).unwrap_err(), InstanceError::AlreadyRunning);
        drop(first);
        assert!(acquire(&name).is_ok());
    }

    #[test]
    fn default_and_overridden_locations_use_different_locks() {
        let default = AppPaths {
            overridden: false,
            ..AppPaths::in_dir("X:/a")
        };
        assert_eq!(instance_name(&default), "Local\\dev.senvakara.desktop");
        let a = instance_name(&AppPaths::in_dir("X:/a"));
        assert_ne!(a, instance_name(&default));
        assert_ne!(a, instance_name(&AppPaths::in_dir("X:/b")));
        assert_eq!(a, instance_name(&AppPaths::in_dir("X:/A")));
    }
}
