//! The mpv JSON IPC player.

use std::path::PathBuf;
use std::process::Stdio;

use bon::Builder;
use serde_json::Value;

use crate::error::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

/// Events from one mpv instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioEvent {
    /// A stream started or resumed.
    Playing,
    /// Nothing is playing.
    Idle,
    /// The stream ended. The reason is the mpv `reason` string.
    Ended(String),
    /// The mpv process is gone.
    Gone(String),
}

/// Builder inputs for [`MpvAudio`].
#[derive(Debug, Builder)]
pub struct MpvAudioConfig {
    /// The stream URL to play (HTTP-FLV or HLS).
    pub url: String,
    /// The Unix socket path for mpv JSON IPC.
    pub socket_path: PathBuf,
    /// Initial volume in percent, 0 to 100.
    #[builder(default = 100.0)]
    pub volume: f64,
    /// Start muted.
    #[builder(default)]
    pub muted: bool,
}

/// One mpv instance, used for one room.
///
/// Start it with [`MpvAudio::start`].
#[derive(Debug)]
pub struct MpvAudio {
    config: MpvAudioConfig,
}

impl MpvAudio {
    /// Create a controller from builder inputs.
    pub fn from_config(config: MpvAudioConfig) -> Self {
        Self { config }
    }

    /// Start the mpv process and its IPC loops.
    ///
    /// The caller owns the event receiver. Commands go out through
    /// the returned [`MpvHandle`].
    pub async fn start(
        self,
        events: mpsc::UnboundedSender<AudioEvent>,
    ) -> Result<MpvHandle, Error> {
        let socket = self.config.socket_path.clone();
        let child = Command::new("mpv")
            .arg("--no-video")
            .arg("--really-quiet")
            .arg("--idle=yes")
            .arg(format!("--input-ipc-server={}", socket.display()))
            .arg(&self.config.url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(Error::Spawn)?;

        // Wait for the IPC socket, then connect.
        let stream = connect_socket(&socket).await?;
        let (rd, wr) = tokio::io::split(stream);

        // Command channel.
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<(u64, Value)>();

        tokio::spawn(reader_loop(BufReader::new(rd), events.clone()));
        tokio::spawn(writer_loop(wr, cmd_rx));

        let handle = MpvHandle {
            cmd_tx,
            socket_path: socket.clone(),
            child,
        };
        if self.config.muted {
            handle.set_muted(true);
        } else {
            handle.set_volume(self.config.volume);
        }
        Ok(handle)
    }
}

/// A running mpv instance.
#[derive(Debug)]
pub struct MpvHandle {
    cmd_tx: mpsc::UnboundedSender<(u64, Value)>,
    socket_path: PathBuf,
    child: tokio::process::Child,
}

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl MpvHandle {
    /// Set the mute state.
    pub fn set_muted(&self, muted: bool) {
        self.send(serde_json::json!(["set_property", "mute", muted]));
    }

    /// Set the volume in percent.
    pub fn set_volume(&self, volume: f64) {
        self.send(serde_json::json!(["set_property", "volume", volume]));
    }

    /// Load a new stream URL, replacing the current one.
    pub fn reload(&self, url: &str) {
        self.send(serde_json::json!(["loadfile", url, "replace"]));
    }

    /// Send a command on the shared writer.
    pub fn send(&self, cmd: Value) {
        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _ = self.cmd_tx.send((id, cmd));
    }

    /// Quit mpv and remove the socket file.
    pub fn stop(&mut self) {
        self.send(serde_json::json!(["quit"]));
        let _ = self.child.start_kill();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl Drop for MpvHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn connect_socket(socket: &PathBuf) -> Result<tokio::net::UnixStream, Error> {
    use std::time::Duration;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match tokio::net::UnixStream::connect(socket).await {
            Ok(s) => return Ok(s),
            Err(e) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(Error::Connect(e));
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

/// Read IPC lines from mpv and map events.
async fn reader_loop<R>(rd: R, events: mpsc::UnboundedSender<AudioEvent>)
where
    R: tokio::io::AsyncBufRead + std::marker::Unpin + Send + 'static,
{
    let mut lines = BufReader::new(rd).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(event) = value["event"].as_str() {
            match event {
                "file_loaded" => {
                    let _ = events.send(AudioEvent::Playing);
                }
                "idle" => {
                    let _ = events.send(AudioEvent::Idle);
                }
                "end_file" => {
                    let reason = value["data"]["reason"].as_str().unwrap_or("unknown");
                    let _ = events.send(AudioEvent::Ended(reason.to_string()));
                }
                _ => {}
            }
        }
    }
    // The reader side closed. The process may still run, but the IPC
    // channel is dead. Report it so the app can respawn.
    let _ = events.send(AudioEvent::Gone("IPC connection closed".into()));
}

/// Write commands to mpv as JSON lines.
async fn writer_loop<W>(mut wr: W, mut cmd_rx: mpsc::UnboundedReceiver<(u64, Value)>)
where
    W: tokio::io::AsyncWrite + std::marker::Unpin + Send + 'static,
{
    while let Some((id, cmd)) = cmd_rx.recv().await {
        let msg = serde_json::json!({ "command": cmd, "request_id": id });
        let line = format!("{msg}\n");
        let _ = wr.write_all(line.as_bytes()).await;
        let _ = wr.flush().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed synthetic mpv IPC events through the reader and verify the
    /// mapped [`AudioEvent`]s.
    #[tokio::test]
    async fn test_reader_maps_events() {
        // `reader` reads one end of the pair; this task writes events
        // into the other end, then closes it.
        let (read_end, mut write_end) = tokio::net::UnixStream::pair().unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel::<AudioEvent>();
        let reader = tokio::spawn(reader_loop(tokio::io::BufReader::new(read_end), tx));
        use tokio::io::AsyncWriteExt;
        let _ = write_end
            .write_all(b"{\"event\":\"file_loaded\",\"data\":{\"path\":\"x.flv\"}}\n")
            .await;
        let _ = write_end
            .write_all(b"{\"event\":\"idle\",\"data\":{\"reason\":\"stream_eof\"}}\n")
            .await;
        let _ = write_end
            .write_all(b"{\"event\":\"end_file\",\"data\":{\"reason\":\"eof\"}}\n")
            .await;
        drop(write_end);

        let got = [
            rx.recv().await,
            rx.recv().await,
            rx.recv().await,
            rx.recv().await,
        ];
        reader.await.unwrap();
        assert_eq!(got[0], Some(AudioEvent::Playing));
        assert_eq!(got[1], Some(AudioEvent::Idle));
        assert_eq!(got[2], Some(AudioEvent::Ended("eof".into())));
        assert_eq!(
            got[3],
            Some(AudioEvent::Gone("IPC connection closed".into()))
        );
    }

    /// The writer loop writes one JSON line per command.
    #[tokio::test]
    async fn test_writer_sends_commands() {
        // The writer writes into `write_end`; this task reads the other
        // end of the pair.
        let (write_end, mut read_end) = tokio::net::UnixStream::pair().unwrap();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<(u64, Value)>();
        let writer = tokio::spawn(writer_loop(write_end, cmd_rx));

        let _ = cmd_tx.send((1, serde_json::json!(["set_property", "mute", true])));
        let _ = cmd_tx.send((2, serde_json::json!(["loadfile", "u://x", "replace"])));
        drop(cmd_tx);
        writer.await.unwrap();

        // The writer dropped its write half, so the read side sees EOF.
        let mut out = String::new();
        use tokio::io::AsyncReadExt;
        let _ = read_end.read_to_string(&mut out).await;
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        let v1: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(
            v1["command"],
            serde_json::json!(["set_property", "mute", true])
        );
        assert_eq!(v1["request_id"], 1);
        let v2: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(
            v2["command"],
            serde_json::json!(["loadfile", "u://x", "replace"])
        );
    }
}
