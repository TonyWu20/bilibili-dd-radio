//! bdr: listen to several bilibili live rooms at once.
//!
//! The default command opens the TUI. Use `bdr probe` to check a room
//! headlessly. Use `bdr login` to log in from a plain terminal.

mod app;
mod audio_local;
mod audio_remote;
mod auth;
mod qr;
mod room;
#[cfg(test)]
mod snapshot_tests;
mod ui;
#[cfg(test)]
mod ui_test;

use std::io::Write;

use anyhow::Result;
use bili_api::{Client, ClientConfig, Quality};
use bili_live::{LiveClient, LiveEvent};
use clap::{Parser, Subcommand};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;

/// bilibili danmaku radio: listen to several live rooms at once.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Play audio on this machine via a local runner.
    /// Give the address of the runner, e.g. `127.0.0.1:7888`.
    #[arg(long, value_name = "HOST:PORT")]
    audio_remote: Option<String>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Open the TUI with the given rooms already added.
    Room {
        /// Room id, short id, or room URL. Give several to open
        /// several rooms at once.
        room_refs: Vec<String>,
        /// Play audio on this machine via a local runner.
        /// Give the address of the runner, e.g. `127.0.0.1:7888`.
        #[arg(long, value_name = "HOST:PORT")]
        audio_remote: Option<String>,
    },
    /// Run the local audio runner (on the machine with speakers).
    /// The TUI reaches it through an SSH reverse forward.
    AudioLocal {
        /// The port to listen on.
        #[arg(short, long, default_value_t = 7888)]
        port: u16,
    },
    /// Headless probe: show room info, then print danmaku lines.
    Probe {
        /// Room id, short id, or room URL.
        room_ref: String,
        /// How many danmaku lines to print before exiting.
        #[arg(short, long, default_value_t = 10)]
        n: u32,
        /// Seconds to wait for lines.
        #[arg(short, long, default_value_t = 30)]
        t: u64,
    },
    /// Log in with a QR scan, or save a pasted cookie string.
    ///
    /// Without `--cookie` the command prints a QR code for the
    /// bilibili app and waits for the scan.
    Login {
        /// A pasted cookie string, e.g. `SESSDATA=…; bili_jct=…;
        /// DedeUserID=…`. Saved without a QR scan.
        #[arg(long)]
        cookie: Option<String>,
    },
    /// Remove the saved login credentials.
    Logout,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let creds = load_credentials();
    let api = build_client(creds.as_ref());
    let logged_in = creds.as_ref().and_then(|c| c.uid());

    match cli.cmd {
        Some(Cmd::Probe { room_ref, n, t }) => probe(&api, &room_ref, n, t).await,
        Some(Cmd::AudioLocal { port }) => audio_local::run_audio_local(port).await,
        Some(Cmd::Login { cookie }) => {
            if let Some(raw) = cookie {
                let saved = auth::parse_cookie_string(&raw)?;
                auth::save(&saved)?;
                println!("credentials saved for uid {}", saved.dede_user_id);
            } else {
                run_qr_login(&api).await?;
            }
            Ok(())
        }
        Some(Cmd::Logout) => {
            auth::clear()?;
            println!("logged out");
            Ok(())
        }
        Some(Cmd::Room {
            room_refs,
            audio_remote,
        }) => {
            let (remote, remote_rx) = remote_audio(&audio_remote)?;
            let mut app = app::App::new(api, remote, remote_rx, logged_in);
            for input in room_refs {
                app.open_room(input);
            }
            run_tui(app).await
        }
        None => {
            let (remote, remote_rx) = remote_audio(&cli.audio_remote)?;
            let app = app::App::new(api, remote, remote_rx, logged_in);
            run_tui(app).await
        }
    }
}

/// Load saved credentials, warning once when the file is unreadable.
fn load_credentials() -> Option<auth::Credentials> {
    match auth::load() {
        Ok(creds) => creds,
        Err(e) => {
            eprintln!("bdr: {e}");
            None
        }
    }
}

/// Build the API client. With credentials the requests carry the
/// login cookie and fingerprint.
fn build_client(creds: Option<&auth::Credentials>) -> Client {
    let config = match creds {
        Some(c) => ClientConfig::from_login(c.cookie_string(), c.buvid3.clone(), c.buvid4.clone()),
        None => ClientConfig::builder().build(),
    };
    Client::from_config(config)
}

/// The plain-terminal QR login flow.
///
/// Prints the code and polls until the scan succeeds, the code
/// expires, or the user presses Ctrl-C.
async fn run_qr_login(api: &Client) -> Result<()> {
    let qr = api.qrcode_generate().await?;
    println!("scan with the bilibili app:");
    let out = &mut std::io::stdout();
    for line in qr::render_block(&qr.url).unwrap_or_default() {
        writeln!(out, "  {line}")?;
    }
    writeln!(out, "or open on a phone: {}", qr.url)?;
    writeln!(out, "waiting for the scan… (Ctrl-C to abort)")?;

    let mut shown: Option<bili_api::QrcodeStatus> = None;
    loop {
        let poll = tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            p = api.qrcode_poll(&qr.qrcode_key) => p,
        };
        match poll {
            Ok(p) => {
                if shown != Some(p.status) {
                    let text = match p.status {
                        bili_api::QrcodeStatus::Scanned => "scanned — confirm on the phone",
                        bili_api::QrcodeStatus::Waiting => "waiting for scan",
                        bili_api::QrcodeStatus::Success => "login confirmed",
                        bili_api::QrcodeStatus::Expired => "qr code expired",
                        _ => "unknown poll status",
                    };
                    writeln!(out, "{text}")?;
                }
                shown = Some(p.status);
                match p.status {
                    bili_api::QrcodeStatus::Success => {
                        let mut creds =
                            auth::Credentials::from_cookies(&p.cookies).ok_or_else(|| {
                                anyhow::anyhow!(
                                    "login ok, but the response carried no login cookies"
                                )
                            })?;
                        creds.refresh_token = p.refresh_token;
                        auth::save(&creds)?;
                        println!(
                            "logged in as uid {}, credentials saved to {}",
                            creds.dede_user_id,
                            auth::credentials_path().display()
                        );
                        return Ok(());
                    }
                    bili_api::QrcodeStatus::Expired => {
                        println!("run bdr login again to get a new code");
                        return Ok(());
                    }
                    _ => {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
            Err(e) => {
                writeln!(out, "poll error: {e}; retrying in 3 s")?;
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
}

/// Create the remote audio channel, when the user asked for it.
///
/// The TUI starts even if the runner is not up. The channel retries
/// in the background every three seconds. Open rooms resync when it
/// reconnects.
fn remote_audio(
    spec: &Option<String>,
) -> Result<(
    Option<audio_remote::RemoteAudio>,
    Option<tokio::sync::mpsc::UnboundedReceiver<audio_remote::RemoteMsg>>,
)> {
    let Some(spec) = spec else {
        return Ok((None, None));
    };
    let addr = audio_remote::parse_addr(spec)?;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let remote = audio_remote::RemoteAudio::supervised(addr, tx);
    Ok((Some(remote), Some(rx)))
}

/// Enter the TUI: raw mode, alternate screen, run, restore.
async fn run_tui(mut app: app::App) -> Result<()> {
    use ratatui_crossterm::CrosstermBackend;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = DefaultTerminal::new(backend)?;
    crossterm::execute!(std::io::stdout(), EnterAlternateScreen)?;
    crossterm::terminal::enable_raw_mode()?;

    let result = async {
        app.run(&mut terminal).await;
        Ok(())
    }
    .await;

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), LeaveAlternateScreen)?;
    result
}

/// A headless room check.
async fn probe(api: &Client, input: &str, n: u32, t: u64) -> Result<()> {
    let candidate = bili_api::parse_room_ref(input).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let info = api.room_info(candidate).await?;
    let out = &mut std::io::stdout();
    writeln!(out, "room_id={}", info.room_id.get())?;
    writeln!(out, "short_id={}", info.short_id)?;
    writeln!(out, "title={}", info.display_name())?;
    writeln!(
        out,
        "live={} online={}",
        info.live_status.is_live(),
        info.online
    )?;

    if !info.live_status.is_live() {
        writeln!(out, "room is offline; nothing to listen to")?;
        return Ok(());
    }

    let play = api.play_url(info.room_id, Quality::Lowest).await?;
    writeln!(out, "stream_url={}", play.url)?;

    let (tx, mut rx) = mpsc::unbounded_channel::<LiveEvent>();
    let handle = LiveClient::builder()
        .api(api.clone())
        .room(info.room_id)
        .build()
        .spawn(tx);

    let mut got = 0u32;
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(t));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            ev = rx.recv() => match ev {
                None => break,
                Some(LiveEvent::Danmaku(d)) => {
                    got += 1;
                    let text = d.text.replace('\n', " ");
                    writeln!(out, "{}: {text}", d.user)?;
                    if got >= n {
                        break;
                    }
                }
                Some(LiveEvent::Online(c)) => writeln!(out, "online={c}")?,
                Some(LiveEvent::Error(e)) => writeln!(out, "ws error: {e}")?,
                _ => {}
            },
        }
    }
    handle.stop();
    writeln!(out, "probe done, {got} danmaku lines")?;
    Ok(())
}
