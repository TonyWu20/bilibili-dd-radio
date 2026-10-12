//! The reconnecting danmaku session client.

use std::time::Duration;

use bili_api::RoomID;
use bon::Builder;
use futures_util::{SinkExt, StreamExt};
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{connect_async, tungstenite};

use crate::cmd::{self, Command};
use std::sync::OnceLock;
use crate::packet::{
    self, build_auth_packet, build_heartbeat_packet, decode_business,
    OP_AUTH_REPLY, OP_SEND_MSG_REPLY,
};

/// Events from a live room session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveEvent {
    /// A new connection (re)established.
    Connected,
    /// The connection dropped. The session reconnects automatically.
    Disconnected,
    /// A recent error, for display. The session keeps reconnecting.
    Error(String),
    /// A danmaku line.
    Danmaku(cmd::Danmaku),
    /// A gift.
    Gift { user: String, gift: String, num: u32 },
    /// The room online count changed.
    Online(u64),
    /// The room title or live state changed.
    RoomChange { title: String, live: bool },
    /// A system message (room announcement, entry effect, ...).
    System(String),
}

/// Builder inputs for [`LiveClient`].
#[derive(Builder)]
pub struct LiveClientConfig {
    /// The API client used to fetch the danmaku server config.
    pub api: bili_api::Client,
    /// The room to join. Use the real room id from `RoomInfo`.
    pub room: RoomID,
    /// The logged-in user mid. Use `None` (anonymous) without login.
    pub uid: Option<u64>,
}

/// A session for one live room.
///
/// The session owns its network task. Start it with [`LiveClient::spawn`].
pub struct LiveClient {
    config: LiveClientConfig,
}

/// The constructor builder for [`LiveClient`].
///
/// ```
/// use bili_api::{Client, ClientConfig};
/// use bili_live::LiveClient;
///
/// let api = Client::from_config(ClientConfig::builder().build());
/// let client = LiveClient::builder()
///     .api(api)
///     .room(bili_api::RoomID(7734200))
///     .build();
/// ```
#[bon::bon]
impl LiveClient {
    #[builder]
    pub const fn new(api: bili_api::Client, room: RoomID, uid: Option<u64>) -> Self {
        Self {
            config: LiveClientConfig { api, room, uid },
        }
    }
}

impl std::fmt::Debug for LiveClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveClient")
            .field("room", &self.config.room)
            .finish()
    }
}

/// Stop outcome of one payload walk.
enum Step {
    /// Keep reading payloads.
    Running,
    /// Auth reply arrived with code 0.
    AuthOk,
    /// Auth reply arrived with a non-zero code.
    AuthFailed(i64),
}

/// A handle that stops a danmaku session.
pub struct LiveHandle {
    stop_tx: watch::Sender<bool>,
    room: RoomID,
}

impl LiveHandle {
    /// The room this handle manages.
    pub fn room(&self) -> RoomID {
        self.room
    }

    /// Ask the session to stop. Idempotent.
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }
}

impl Drop for LiveHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

impl LiveClient {
    /// Start the session task. Returns the stop handle.
    pub fn spawn(self, events: mpsc::UnboundedSender<LiveEvent>) -> LiveHandle {
        let room = self.config.room;
        let (stop_tx, mut stop_rx) = watch::channel(false);
        tokio::spawn(async move {
            self.run(&mut stop_rx, events).await;
        });
        LiveHandle { stop_tx, room }
    }

    /// The event loop: connect, pump messages, reconnect with backoff.
    async fn run(&self, stop_rx: &mut watch::Receiver<bool>, events: mpsc::UnboundedSender<LiveEvent>) {
        let mut attempts = 0usize;
        let mut server_idx = 0usize;
        loop {
            let outcome = self.connect_once(stop_rx, &events, &mut server_idx).await;
            let _ = events.send(LiveEvent::Disconnected);
            if *stop_rx.borrow() {
                return;
            }
            match outcome {
                Ok(()) => attempts = 0,
                Err(e) => {
                    let _ = events.send(LiveEvent::Error(e.to_string()));
                }
            }
            let delay = backoff_delay(attempts);
            attempts += 1;
            tokio::select! {
                _ = stop_rx.changed() => return,
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }

    /// One connect + pump cycle. Returns `Ok(())` on a clean close.
    async fn connect_once(
        &self,
        stop_rx: &mut watch::Receiver<bool>,
        events: &mpsc::UnboundedSender<LiveEvent>,
        server_idx: &mut usize,
    ) -> Result<(), Error> {
        let conf = self.config.api.danmu_server_conf(self.config.room).await?;
        if conf.servers.is_empty() {
            return Err(Error::NoServers);
        }
        *server_idx = server_idx.wrapping_add(1);
        let server = &conf.servers[*server_idx % conf.servers.len()];
        let url = format!("wss://{}:{}/sub", server.host, server.wss_port);
        let mut ws = match tokio::time::timeout(Duration::from_secs(10), connect_async(&url)).await {
            Ok(Ok((ws, _))) => ws,
            Ok(Err(e)) => return Err(Error::Ws(e)),
            Err(_) => return Err(Error::ConnectTimeout),
        };
        let buvid = self.config.api.buvid3().await.unwrap_or_default();
        let uid = self.config.uid.unwrap_or(0);
        ws.send(tungstenite::Message::Binary(
            build_auth_packet(uid, self.config.room.get(), &buvid, &conf.token).into(),
        ))
        .await
        .map_err(Error::Ws)?;

        // Wait for the auth reply. Business messages may arrive first.
        let deadline_at = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline_at) => return Err(Error::AuthTimeout),
                _ = stop_rx.changed() => return Ok(()),
                msg = ws.next() => {
                    let Some(msg) = msg else { return Ok(()) };
                    let Some(bytes) = to_bytes(msg.map_err(Error::Ws)?) else { continue };
                    let step = self.pump_payload(&bytes, events, true);
                    match step {
                        Step::AuthOk => break,
                        Step::AuthFailed(code) => return Err(Error::AuthRejected(code)),
                        Step::Running => {}
                    }
                }
            }
        };
        let _ = events.send(LiveEvent::Connected);

        // Pump messages until the socket dies or the session stops.
        let mut hb = tokio::time::interval(Duration::from_secs(30));
        hb.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        hb.tick().await; // the first tick completes immediately
        loop {
            tokio::select! {
                _ = stop_rx.changed() => {
                    let _ = ws.close(None).await;
                    return Ok(());
                }
                _ = hb.tick() => {
                    let ok = ws
                        .send(tungstenite::Message::Binary(build_heartbeat_packet().into()))
                        .await
                        .is_ok();
                    if !ok {
                        return Err(Error::Ws(tungstenite::Error::ConnectionClosed));
                    }
                }
                msg = ws.next() => {
                    let Some(msg) = msg else { return Ok(()) };
                    let Some(bytes) = to_bytes(msg.map_err(Error::Ws)?) else { continue };
                    let _ = self.pump_payload(&bytes, events, false);
                }
            }
        }
    }

    /// Walk all frames in one payload. Returns [`Step`] when
    /// `want_auth` is set and an auth reply is seen.
    fn pump_payload(
        &self,
        bytes: &[u8],
        events: &mpsc::UnboundedSender<LiveEvent>,
        want_auth: bool,
    ) -> Step {
        let step = Step::Running;
        let mut off = 0usize;
        while off + packet::HEADER_LEN <= bytes.len() {
            let Ok(header) = packet::PacketHeader::read(&bytes[off..]) else {
                break;
            };
            let end = (off + header.pack_len as usize).min(bytes.len());
            let body = &bytes[off + packet::HEADER_LEN..end];
            if want_auth && header.operation == OP_AUTH_REPLY {
                let code = serde_json::from_slice::<serde_json::Value>(body)
                    .map(|v| v["code"].as_i64().unwrap_or(-1))
                    .unwrap_or(-1);
                if code == 0 {
                    return Step::AuthOk;
                }
                return Step::AuthFailed(code);
            }
            if header.operation == OP_SEND_MSG_REPLY {
                if trace_enabled() {
                    eprintln!(
                        "frame op={} ver={} body_len={}",
                        header.operation,
                        header.ver,
                        body.len()
                    );
                }
                self.emit_business(header.ver, body, events);
            }
            if (header.pack_len as usize) <= packet::HEADER_LEN {
                break;
            }
            off = end;
        }
        step
    }

    fn emit_business(&self, ver: u16, body: &[u8], events: &mpsc::UnboundedSender<LiveEvent>) {
        match decode_business(ver, body) {
            Ok(cmds) => {
                for c in &cmds {
                    if trace_enabled() {
                        eprintln!("cmd: {} ({} commands in payload)", c.cmd, cmds.len());
                    }
                    self.emit_command(c, events);
                }
            }
            Err(e) => {
                if trace_enabled() {
                    eprintln!("decode_business ver={ver} failed: {e}");
                }
            }
        }
    }

    fn emit_command(&self, c: &Command, events: &mpsc::UnboundedSender<LiveEvent>) {
        match c.cmd.as_str() {
            "DANMU_MSG" => {
                if trace_enabled() {
                    let raw = c.value.to_string();
                    let cut = raw.floor_char_boundary(20000);
                    eprintln!("danmu full: {}", &raw[..cut]);
                }
                for d in cmd::danmus(c) {
                    let _ = events.send(LiveEvent::Danmaku(d));
                }
            }
            "SEND_GIFT" => {
                if let Some((user, gift, num)) = cmd::gift(c) {
                    let _ = events.send(LiveEvent::Gift { user, gift, num });
                }
            }
            "ONLINE_RANK_COUNT" | "ONLINE_CNT_RPK" => {
                if let Some(count) = cmd::online_count(c) {
                    let _ = events.send(LiveEvent::Online(count));
                }
            }
            "ROOM_CHANGE" => {
                if let Some((title, live)) = cmd::room_change(c) {
                    let _ = events.send(LiveEvent::RoomChange { title, live });
                }
            }
            "INTERFERE" | "SYSTEM_MSG" => {
                if let Some(msg) = cmd::system_msg(c) {
                    let _ = events.send(LiveEvent::System(msg));
                }
            }
            _ => {}
        }
    }
}

/// When the `BILI_LIVE_TRACE` environment variable is set, the client
/// writes protocol traces to stderr.
fn trace_enabled() -> bool {
    static ONCE: OnceLock<bool> = OnceLock::new();
    *ONCE.get_or_init(|| std::env::var_os("BILI_LIVE_TRACE").is_some())
}

/// Extract the binary bytes from a WebSocket message.
fn to_bytes(msg: tungstenite::Message) -> Option<Vec<u8>> {
    match msg {
        tungstenite::Message::Binary(b) => Some(b.to_vec()),
        _ => None,
    }
}

/// Exponential backoff, capped at 30 seconds.
fn backoff_delay(attempts: usize) -> Duration {
    let secs = (1u64 << attempts.min(5)).min(30);
    Duration::from_secs(secs)
}

/// Errors from a danmaku session.
#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Api(#[from] bili_api::Error),

    #[error("websocket error: {0}")]
    Ws(#[source] tungstenite::Error),

    #[error("no danmaku servers in config")]
    NoServers,

    #[error("websocket connect timeout")]
    ConnectTimeout,

    #[error("auth reply timeout")]
    AuthTimeout,

    #[error("auth rejected, code {0}")]
    AuthRejected(i64),
}
