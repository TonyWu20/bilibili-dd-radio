//! Smoke test against the live bilibili APIs.
//!
//! Run: `cargo run -p bili-api --example api_smoke -- [room_id]`

use bili_api::{Client, ClientConfig, Quality};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let candidate: u64 = args.get(1).map(|s| s.parse().unwrap_or(6)).unwrap_or(6);
    let client = Client::from_config(ClientConfig::builder().build());
    let info = client.room_info(candidate).await?;
    println!(
        "room: {} | title: {} | live: {} | online: {}",
        info.room_id,
        info.display_name(),
        info.live_status,
        info.online
    );
    let conf = client.danmu_server_conf(info.room_id).await?;
    println!(
        "danmu servers: {} token len: {}",
        conf.servers.len(),
        conf.token.len()
    );
    if !conf.servers.is_empty() {
        println!(
            "first server: {}:{}",
            conf.servers[0].host, conf.servers[0].wss_port
        );
    }
    let up = client.up_info(info.uid).await;
    match up {
        Ok(info) => println!("up: {}", info.uname),
        Err(e) => println!("up lookup failed (fallback to uid display): {e}"),
    }
    if info.live_status.is_live() {
        let url = client.play_url(info.room_id, Quality::Lowest).await?;
        println!("play url: {}", &url.url[..url.url.len().min(90)]);
    } else {
        println!("room is offline, skipping play url");
    }
    Ok(())
}
