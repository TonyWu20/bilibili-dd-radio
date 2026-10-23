//! The remote audio control channel.
//!
//! When the TUI runs on a headless machine, audio plays on a laptop that
//! runs `bdr audio-local`. The two talk JSON lines over a TCP connection
//! that the user bridges with an SSH reverse forward.
//!
//! TUI to runner:
//! `{"cmd":"load","room":"7734200","url":"..."}`
//! `{"cmd":"mute","room":"7734200","muted":true}`
//! `{"cmd":"stop","room":"7734200"}`
//! `{"cmd":"quit"}`
//!
//! Runner to TUI:
//! `{"event":"playing","room":"7734200"}` (also `idle`, `ended`, `gone`)

use bili_audio::AudioEvent;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use std::net::SocketAddr;

/// Messages from the remote audio runner.
#[derive(Debug)]
pub enum RemoteMsg {
    /// An audio event for one room.
    Event { room: u64, event: AudioEvent },
    /// The control connection is established.
    Up,
    /// The control connection is gone.
    Lost,
}

/// The TUI side of the remote audio channel.
pub struct RemoteAudio {
    cmd_tx: mpsc::UnboundedSender<Value>,
}

impl RemoteAudio {
    /// Connect once and start the reader/writer tasks.
    #[cfg(test)]
    pub async fn connect(
        addr: SocketAddr,
        out: mpsc::UnboundedSender<RemoteMsg>,
    ) -> Result<Self, std::io::Error> {
        let stream = TcpStream::connect(addr).await?;
        let (rd, wr) = stream.into_split();
        let (drop_tx, _drop_rx) = mpsc::unbounded_channel();
        Ok(Self::from_halves(rd, wr, out, drop_tx).await)
    }

    /// A stable handle that reconnects on its own.
    ///
    /// The TUI can start before the runner. Every new connection sends
    /// `RemoteMsg::Up`. A drop sends `RemoteMsg::Lost`. While
    /// disconnected, queued commands are dropped. The app resyncs
    /// open rooms on `RemoteMsg::Up`.
    pub fn supervised(addr: SocketAddr, out: mpsc::UnboundedSender<RemoteMsg>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Value>();
        tokio::spawn(supervise(addr, cmd_rx, out));
        Self { cmd_tx }
    }

    async fn from_halves(
        rd: tokio::net::tcp::OwnedReadHalf,
        wr: tokio::net::tcp::OwnedWriteHalf,
        out: mpsc::UnboundedSender<RemoteMsg>,
        drop_tx: mpsc::UnboundedSender<()>,
    ) -> Self {
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Value>();
        let writer_drop = drop_tx.clone();

        // Writer: one JSON line per command.
        tokio::spawn(async move {
            let mut wr = wr;
            while let Some(value) = cmd_rx.recv().await {
                let line = format!("{}\n", serde_json::to_string(&value).unwrap_or_default());
                if wr.write_all(line.as_bytes()).await.is_err() {
                    let _ = writer_drop.send(());
                    break;
                }
                let _ = wr.flush().await;
            }
        });

        // Reader: parse event lines, forward them to the app.
        let reader_drop = drop_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(rd).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let Ok(value) = serde_json::from_str::<Value>(&line) else {
                            continue;
                        };
                        if let Some(msg) = parse_event(&value) {
                            let _ = out.send(msg);
                        }
                    }
                    _ => {
                        let _ = out.send(RemoteMsg::Lost);
                        let _ = reader_drop.send(());
                        break;
                    }
                }
            }
        });

        Self { cmd_tx }
    }

    /// Tell the runner to play one room's stream.
    pub fn send_load(&self, room: u64, url: &str) {
        let value = serde_json::json!({
            "cmd": "load",
            "room": room.to_string(),
            "url": url,
        });
        let _ = self.cmd_tx.send(value);
    }

    /// Tell the runner to mute or unmute one room.
    pub fn send_mute(&self, room: u64, muted: bool) {
        let value = serde_json::json!({
            "cmd": "mute",
            "room": room.to_string(),
            "muted": muted,
        });
        let _ = self.cmd_tx.send(value);
    }

    /// Tell the runner to stop one room.
    pub fn send_stop(&self, room: u64) {
        let value = serde_json::json!({
            "cmd": "stop",
            "room": room.to_string(),
        });
        let _ = self.cmd_tx.send(value);
    }

    /// Tell the runner to stop all rooms.
    pub fn send_quit(&self) {
        let _ = self.cmd_tx.send(serde_json::json!({ "cmd": "quit" }));
    }

    /// An owned handle that shares this connection's command channel.
    pub fn shared(&self) -> Self {
        Self {
            cmd_tx: self.cmd_tx.clone(),
        }
    }
}

/// Keep the control connection alive.
///
/// Retries every three seconds while the runner is down. Each
/// connection has its own drop-notify channel, so a stale
/// notification never affects the next connection. Commands sent
/// while disconnected are dropped; the app resyncs open rooms on
/// `RemoteMsg::Up`.
async fn supervise(
    addr: SocketAddr,
    facade_rx: mpsc::UnboundedReceiver<Value>,
    out: mpsc::UnboundedSender<RemoteMsg>,
) {
    let mut facade_rx = facade_rx;
    loop {
        let (drop_tx, mut drop_rx) = mpsc::unbounded_channel::<()>();
        match TcpStream::connect(addr).await {
            Ok(stream) => {
                let (rd, wr) = stream.into_split();
                let conn = RemoteAudio::from_halves(rd, wr, out.clone(), drop_tx).await;
                let conn_cmd = conn.cmd_tx.clone();
                let _ = out.send(RemoteMsg::Up);
                // Forward facade commands until this connection drops.
                'live: loop {
                    tokio::select! {
                        value = facade_rx.recv() => {
                            let Some(value) = value else {
                                return;
                            };
                            if conn_cmd.send(value).is_err() {
                                break 'live;
                            }
                        }
                        _ = drop_rx.recv() => {
                            break 'live;
                        }
                    }
                }
                drop(conn);
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            Err(_) => {
                // Hint for the "never connected" case, same channel.
                let _ = out.send(RemoteMsg::Lost);
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
}

/// Parse one runner event line.
fn parse_event(value: &Value) -> Option<RemoteMsg> {
    let event = value["event"].as_str()?;
    let room: u64 = value["room"].as_str()?.parse().ok()?;
    let mapped = match event {
        "playing" => AudioEvent::Playing,
        "idle" => AudioEvent::Idle,
        "ended" => AudioEvent::Ended(value["reason"].as_str().unwrap_or("eof").into()),
        "gone" => AudioEvent::Gone(
            value["reason"]
                .as_str()
                .unwrap_or("remote runner closed")
                .into(),
        ),
        _ => return None,
    };
    Some(RemoteMsg::Event {
        room,
        event: mapped,
    })
}

/// Parse a `host:port` string into a socket address.
pub fn parse_addr(input: &str) -> Result<SocketAddr, std::io::Error> {
    use std::net::ToSocketAddrs;
    input
        .to_socket_addrs()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty address"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Commands go out as JSON lines; events come back tagged by room.
    #[tokio::test]
    async fn test_remote_round_trip() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<RemoteMsg>();
        let remote = RemoteAudio::connect(addr, out_tx).await.unwrap();

        let (server, _) = listener.accept().await.unwrap();

        // The server sees the load command.
        let (rd, mut wr) = server.into_split();
        remote.send_load(42, "u://x");
        use tokio::io::AsyncWriteExt;
        let mut lines = tokio::io::BufReader::new(rd).lines();
        let line = tokio::time::timeout(std::time::Duration::from_secs(3), lines.next_line())
            .await
            .expect("load line timed out")
            .expect("load read failed")
            .expect("no load line");
        let value: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["cmd"], "load");
        assert_eq!(value["room"], "42");
        assert_eq!(value["url"], "u://x");

        // The client sees the event.
        let event = serde_json::json!({ "event": "idle", "room": "42" });
        let _ = wr.write_all(format!("{event}\n").as_bytes()).await;
        let _ = wr.flush().await;
        let msg = tokio::time::timeout(std::time::Duration::from_secs(3), out_rx.recv())
            .await
            .expect("event timed out");
        match msg {
            Some(RemoteMsg::Event { room, event }) => {
                assert_eq!(room, 42);
                assert_eq!(event, AudioEvent::Idle);
            }
            other => panic!("expected event, got {other:?}"),
        }

        // When the server goes away, the client reports Lost.
        drop(lines);
        drop(wr);
        drop(remote);
        let lost = tokio::time::timeout(std::time::Duration::from_secs(3), out_rx.recv())
            .await
            .expect("lost timed out");
        assert!(matches!(lost, Some(RemoteMsg::Lost)));
    }

    /// A bad line is skipped; a good line is parsed.
    #[tokio::test]
    async fn test_bad_lines_skipped() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<RemoteMsg>();
        let remote = RemoteAudio::connect(addr, out_tx).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();

        let (_rd, mut wr) = server.into_split();
        use tokio::io::AsyncWriteExt;
        let _ = wr.write_all(b"not json\n").await;
        let _ = wr
            .write_all(b"{\"event\":\"playing\",\"room\":\"7\"}\n")
            .await;
        let _ = wr.flush().await;
        drop(_rd);
        drop(wr);
        drop(remote);

        let mut seen = Vec::new();
        for _ in 0..4 {
            match tokio::time::timeout(std::time::Duration::from_secs(3), out_rx.recv()).await {
                Ok(Some(RemoteMsg::Event { room, .. })) => seen.push(room),
                Ok(Some(RemoteMsg::Lost)) => break,
                Ok(None) => break,
                _ => panic!("recv timeout"),
            }
        }
        assert_eq!(seen, vec![7]);
    }

    /// The supervisor reconnects and routes commands to the new link.
    #[tokio::test]
    async fn test_supervised_reconnects() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<RemoteMsg>();
        let remote = RemoteAudio::supervised(addr, out_tx);

        // First link comes up.
        let (c1, _) = listener.accept().await.unwrap();
        let up = next_msg(&mut out_rx).await;
        assert!(matches!(up, Some(RemoteMsg::Up)));

        // A load reaches the server through the stable facade.
        let (rd1, wr1) = c1.into_split();
        remote.send_load(11, "u://11");
        let mut lines1 = tokio::io::BufReader::new(rd1).lines();
        let line1 = tokio::time::timeout(std::time::Duration::from_secs(5), lines1.next_line())
            .await
            .expect("load 1 timed out")
            .expect("load 1 read failed")
            .expect("no load 1 line");
        let value: Value = serde_json::from_str(line1.trim()).unwrap();
        assert_eq!(value["room"], "11");

        // Cut the link; the supervisor reports Lost.
        drop(lines1);
        drop(wr1);
        let lost = next_msg(&mut out_rx).await;
        assert!(matches!(lost, Some(RemoteMsg::Lost)));

        // The supervisor reconnects to the same listener.
        let (c2, _) = tokio::time::timeout(std::time::Duration::from_secs(10), listener.accept())
            .await
            .expect("reconnect timed out")
            .expect("reconnect accept failed");
        let up2 = next_msg(&mut out_rx).await;
        assert!(matches!(up2, Some(RemoteMsg::Up)));

        // A load after the reconnect reaches the new link.
        let (rd2, _wr2) = c2.into_split();
        remote.send_load(12, "u://12");
        let mut lines2 = tokio::io::BufReader::new(rd2).lines();
        let line2 = tokio::time::timeout(std::time::Duration::from_secs(5), lines2.next_line())
            .await
            .expect("load 2 timed out")
            .expect("load 2 read failed")
            .expect("no load 2 line");
        let value: Value = serde_json::from_str(line2.trim()).unwrap();
        assert_eq!(value["room"], "12");
    }

    /// The next message, with a timeout.
    async fn next_msg(rx: &mut mpsc::UnboundedReceiver<RemoteMsg>) -> Option<RemoteMsg> {
        tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .ok()
            .flatten()
    }

    /// Address parsing.
    #[test]
    fn test_parse_addr() {
        let addr = parse_addr("127.0.0.1:7888").unwrap();
        assert_eq!(addr.port(), 7888);
        assert!(parse_addr("not-an-address").is_err());
    }
}
