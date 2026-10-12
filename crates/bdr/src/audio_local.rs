//! The local audio runner: `bdr audio-local`.
//!
//! Runs on the machine that has speakers. It listens on a TCP port,
//! starts one mpv per room, and applies mute/stop commands. The TUI
//! reaches this port through an SSH reverse forward.
//!
//! The runner keeps running after a TUI disconnects, so the next TUI
//! session can reconnect without restarting audio.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use bili_audio::{AudioEvent, MpvAudio, MpvAudioConfig, MpvHandle};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

type Rooms = Arc<Mutex<HashMap<u64, MpvHandle>>>;
/// Rooms the user stopped while their mpv was still starting.
type Stopped = Arc<Mutex<HashSet<u64>>>;

/// Run the local audio runner on `port`.
pub async fn run_audio_local(port: u16) -> Result<(), anyhow::Error> {
    let addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(addr.as_str()).await?;
    println!("bdr audio-local listening on {addr} (Ctrl-C to stop)");

    let rooms: Rooms = Arc::new(Mutex::new(HashMap::new()));
    let stopped: Stopped = Arc::new(Mutex::new(HashSet::new()));
    loop {
        let (stream, _) = listener.accept().await?;
        let rooms = rooms.clone();
        let stopped = stopped.clone();
        tokio::spawn(handle_connection(stream, rooms, stopped));
    }
}

/// One control connection from the TUI.
async fn handle_connection(stream: TcpStream, rooms: Rooms, stopped: Stopped) {
    let (rd, wr) = stream.into_split();
    let (out_tx, out_rx) = mpsc::unbounded_channel::<String>();

    // Writer: send queued event lines to the TUI.
    let writer = tokio::spawn(async move {
        let mut wr = wr;
        let mut rx = out_rx;
        while let Some(line) = rx.recv().await {
            if wr.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            let _ = wr.flush().await;
        }
    });

    handle_commands(rd, rooms, stopped, out_tx).await;
    let _ = writer.await;
}

/// Read commands and apply them to the room map.
async fn handle_commands(
    rd: tokio::net::tcp::OwnedReadHalf,
    rooms: Rooms,
    stopped: Stopped,
    out_tx: mpsc::UnboundedSender<String>,
) {
    let mut lines = BufReader::new(rd).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match value["cmd"].as_str() {
            Some("load") => {
                let room = value["room"].as_str().and_then(|s| s.parse().ok());
                let Some(room) = room else { continue };
                let url = value["url"].as_str().unwrap_or_default().to_string();
                eprintln!("[audio-local] load room={room}");
                stopped.lock().unwrap().remove(&room);
                load_room(&rooms, &stopped, &out_tx, room, &url);
            }
            Some("mute") => {
                let room = value["room"].as_str().and_then(|s| s.parse().ok());
                let Some(room) = room else { continue };
                let muted = value["muted"].as_bool().unwrap_or(true);
                if let Some(handle) = rooms.lock().unwrap().get(&room) {
                    handle.set_muted(muted);
                }
            }
            Some("stop") => {
                let room = value["room"].as_str().and_then(|s| s.parse().ok());
                let Some(room) = room else { continue };
                eprintln!("[audio-local] stop room={room}");
                stop_room(&rooms, &stopped, room);
            }
            Some("quit") => {
                eprintln!("[audio-local] quit: stopping all rooms");
                stop_all(&rooms, &stopped);
            }
            _ => {}
        }
    }
}

/// Start or replace the mpv for one room.
fn load_room(
    rooms: &Rooms,
    stopped: &Stopped,
    out_tx: &mpsc::UnboundedSender<String>,
    room: u64,
    url: &str,
) {
    {
        let map = rooms.lock().unwrap();
        if let Some(handle) = map.get(&room) {
            handle.reload(url);
            return;
        }
    }

    let socket = std::env::temp_dir().join(format!("bdr-room-{room}.sock"));
    let _ = std::fs::remove_file(&socket);
    let (ev_tx, ev_rx) = mpsc::unbounded_channel::<AudioEvent>();
    let config = MpvAudioConfig::builder()
        .url(url.to_string())
        .socket_path(socket)
        .build();

    // Start mpv on the runtime and forward its events, tagged with the room.
    let out = out_tx.clone();
    let rooms = rooms.clone();
    let stopped = stopped.clone();
    tokio::spawn(async move {
        match MpvAudio::from_config(config).start(ev_tx).await {
            Ok(mut handle) => {
                // A stop may have arrived while mpv was starting.
                if stopped.lock().unwrap().contains(&room) {
                    handle.stop();
                    return;
                }
                if let Ok(mut map) = rooms.lock() {
                    map.insert(room, handle);
                }
                forward_events(ev_rx, out, room).await;
            }
            Err(e) => {
                eprintln!("[audio-local] mpv start failed room={room}: {e}");
                let _ = out.send(event_line(room, "gone", Some(&e.to_string())));
            }
        }
    });
}

/// Tag one room's audio events and queue them for the writer.
async fn forward_events(
    mut rx: mpsc::UnboundedReceiver<AudioEvent>,
    out: mpsc::UnboundedSender<String>,
    room: u64,
) {
    while let Some(ev) = rx.recv().await {
        let line = match ev {
            AudioEvent::Playing => event_line(room, "playing", None),
            AudioEvent::Idle => event_line(room, "idle", None),
            AudioEvent::Ended(reason) => event_line(room, "ended", Some(&reason)),
            AudioEvent::Gone(reason) => event_line(room, "gone", Some(&reason)),
        };
        let _ = out.send(line);
    }
}

/// One JSON event line for the TUI.
fn event_line(room: u64, event: &str, reason: Option<&str>) -> String {
    let mut value = serde_json::json!({ "event": event, "room": room.to_string() });
    if let Some(reason) = reason {
        value["reason"] = serde_json::json!(reason);
    }
    format!("{value}")
}

/// Stop the mpv for one room.
fn stop_room(rooms: &Rooms, stopped: &Stopped, room: u64) {
    if let Some(mut handle) = rooms.lock().unwrap().remove(&room) {
        handle.stop();
    }
    // Mark it so a still-starting mpv drops itself.
    stopped.lock().unwrap().insert(room);
}

/// Stop every room's mpv.
fn stop_all(rooms: &Rooms, stopped: &Stopped) {
    let mut map = rooms.lock().unwrap();
    for (_, mut handle) in map.drain() {
        handle.stop();
    }
    stopped.lock().unwrap().clear();
}
