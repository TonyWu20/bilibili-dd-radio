//! bdr: listen to several bilibili live rooms at once.
//!
//! The default command opens the TUI. Use `bdr probe` to check a room
//! headlessly.

mod app;
mod audio_local;
mod audio_remote;
mod room;
mod ui;
#[cfg(test)]
mod ui_test;
#[cfg(test)]
mod snapshot_tests;

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
    /// Open the TUI with one room already added.
    Room {
        /// Room id, short id, or room URL.
        room_ref: String,
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let api = Client::from_config(ClientConfig::builder().build());

    match cli.cmd {
        Some(Cmd::Probe { room_ref, n, t }) => probe(&api, &room_ref, n, t).await,
        Some(Cmd::AudioLocal { port }) => audio_local::run_audio_local(port).await,
        Some(Cmd::Room { room_ref, audio_remote }) => {
            let (remote, remote_rx) = remote_audio(&audio_remote)?;
            let mut app = app::App::new(api, remote, remote_rx);
            app.open_room(room_ref);
            run_tui(app).await
        }
        None => {
            let (remote, remote_rx) = remote_audio(&cli.audio_remote)?;
            let app = app::App::new(api, remote, remote_rx);
            run_tui(app).await
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
) -> Result<(Option<audio_remote::RemoteAudio>, Option<tokio::sync::mpsc::UnboundedReceiver<audio_remote::RemoteMsg>>)> {
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
    let candidate = bili_api::parse_room_ref(input)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let info = api.room_info(candidate).await?;
    let out = &mut std::io::stdout();
    writeln!(out, "room_id={}", info.room_id.get())?;
    writeln!(out, "short_id={}", info.short_id)?;
    writeln!(out, "title={}", info.display_name())?;
    writeln!(out, "live={} online={}", info.live_status.is_live(), info.online)?;

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
