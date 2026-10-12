//! Error types for the mpv audio controller.

use thiserror::Error;

/// Errors from the mpv audio controller.
#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to spawn mpv: {0}")]
    Spawn(std::io::Error),

    #[error("failed to connect to the mpv IPC socket: {0}")]
    Connect(std::io::Error),

    #[error("mpv IPC error: {0}")]
    Ipc(String),

    #[error("the mpv process exited unexpectedly: {0}")]
    ProcessGone(String),
}
