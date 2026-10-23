//! Headless check of the live home (followed + recommended rooms).
//!
//! Run: `cargo run -p bili-api --example home_smoke`.
//! With a login cookie in the environment it prints followed rooms:
//! `BDR_COOKIE="SESSDATA=...; bili_jct=..." cargo run ...`

use bili_api::ClientConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cookie = std::env::var("BDR_COOKIE").unwrap_or_default();
    let api = bili_api::Client::from_config(ClientConfig::builder().cookie(cookie).build());

    let home = api.live_home().await?;
    println!("followed: {}", home.followed.len());
    for r in home.followed.iter().take(20) {
        println!(
            "  {} {} online={} room={}",
            r.uname,
            r.title,
            r.online,
            r.room_id.get()
        );
    }
    println!("recommended: {}", home.recommended.len());
    for r in home.recommended.iter().take(5) {
        println!(
            "  {} {} online={} room={}",
            r.uname,
            r.title,
            r.online,
            r.room_id.get()
        );
    }
    Ok(())
}
