//! Per-room audio playback through an external mpv process.
//!
//! Each room gets one mpv instance. The OS audio stack (PipeWire or
//! PulseAudio) mixes the instances. This crate spawns mpv, speaks
//! JSON IPC to it, and reports playback events.

mod error;
mod player;

pub use error::Error;
pub use player::{AudioEvent, MpvAudio, MpvAudioConfig, MpvHandle};
