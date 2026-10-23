//! Headless check of the QR login endpoints (generate + one poll).
//!
//! Run: `cargo run -p bili-api --example login_smoke`.

use bili_api::Client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let api = Client::new();
    let qr = api.qrcode_generate().await?;
    let key: String = qr.qrcode_key.chars().take(8).collect();
    println!("qrcode_key={key}... ({} chars)", qr.qrcode_key.len());
    println!("url={}", &qr.url[..qr.url.len().min(60)]);
    let poll = api.qrcode_poll(&qr.qrcode_key).await?;
    println!("poll status={:?}", poll.status);
    Ok(())
}
