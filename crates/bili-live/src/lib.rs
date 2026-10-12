//! In-house bilibili live danmaku WebSocket client.
//!
//! Join one room, receive danmaku, gifts, online counts, room
//! changes, and system messages. The session reconnects
//! automatically with exponential backoff.

mod cmd;
mod client;
mod packet;

pub use cmd::{Danmaku, Danmaku as DanmakuLine};
pub use client::{LiveClient, LiveClientConfig, LiveEvent, LiveHandle};
