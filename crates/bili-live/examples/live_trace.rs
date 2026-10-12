//! Raw trace of one danmaku WS session, for protocol debugging.
//!
//! Run: `cargo run -p bili-live --example live_trace -- [room_id] [secs]`

use std::time::Duration;

use bili_api::{Client, ClientConfig};
use bili_live::{LiveClient, LiveEvent};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let candidate: u64 = args.get(1).map(|s| s.parse().unwrap_or(6)).unwrap_or(6);
    let secs: u64 = args.get(2).map(|s| s.parse().unwrap_or(60)).unwrap_or(60);
    let api = Client::from_config(ClientConfig::builder().build());
    let info = api.room_info(candidate).await?;
    println!("room {} | {} | live: {}", info.room_id, info.display_name(), info.live_status);
    let (tx, mut rx) = mpsc::unbounded_channel::<LiveEvent>();
    let handle = LiveClient::builder()
        .api(api.clone())
        .room(info.room_id)
        .build()
        .spawn(tx);
    let mut deadline = tokio::time::sleep(Duration::from_secs(secs));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            ev = rx.recv() => match ev {
                None => break,
                Some(e) => println!("ev: {e:?}"),
            },
        }
    }
    handle.stop();
    Ok(())
}
