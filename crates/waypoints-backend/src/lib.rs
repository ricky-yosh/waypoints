//! Local backend lifecycle and workflow requests shared by Waypoints clients.

#[cfg(not(unix))]
compile_error!("The initial Waypoints backend requires Unix sockets (macOS or Linux)");

mod client;
mod config;
mod protocol;
mod runs;
mod server;

pub use client::Client;
pub use config::Config;
pub use protocol::BackendInfo;
pub use runs::{Permission, PermissionOption, RunEvent, RunEventOrigin, RunEvents, RunStatus};
pub use server::serve;

use std::error::Error;
use std::fmt;

/// Exit status used by `waypoints serve` when an ownership lock is already held.
///
/// Auto-start clients may retry this temporary failure within their startup deadline.
pub const OWNERSHIP_UNAVAILABLE_EXIT_CODE: u8 = 75;

/// A backend startup, transport, or request failure.
#[derive(Debug)]
pub struct BackendError {
    inner: anyhow::Error,
}

impl BackendError {
    /// Whether startup was refused because another process holds an ownership lock.
    pub fn is_ownership_unavailable(&self) -> bool {
        matches!(
            self.inner.downcast_ref::<std::fs::TryLockError>(),
            Some(std::fs::TryLockError::WouldBlock)
        )
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:#}", self.inner)
    }
}

impl Error for BackendError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.inner.as_ref())
    }
}

impl From<anyhow::Error> for BackendError {
    fn from(inner: anyhow::Error) -> Self {
        Self { inner }
    }
}
