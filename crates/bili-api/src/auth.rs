//! QR code login through the bilibili passport.
//!
//! The flow is the same as the web login: a client generates a QR
//! session, the user scans it with the bilibili phone app, and the
//! poll endpoint reports the status. On success the response's
//! `Set-Cookie` headers carry the login session.

use serde::Deserialize;

/// QR code data from `GET /x/passport-login/web/qrcode/generate`.
#[derive(Debug, Clone, Deserialize)]
pub struct Qrcode {
    /// The URL the phone app opens for the scan login screen.
    /// Encode this string in a QR code.
    pub url: String,
    /// The key used to poll the login status.
    pub qrcode_key: String,
}

/// The status of a QR login attempt, from the poll `data.code` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QrcodeStatus {
    /// The user has not scanned yet (86101). The default status.
    #[default]
    Waiting,
    /// The user scanned; waiting for confirmation (86090).
    Scanned,
    /// Login succeeded (0).
    Success,
    /// The QR code expired (86038).
    Expired,
    /// A status code this client does not know.
    Unknown(i32),
}

impl QrcodeStatus {
    /// Map the poll `data.code` value to a status.
    pub fn from_code(code: i32) -> Self {
        match code {
            86101 => Self::Waiting,
            86090 => Self::Scanned,
            0 => Self::Success,
            86038 => Self::Expired,
            other => Self::Unknown(other),
        }
    }
}

/// The result of one QR login poll.
#[derive(Debug, Clone)]
pub struct QrcodePoll {
    /// The login status.
    pub status: QrcodeStatus,
    /// The cookies set by the response.
    ///
    /// On success they carry the login session: `SESSDATA`,
    /// `bili_jct`, `DedeUserID`, and friends.
    pub cookies: Vec<(String, String)>,
    /// The passport refresh token, when the poll gives one.
    pub refresh_token: Option<String>,
}
