//! Snapshot tests for the TUI using ratatui `TestBackend` + `insta`.
//!
//! Follows the official ratatui recipe
//! <https://ratatui.rs/recipes/testing/snapshots/> and the method in
//! `rushi-dev/rushi-tui/docs/tui-insta-snapshot-testing.md`.
//!
//! Each test drives the `App` into a concrete user-visible state,
//! renders one frame onto a `TestBackend`, and snapshots the
//! resulting terminal grid.
//!
//! Generate or review snapshots with:
//! ```sh
//! cargo insta test -p bdr && cargo insta review
//! ```

#![cfg(test)]

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::app::{App, View};
use crate::audio_remote::RemoteAudio;
use bili_audio::AudioEvent;

use crate::room::{AudioState, Line, RoomState};
use crate::ui;

const W: u16 = 120;
const H: u16 = 30;

// ── shared harness ──────────────────────────────────────────────────

fn render(app: &mut App) -> String {
    let backend = TestBackend::new(W, H);
    let mut term = Terminal::new(backend).expect("test backend");
    let _ = ui::draw(&mut term, app);
    term.backend().to_string()
}

fn api() -> bili_api::Client {
    bili_api::Client::from_config(bili_api::ClientConfig::builder().build())
}

fn room_with(input: &str) -> RoomState {
    let mut room = RoomState::pending(input.to_string());
    room.resolved = true;
    room.room_id = Some(bili_api::RoomID(1234));
    room.info = Some(bili_api::RoomInfo {
        room_id: bili_api::RoomID(1234),
        short_id: 6,
        uid: bili_api::UID(42),
        title: "my stream".into(),
        parent_area_name: String::new(),
        live_status: bili_api::LiveStatus::LIVE,
        online: 42,
    });
    room.up_name = Some("Streamer".into());
    room
}

fn app_with_room() -> App {
    let mut app = App::new(api(), None, None);
    app.last_width = W;
    app.rooms.push(room_with("6"));
    app
}

// ── grid ────────────────────────────────────────────────────────────

#[test]
fn snap_empty_grid() {
    let mut app = App::new(api(), None, None);
    app.last_width = W;
    let out = render(&mut app);
    insta::assert_snapshot!(out);
}

#[test]
fn snap_grid_one_room() {
    let mut app = app_with_room();
    let room = &mut app.rooms[0];
    room.lines.push_back(Line::Danmaku {
        user: "alice".into(),
        text: "hello world".into(),
    });
    room.lines.push_back(Line::Gift {
        user: "bob".into(),
        gift: "chili".into(),
        num: 3,
    });
    let out = render(&mut app);
    insta::assert_snapshot!(out);
}

#[test]
fn snap_grid_two_rooms_wrapped_focus() {
    let mut app = app_with_room();
    app.rooms.push(room_with("7"));
    app.focused = 1;
    app.move_focus(1, 0);
    assert_eq!(app.focused, 0, "l on the last room wraps to the first");
    let out = render(&mut app);
    insta::assert_snapshot!(out);
}

#[test]
fn snap_room_muted() {
    let mut app = app_with_room();
    let room = &mut app.rooms[0];
    room.muted = true;
    room.audio_state = AudioState::Playing;
    let out = render(&mut app);
    insta::assert_snapshot!(out);
}

#[test]
fn snap_room_audio_gone() {
    let mut app = app_with_room();
    let room = &mut app.rooms[0];
    room.apply_audio(AudioEvent::Gone("mpv: not found".into()));
    assert_eq!(room.audio_state, AudioState::Dead("mpv: not found".into()));
    let out = render(&mut app);
    insta::assert_snapshot!(out);
}

// ── prompt ──────────────────────────────────────────────────────────

#[test]
fn snap_prompt_with_input() {
    let mut app = app_with_room();
    app.view = View::Prompt;
    app.prompt = "7734200".into();
    let out = render(&mut app);
    insta::assert_snapshot!(out);
}

// ── danmaku view ────────────────────────────────────────────────────

#[test]
fn snap_danmaku_view() {
    let mut app = app_with_room();
    let room = &mut app.rooms[0];
    for i in 0..5 {
        room.lines.push_back(Line::Danmaku {
            user: format!("user{i}"),
            text: format!("line {i}"),
        });
    }
    room.lines.push_back(Line::System("welcome".into()));
    app.view = View::Danmaku;
    let out = render(&mut app);
    insta::assert_snapshot!(out);
}

// ── help ────────────────────────────────────────────────────────────

#[test]
fn snap_help_view() {
    let mut app = app_with_room();
    app.view = View::Help;
    let out = render(&mut app);
    insta::assert_snapshot!(out);
}

// ── status bar ──────────────────────────────────────────────────────

#[test]
fn snap_status_mute_hint() {
    let mut app = app_with_room();
    app.toggle_mute();
    assert_eq!(app.rooms[0].muted, true);
    let out = render(&mut app);
    insta::assert_snapshot!(out);
}

#[tokio::test]
async fn snap_status_remote_retrying() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let remote = RemoteAudio::supervised("127.0.0.1:1".parse().unwrap(), tx);
    let mut app = App::new(api(), Some(remote), None);
    app.last_width = W;
    app.rooms.push(room_with("6"));
    app.status = Some("audio remote not connected, retrying".into());
    let out = render(&mut app);
    insta::assert_snapshot!(out);
}
