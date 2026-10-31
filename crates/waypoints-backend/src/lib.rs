//! Local backend lifecycle and workflow requests shared by Waypoints clients.

#[cfg(not(unix))]
compile_error!("The initial Waypoints backend requires Unix sockets (macOS or Linux)");

mod client;
mod config;
mod protocol;
mod server;

pub use client::Client;
pub use config::Config;
pub use protocol::BackendInfo;
pub use server::serve;

use std::error::Error;
use std::fmt;

/// A backend startup, transport, or request failure.
#[derive(Debug)]
pub struct BackendError {
    inner: anyhow::Error,
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
