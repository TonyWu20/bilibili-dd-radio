//! Smoke test for the danmaku WS client against the live service.
//!
//! Run: `cargo run -p bili-live --example live_smoke -- [room_id]`

use std::time::Duration;

use bili_api::{Client, ClientConfig};
use bili_live::{LiveClient, LiveEvent};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let candidate: u64 = args.get(1).map(|s| s.parse().unwrap_or(6)).unwrap_or(6);
    let api = Client::from_config(ClientConfig::builder().build());
    let info = api.room_info(candidate).await?;
    println!(
        "room {} | {} | live: {}",
        info.room_id,
        info.display_name(),
        info.live_status
    );
    let (tx, mut rx) = mpsc::unbounded_channel::<LiveEvent>();
    let client = LiveClient::builder()
        .api(api.clone())
        .room(info.room_id)
        .build();
    let handle = client.spawn(tx);
    let mut seen_danmaku = 0u32;
    let deadline = tokio::time::sleep(Duration::from_secs(45));
    tokio::pin!(deadline);
    while seen_danmaku < 3 {
        tokio::select! {
            _ = &mut deadline => break,
            ev = rx.recv() => match ev {
                None => break,
                Some(LiveEvent::Danmaku(d)) => {
                    seen_danmaku += 1;
                    println!("danmaku: {user}: {text}", user = d.user, text = d.text);
                }
                Some(LiveEvent::Connected) => println!("connected"),
                Some(LiveEvent::Disconnected) => println!("disconnected"),
                Some(LiveEvent::Error(e)) => println!("error: {e}"),
                Some(LiveEvent::Online(n)) => println!("online: {n}"),
                Some(LiveEvent::Gift { user, gift, num }) => {
                    println!("gift: {user} x{num} {gift}")
                }
                Some(LiveEvent::RoomChange { title, live }) => {
                    println!("room change: live={live} title={title}")
                }
                Some(LiveEvent::System(m)) => println!("system: {m}"),
            },
        }
    }
    handle.stop();
    println!("smoke done, saw {seen_danmaku} danmaku");
    Ok(())
}
