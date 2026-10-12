//! Error types for the bilibili API client.

use thiserror::Error;

/// Errors from the API client.
#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP request failed: {0}")]
    Http(#[source] reqwest::Error),

    #[error("bilibili API returned code {code}: {msg}")]
    Api { code: i64, msg: String },

    #[error("invalid room reference: {0}")]
    BadRoomRef(String),

    #[error("WBI signing failed: {0}")]
    Wbi(String),

    #[error("API response is missing the field {0}")]
    MissingField(&'static str),

    #[error("API response has an unexpected shape: {0}")]
    UnexpectedResponse(String),
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}
