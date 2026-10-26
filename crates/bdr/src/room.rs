//! Per-room state and the async "open room" pipeline.

use std::collections::VecDeque;

use bili_api::{Client, Quality, RoomID, RoomInfo, UpInfo};
use bili_audio::{AudioEvent, MpvAudio, MpvAudioConfig, MpvHandle};
use bili_live::{LiveClient, LiveEvent};
use tokio::sync::mpsc;

/// How many recent lines a room keeps for the ticker and the danmaku view.
pub const LINE_CAP: usize = 200;

/// One visible line in a room ticker.
#[derive(Clone, Debug)]
pub enum Line {
    Danmaku {
        user: String,
        text: String,
    },
    Gift {
        user: String,
        gift: String,
        num: u32,
    },
    System(String),
}

impl Line {
    /// A single-line, width-trimmed representation for the grid.
    pub fn short(&self, width: usize) -> String {
        let raw = match self {
            Line::Danmaku { user, text } => {
                if text.is_empty() {
                    user.clone()
                } else {
                    format!("{user}: {text}")
                }
            }
            Line::Gift { user, gift, num } => format!("[gift] {user} x{num} {gift}"),
            Line::System(s) => format!("* {s}"),
        };
        truncate(&raw, width)
    }
}

/// Trim a string to `width` unicode chars, with an ellipsis.
pub fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The audio state of one room.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AudioState {
    /// Not started yet.
    #[default]
    Unknown,
    Playing,
    Idle,
    Ended,
    /// mpv is gone (killed or crashed).
    Dead(String),
}

/// State of one open room.
pub struct RoomState {
    /// The user input that produced this room.
    pub input: String,
    /// True once the async pipeline resolved the room.
    pub resolved: bool,
    pub room_id: Option<RoomID>,
    pub info: Option<RoomInfo>,
    /// The UP display name. `None` until the lookup finishes.
    pub up_name: Option<String>,
    /// Latest online count from the WS, when known.
    pub online: Option<u64>,
    pub ws_handle: Option<bili_live::LiveHandle>,
    pub ws_rx: Option<mpsc::UnboundedReceiver<LiveEvent>>,
    pub audio: Option<MpvHandle>,
    pub audio_rx: Option<mpsc::UnboundedReceiver<AudioEvent>>,
    pub muted: bool,
    pub audio_state: AudioState,
    /// The user video toggle for this room.
    pub video: bool,
    /// The (top, left) cell of the running video overlay.
    /// None when the current mpv instance is audio-only.
    pub video_spawn: Option<(u32, u32)>,
    /// True while a respawn task runs for this room. The sync
    /// pass skips such rooms, so one toggle schedules one task.
    pub video_pending: bool,
    /// The kitty image id this room's overlay owns. Assigned once,
    /// when the overlay first turns on, and stable for the room's
    /// whole life in the app.
    pub kitty_id: Option<u32>,
    /// The stream URL, for resync after a remote reconnect.
    pub stream_url: Option<String>,
    /// Recent lines, oldest first.
    pub lines: VecDeque<Line>,
    /// A transient status or error line for the status bar.
    pub note: Option<String>,
}

impl RoomState {
    /// A placeholder for a room that is still resolving.
    pub fn pending(input: String) -> Self {
        Self {
            input,
            resolved: false,
            room_id: None,
            info: None,
            up_name: None,
            online: None,
            ws_handle: None,
            ws_rx: None,
            audio: None,
            audio_rx: None,
            muted: false,
            audio_state: AudioState::Unknown,
            video: false,
            video_spawn: None,
            video_pending: false,
            kitty_id: None,
            stream_url: None,
            lines: VecDeque::with_capacity(LINE_CAP),
            note: None,
        }
    }

    /// A danmaku ticker line for the grid cell.
    pub fn last_lines(&self, n: usize) -> Vec<&Line> {
        let start = self.lines.len().saturating_sub(n);
        self.lines.iter().skip(start).collect()
    }

    /// The display title: room title, or the input while resolving.
    pub fn title(&self) -> String {
        self.info
            .as_ref()
            .map(|i| i.display_name().to_string())
            .unwrap_or_else(|| "resolving…".to_string())
    }

    /// The display name of the room owner.
    pub fn owner(&self) -> String {
        match &self.up_name {
            Some(name) => name.clone(),
            None => self
                .info
                .as_ref()
                .map(|i| format!("uid:{}", i.uid.get()))
                .unwrap_or_else(|| "…".into()),
        }
    }

    /// Apply one WS event to the room state.
    pub fn apply_live(&mut self, ev: LiveEvent) {
        match ev {
            LiveEvent::Danmaku(d) => {
                // Heart clicks arrive with empty text. Skip them.
                if !d.text.is_empty() {
                    self.push_line(Line::Danmaku {
                        user: d.user,
                        text: d.text,
                    });
                }
            }
            LiveEvent::Gift { user, gift, num } => self.push_line(Line::Gift { user, gift, num }),
            LiveEvent::System(s) => self.push_line(Line::System(s)),
            LiveEvent::Online(n) => self.online = Some(n),
            LiveEvent::RoomChange { title, live } => {
                if let Some(info) = &mut self.info {
                    info.title = title;
                    info.live_status = bili_api::LiveStatus(live as u8);
                }
            }
            LiveEvent::Connected => self.note = None,
            LiveEvent::Disconnected => {}
            LiveEvent::Error(e) => self.note = Some(format!("ws: {e}")),
        }
    }

    /// Apply one audio event to the room state.
    pub fn apply_audio(&mut self, ev: AudioEvent) {
        match ev {
            AudioEvent::Playing => self.audio_state = AudioState::Playing,
            AudioEvent::Idle => self.audio_state = AudioState::Idle,
            AudioEvent::Ended(_) => self.audio_state = AudioState::Ended,
            AudioEvent::Gone(reason) => {
                self.note = Some(format!("audio: {reason}"));
                self.audio_state = AudioState::Dead(reason);
            }
        }
    }

    fn push_line(&mut self, line: Line) {
        self.lines.push_back(line);
        if self.lines.len() > LINE_CAP {
            self.lines.pop_front();
        }
    }

    /// Toggle mute and forward it to mpv.
    pub fn toggle_mute(&mut self, enabled: bool) {
        self.muted = enabled;
        if let Some(audio) = &self.audio {
            audio.set_muted(enabled);
        }
    }

    /// A load command for remote resync, when the room is live.
    pub fn resync_load(&self) -> Option<(u64, &str)> {
        let id = self.room_id?.get();
        let url = self.stream_url.as_deref()?;
        let live = self.info.as_ref().is_some_and(|i| i.live_status.is_live());
        live.then_some((id, url))
    }

    /// Stop the room's tasks and processes.
    pub fn close(&mut self) {
        if let Some(h) = self.ws_handle.take() {
            h.stop();
        }
        if let Some(mut a) = self.audio.take() {
            a.stop();
        }
        self.ws_rx = None;
        self.audio_rx = None;
    }
}

/// The Unix socket path of one room mpv instance.
pub fn room_socket(room_id: u64) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("bdr-room-{room_id}.sock"))
}

/// Messages from an audio-or-video respawn.
#[derive(Debug)]
pub enum RespawnMsg {
    /// The new instance is up. Attach it to the room.
    Ready {
        input: String,
        handle: MpvHandle,
        rx: mpsc::UnboundedReceiver<AudioEvent>,
        /// The overlay cell of the new instance, or none for
        /// audio-only.
        video_spawn: Option<(u32, u32)>,
    },
    /// The respawn failed. The room keeps its old instance.
    Failed { input: String, err: String },
}

/// Respawn one room mpv with a new profile (video on or off).
///
/// The caller owns the result channel. The old instance stops
/// when the app swaps the handles in [`drain order`].
pub async fn respawn_audio(
    input: String,
    config: bili_audio::MpvAudioConfig,
    video_spawn: Option<(u32, u32)>,
    out: mpsc::UnboundedSender<RespawnMsg>,
) {
    if std::env::var("BDR_TRACE").is_ok() {
        eprintln!(
            "[bdr] respawn start input={} video={}",
            input,
            config.video
        );
    }
    let socket = config.socket_path.clone();
    let _ = std::fs::remove_file(&socket);
    let (tx, rx) = mpsc::unbounded_channel::<AudioEvent>();
    match bili_audio::MpvAudio::from_config(config).start(tx).await {
        Ok(handle) => {
            if std::env::var("BDR_TRACE").is_ok() {
                eprintln!("[bdr] respawn ok input={input}");
            }
            let _ = out.send(RespawnMsg::Ready {
                input,
                handle,
                rx,
                video_spawn,
            });
        }
        Err(e) => {
            if std::env::var("BDR_TRACE").is_ok() {
                eprintln!("[bdr] respawn failed input={input}: {e}");
            }
            let _ = out.send(RespawnMsg::Failed {
                input,
                err: e.to_string(),
            });
        }
    }
}

/// A room that is being opened.
/// Messages from the open pipeline. The `Ready` variant is large, so
/// keep it on the heap to avoid padding the small variants.
#[allow(clippy::large_enum_variant)]
pub enum OpenMsg {
    /// The room is open. Attach these handles to the room state.
    Ready {
        input: String,
        room_id: RoomID,
        info: RoomInfo,
        ws_handle: bili_live::LiveHandle,
        ws_rx: mpsc::UnboundedReceiver<LiveEvent>,
        audio: Option<MpvHandle>,
        audio_rx: Option<mpsc::UnboundedReceiver<AudioEvent>>,
        stream_url: Option<String>,
        note: Option<String>,
    },
    /// The UP display name finished resolving.
    UpName {
        room_id: RoomID,
        name: Option<UpInfo>,
    },
    /// Opening failed.
    Fail { input: String, err: String },
}

/// The async pipeline that opens one room.
///
/// It resolves the room, starts the danmaku session, and (when the room
/// is live) starts the audio. Results come back through `out`.
pub fn spawn_open(
    api: Client,
    candidate: u64,
    input: String,
    out: mpsc::UnboundedSender<OpenMsg>,
    remote: Option<&crate::audio_remote::RemoteAudio>,
) {
    // An owned handle that shares the connection's command channel.
    let remote = remote.map(|r| r.shared());
    tokio::spawn(async move {
        let info = match api.room_info(candidate).await {
            Ok(info) => info,
            Err(e) => {
                let _ = out.send(OpenMsg::Fail {
                    input,
                    err: e.to_string(),
                });
                return;
            }
        };

        // UP name lookup, in parallel with the rest.
        {
            let api = api.clone();
            let out = out.clone();
            let uid = info.uid;
            tokio::spawn(async move {
                let name = api.up_info(uid).await.ok();
                let _ = out.send(OpenMsg::UpName {
                    room_id: info.room_id,
                    name,
                });
            });
        }

        let room_id = info.room_id;
        let (ws_tx, ws_rx) = mpsc::unbounded_channel::<LiveEvent>();
        let ws_handle = LiveClient::builder()
            .api(api.clone())
            .room(room_id)
            .build()
            .spawn(ws_tx);

        // Audio, when the room is live. Local mpv, or a remote runner.
        let mut audio: Option<MpvHandle> = None;
        let mut audio_rx: Option<mpsc::UnboundedReceiver<AudioEvent>> = None;
        let mut stream_url: Option<String> = None;
        let mut note: Option<String> = None;
        if info.live_status.is_live() {
            match api.play_url(room_id, Quality::Lowest).await {
                Ok(play) => {
                    stream_url = Some(play.url.clone());
                    match remote {
                        Some(remote) => {
                            remote.send_load(room_id.get(), &play.url);
                            if std::env::var("BDR_TRACE").is_ok() {
                                eprintln!("[bdr] remote load room={room_id}");
                            }
                        }
                        None => {
                            let socket = room_socket(room_id.get());
                            let _ = std::fs::remove_file(&socket);
                            let (a_tx, a_rx) = mpsc::unbounded_channel::<AudioEvent>();
                            let config = MpvAudioConfig::builder()
                                .url(play.url)
                                .socket_path(socket)
                                .build();
                            let started = MpvAudio::from_config(config).start(a_tx).await;
                            if std::env::var("BDR_TRACE").is_ok() {
                                eprintln!("[bdr] mpv start: {started:?}");
                            }
                            match started {
                                Ok(handle) => {
                                    audio = Some(handle);
                                    audio_rx = Some(a_rx);
                                }
                                Err(e) => note = Some(format!("audio: {e}")),
                            }
                        }
                    }
                }
                Err(e) => note = Some(format!("play url: {e}")),
            }
        } else {
            note = Some("room is offline, no audio".into());
        }

        let _ = out.send(OpenMsg::Ready {
            input,
            room_id,
            info,
            ws_handle,
            ws_rx,
            audio,
            audio_rx,
            stream_url,
            note,
        });
    });
}
