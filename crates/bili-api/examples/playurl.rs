//! Print a playable stream URL for a room.
//!
//! Run: `cargo run -p bili-api --example playurl -- [room_id]`

use bili_api::{Client, ClientConfig, Quality};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let candidate: u64 = args.get(1).map(|s| s.parse().unwrap_or(6)).unwrap_or(6);
    let client = Client::from_config(ClientConfig::builder().build());
    let info = client.room_info(candidate).await?;
    let url = client.play_url(info.room_id, Quality::Lowest).await?;
    println!("{}", url.url);
    Ok(())
}
