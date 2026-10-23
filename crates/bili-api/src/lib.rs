//! In-house bilibili HTTP API client.
//!
//! Covers the live-room endpoints that `bdr` needs: room info,
//! play URLs, danmaku server config (WBI signed), UP lookup, QR
//! login, and the live home (followed + recommended rooms).

mod auth;
mod client;
mod error;
mod types;
mod url;
mod wbi;

pub use auth::{Qrcode, QrcodePoll, QrcodeStatus};
pub use client::{Client, ClientConfig, Quality};
pub use error::Error;
pub use types::{
    DanmuServer, DanmuServerConf, LiveHome, LiveRoomEntry, LiveStatus, PlayUrl, RoomID, RoomInfo,
    UID, UpInfo,
};
pub use url::parse_room_ref;
pub use wbi::WbiSigner;
