//! In-house bilibili HTTP API client.
//!
//! Covers the live-room endpoints that `bdr` needs: room info,
//! play URLs, danmaku server config (WBI signed), and UP lookup.

mod client;
mod error;
mod types;
mod url;
mod wbi;

pub use client::{Client, ClientConfig, Quality};
pub use error::Error;
pub use types::{DanmuServer, DanmuServerConf, LiveStatus, PlayUrl, RoomID, RoomInfo, UpInfo, UID};
pub use url::parse_room_ref;
pub use wbi::WbiSigner;
