//! Protocol types for the bilibili live APIs.
//!
//! Primitive newtypes keep same-shaped values apart. For example, a
//! `RoomID` and a `UID` are both unsigned integers, but they mean
//! different things and must not be swapped.

use std::fmt;

/// A bilibili live room identifier.
///
/// The value is the real room id returned by `Room/get_info`, or a
/// candidate (short id) that must be resolved first. The inner type is
/// `u64` because room ids are plain integers in every bilibili API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Deserialize, serde::Serialize)]
pub struct RoomID(pub u64);

impl RoomID {
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for RoomID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl std::str::FromStr for RoomID {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.trim()
            .parse::<u64>()
            .map(RoomID)
            .map_err(|_| format!("room id must be a number, got {s:?}"))
    }
}

/// A bilibili user (UP) identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Deserialize, serde::Serialize)]
pub struct UID(pub u64);

impl UID {
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for UID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// Live room state as returned by `Room/get_info`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize, Default)]
pub struct LiveStatus(pub u8);

impl From<u8> for LiveStatus {
    fn from(v: u8) -> Self {
        Self(v)
    }
}

impl LiveStatus {
    /// The room is streaming now.
    pub const LIVE: Self = Self(1);
    /// The room is not streaming.
    pub const OFFLINE: Self = Self(0);

    pub fn is_live(self) -> bool {
        self == Self::LIVE
    }
}

impl std::fmt::Display for LiveStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_live() {
            f.write_str("live")
        } else {
            f.write_str("offline")
        }
    }
}

/// Room metadata from `GET room/v1/Room/get_info`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RoomInfo {
    /// Real room id.
    pub room_id: RoomID,
    /// Short id used in room URLs, when present.
    #[serde(default)]
    pub short_id: u64,
    /// The UP that owns the room.
    pub uid: UID,
    #[serde(default)]
    pub live_status: LiveStatus,
    #[serde(default)]
    pub online: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub parent_area_name: String,
}

impl RoomInfo {
    /// A short display name for the room, for grid cells.
    pub fn display_name(&self) -> &str {
        if self.title.is_empty() {
            "unnamed room"
        } else {
            &self.title
        }
    }
}

/// A danmaku WebSocket server endpoint from `getDanmuInfo`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct DanmuServer {
    pub host: String,
    #[serde(default)]
    pub wss_port: u16,
}

/// Danmaku WebSocket server configuration for one room.
#[derive(Clone, Debug)]
pub struct DanmuServerConf {
    /// Candidate servers. The client round-robins them on reconnect.
    pub servers: Vec<DanmuServer>,
    /// Token to send in the WS auth packet. May be empty.
    pub token: String,
}

/// A playable stream URL from `Room/playUrl`.
#[derive(Clone, Debug)]
pub struct PlayUrl {
    /// The stream URL (HTTP-FLV or HLS, depending on what the API returns).
    pub url: String,
}

/// UP (user) display info.
#[derive(Clone, Debug)]
pub struct UpInfo {
    pub uid: UID,
    /// The UP display name, or the uid as a string when the lookup failed.
    pub uname: String,
}
