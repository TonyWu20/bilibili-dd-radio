//! Smoke test for the mpv audio controller.
//!
//! Run with mpv on PATH:
//! `cargo run -p bili-audio --example audio_smoke -- <url> [secs]`

use std::time::Duration;

use bili_audio::{AudioEvent, MpvAudio, MpvAudioConfig};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let url = args.get(1).cloned().ok_or_else(|| anyhow::anyhow!("usage: audio_smoke <url> [secs]"))?;
    let secs: u64 = args.get(2).map(|s| s.parse().unwrap_or(20)).unwrap_or(20);
    let socket = std::env::temp_dir().join(format!("bdr-audio-smoke-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let (tx, mut rx) = mpsc::unbounded_channel::<AudioEvent>();
    let config = MpvAudioConfig::builder()
        .url(url.clone())
        .socket_path(socket.clone())
        .build();
    eprintln!("starting mpv...");
    let start_result = MpvAudio::from_config(config).start(tx).await;
    match start_result {
        Ok(mut handle) => {
            eprintln!("mpv started");
            let mut deadline = tokio::time::sleep(Duration::from_secs(secs));
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    _ = &mut deadline => break,
                    ev = rx.recv() => match ev {
                        None => {
                            eprintln!("channel closed");
                            break;
                        }
                        Some(e) => println!("audio: {e:?}"),
                    },
                }
            }
            handle.stop();
        }
        Err(e) => {
            eprintln!("start failed: {e}");
            anyhow::bail!("{e}");
        }
    }
    println!("audio smoke done");
    Ok(())
}
