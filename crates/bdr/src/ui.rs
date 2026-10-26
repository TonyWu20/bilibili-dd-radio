//! Rendering.

use ratatui::backend::Backend;
use ratatui::layout::Alignment;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::{Block, Paragraph};
use ratatui::{Frame, Terminal};

use bili_api::QrcodeStatus;

use crate::app::{App, LoginSession, View, help_lines};
use crate::qr;
use crate::room::AudioState;

/// Draw one frame on any backend.
pub fn draw<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<(), B::Error> {
    let size = terminal.size()?;
    app.last_width = size.width;
    app.last_height = size.height;
    terminal.draw(|f| {
        let area = f.area();
        let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(area);
        match app.view {
            View::Grid => draw_grid(f, app, chunks[0]),
            View::Prompt => {
                draw_grid(f, app, chunks[0]);
                draw_prompt(f, app, chunks[0]);
            }
            View::Danmaku => draw_danmaku(f, app, chunks[0]),
            View::Help => draw_help(f, chunks[0]),
            View::Login => draw_login(f, app, chunks[0]),
            View::Following => draw_following(f, app, chunks[0]),
        }
        draw_status(f, app, chunks[1]);
    })?;
    Ok(())
}

/// The room grid.
fn draw_grid(f: &mut Frame, app: &App, area: Rect) {
    if app.rooms.is_empty() {
        let text = vec![
            Line::raw("no rooms"),
            Line::raw("press g to add a room: id, short id, or room URL"),
        ];
        f.render_widget(
            Paragraph::new(text).block(Block::bordered().title(" rooms ")),
            area,
        );
        return;
    }
    let n = app.rooms.len();
    let video = matches!(app.view, View::Grid | View::Prompt);
    // The same cell rects the video geometry reads. They tile the
    // area from its origin; the chunks start at the origin.
    let cells = App::grid_cells(n, area.width, area.height);
    for (idx, (cx, cy, cw, ch)) in cells.iter().enumerate() {
        let cell = Rect::new(*cx, *cy, *cw, *ch);
        let focused = idx == app.focused && app.view == View::Grid;
        draw_cell(f, &app.rooms[idx], cell, focused, room_wants_video(&app.rooms[idx], video));
    }
}

/// The cell of a video room stays blank. The kitty image renders
/// above the text.
fn room_wants_video(room: &crate::room::RoomState, view_shows_grid: bool) -> bool {
    room.video && view_shows_grid
}

/// One room cell.
fn draw_cell(
    f: &mut Frame,
    room: &crate::room::RoomState,
    area: Rect,
    focused: bool,
    video: bool,
) {
    let live = room
        .info
        .as_ref()
        .map(|i| i.live_status.is_live())
        .unwrap_or(false);
    let dot = if live { "●" } else { "○" };
    let title = room.title();
    let owner = room.owner();
    let online = room.online.map_or_else(
        || {
            room.info
                .as_ref()
                .map(|i| i.online)
                .unwrap_or(0)
                .to_string()
        },
        |n| n.to_string(),
    );
    let audio = audio_text(&room.audio_state, room.muted);

    let mut spans: Vec<Line> = Vec::new();
    spans.push(Line::raw(format!(
        "{dot} {}",
        crate::room::truncate(&title, 24)
    )));
    spans.push(Line::raw(format!(
        "up {}",
        crate::room::truncate(&owner, 24)
    )));
    spans.push(Line::raw(format!("on {online} {audio}")));
    let ticker = if room.note.is_some() { 2 } else { 3 };
    if let Some(note) = &room.note {
        spans.push(Line::raw(crate::room::truncate(note, 26)));
    }
    for line in room.last_lines(ticker) {
        spans.push(Line::raw(line.short(26)));
    }
    while spans.len() < 7 {
        spans.push(Line::raw(""));
    }

    let block = Block::bordered().title(crate::room::truncate(&room.input, 20));
    if video {
        // The image covers the cell interior. Only the border and
        // the title stay visible.
        let block = if focused {
            block.style(Style::new().add_modifier(Modifier::BOLD))
        } else {
            block
        };
        f.render_widget(block, area);
        return;
    }
    if focused {
        f.render_widget(
            Paragraph::new(spans).block(block.style(Style::new().add_modifier(Modifier::BOLD))),
            area,
        );
    } else {
        f.render_widget(Paragraph::new(spans).block(block), area);
    }
}

/// The one-word audio state shown in a cell.
fn audio_text(state: &AudioState, muted: bool) -> &'static str {
    if muted {
        return "muted";
    }
    match state {
        AudioState::Unknown => "audio:…",
        AudioState::Playing => "playing",
        AudioState::Idle => "idle",
        AudioState::Ended => "ended",
        AudioState::Dead(_) => "gone",
    }
}

/// The danmaku view for the focused room.
fn draw_danmaku(f: &mut Frame, app: &App, area: Rect) {
    let Some(room) = app
        .rooms
        .get(app.focused.min(app.rooms.len().saturating_sub(1)))
        .filter(|_| !app.rooms.is_empty())
    else {
        return;
    };
    let mut lines: Vec<Line> = Vec::new();
    // One renderer for both views. The grid trims to 26 chars.
    // The danmaku view trims to 60.
    for line in room.last_lines(60) {
        lines.push(Line::raw(line.short(60)));
    }
    let video_tag = if room.video { " · video" } else { "" };
    let title = format!(
        "danmaku: {} — {} [audio: {}{}]",
        room.title(),
        room.owner(),
        audio_text(&room.audio_state, room.muted),
        video_tag
    );
    let para = Paragraph::new(lines).block(
        Block::bordered()
            .title(crate::room::truncate(&title, 40))
            .title_bottom(Line::raw("Esc to go back")),
    );
    f.render_widget(para, area);
}

/// The prompt view input box.
fn draw_prompt(f: &mut Frame, app: &App, area: Rect) {
    let h = area.height;
    if h < 4 {
        return;
    }
    let y = area.y + h - 3;
    let input_rect = Rect::new(area.x, y, area.width, 3);
    let block =
        Block::bordered().title(" add room: id, short id, or URL · Enter add · Esc cancel ");
    let text = Line::from(vec![
        Span::raw(format!(" {prompt}", prompt = app.prompt)),
        Span::styled("▌", Style::default().fg(Color::Yellow)),
    ]);
    f.render_widget(
        Paragraph::new(text).block(block).style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        input_rect,
    );
}

/// The help view.
fn draw_help(f: &mut Frame, area: Rect) {
    let text: Vec<Line> = help_lines().into_iter().map(Line::raw).collect();
    f.render_widget(
        Paragraph::new(text).block(Block::bordered().title(" help ")),
        area,
    );
}

/// The QR login view.
fn draw_login(f: &mut Frame, app: &App, area: Rect) {
    let session = match &app.login {
        Some(s) => s,
        None => {
            let text = vec![Line::raw("logged in")];
            f.render_widget(
                Paragraph::new(text)
                    .block(Block::bordered().title(" login "))
                    .alignment(Alignment::Center),
                area,
            );
            return;
        }
    };
    let mut lines: Vec<Line> = Vec::new();
    match &session.url {
        Some(url) => {
            match qr::render_block(url) {
                Some(rows) => {
                    for row in rows {
                        lines.push(Line::raw(row));
                    }
                }
                None => lines.push(Line::raw("cannot build the QR code")),
            }
            lines.push(Line::raw(""));
            lines.push(Line::raw(login_status_text(session)));
        }
        None => {
            lines.push(Line::raw("generating the QR code…"));
            if let Some(e) = &session.error {
                lines.push(Line::raw(format!("error: {e}")));
            }
        }
    }
    lines.push(Line::raw("r refresh · Esc back"));
    let block =
        Block::bordered().title(" login: scan with the bilibili app · credentials saved to disk ");
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Center),
        area,
    );
}

/// One status line for the login view.
fn login_status_text(session: &LoginSession) -> String {
    if let Some(e) = &session.error {
        return format!("error: {e} (press r to retry)");
    }
    match session.status {
        QrcodeStatus::Waiting => "waiting for the scan".to_string(),
        QrcodeStatus::Scanned => "scanned — confirm the login on the phone".to_string(),
        QrcodeStatus::Success => "logged in".to_string(),
        QrcodeStatus::Expired => "QR code expired — press r for a new one".to_string(),
        QrcodeStatus::Unknown(code) => format!("unknown poll status {code}"),
    }
}

/// The live-home view: followed rooms, then recommended rooms.
fn draw_following(f: &mut Frame, app: &App, area: Rect) {
    let f_state = match &app.following {
        Some(s) => s,
        None => {
            let text = vec![Line::raw("loading…")];
            f.render_widget(
                Paragraph::new(text)
                    .block(follow_block())
                    .alignment(Alignment::Center),
                area,
            );
            return;
        }
    };
    let mut lines: Vec<Line> = Vec::new();
    if let Some(e) = &f_state.error {
        lines.push(Line::raw(format!("fetch failed: {e} (press r to retry)")));
    } else if let Some(home) = &f_state.home {
        render_following_home(&mut lines, home, app, f_state.sel);
    } else {
        lines.push(Line::raw("loading…"));
    }
    f.render_widget(Paragraph::new(lines).block(follow_block()), area);
}

fn follow_block() -> Block<'static> {
    Block::bordered().title(
        " following: live rooms of the livers you follow · Enter open · r refresh · Esc back ",
    )
}

/// Fill `lines` with the live-home rows. The selected room row is
/// reversed so it stands out.
fn render_following_home(lines: &mut Vec<Line>, home: &bili_api::LiveHome, app: &App, sel: usize) {
    if app.logged_in.is_none() {
        lines.push(Line::styled(
            "not logged in — press L in the grid to log in; showing recommended rooms",
            Style::default().fg(Color::Yellow),
        ));
    }
    if home.followed.is_empty() {
        lines.push(Line::styled(
            "no followed livers are live",
            Style::default().fg(Color::DarkGray),
        ));
    }
    let mut idx = 0usize;
    for e in &home.followed {
        push_room_row(lines, e, idx == sel, "  ");
        idx += 1;
    }
    if !home.followed.is_empty() && !home.recommended.is_empty() {
        lines.push(Line::styled(
            "recommended",
            Style::default().fg(Color::DarkGray),
        ));
    }
    for e in &home.recommended {
        push_room_row(lines, e, idx == sel, "  ");
        idx += 1;
    }
    if home.followed.is_empty() && home.recommended.is_empty() {
        lines.push(Line::raw("no live rooms"));
    }
}

/// One room row: `uname · title · on N`, reversed when selected.
fn push_room_row(lines: &mut Vec<Line>, e: &bili_api::LiveRoomEntry, selected: bool, prefix: &str) {
    let text = format!(
        "{}{} · {} · on {}",
        if selected { "▶ " } else { prefix },
        bili_api_room_label(e),
        crate::room::truncate(&e.title, 60),
        e.online
    );
    let line = if selected {
        Line::styled(text, Style::default().add_modifier(Modifier::REVERSED))
    } else {
        Line::raw(text)
    };
    lines.push(line);
}

/// The row label: the UP name, or the uid when the name is absent.
fn bili_api_room_label(e: &bili_api::LiveRoomEntry) -> String {
    if e.uname.is_empty() {
        format!("uid:{}", e.uid.get())
    } else {
        e.uname.clone()
    }
}

/// The bottom status bar.
fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let mut status = app
        .status
        .as_deref()
        .map(|s| format!(" {s}"))
        .unwrap_or_else(|| {
            " q quit · g add · f follows · L login · m mute · v video · d danmaku · x close · ? help ".into()
        });
    if app.is_remote_audio() {
        status = format!("{status} · remote audio");
    }
    if let Some(u) = app.logged_in {
        status = format!("{status} · uid {}", u.get());
    }
    let text = Line::raw(format!(" rooms: {} |{status}", app.rooms.len()));
    f.render_widget(
        Paragraph::new(text).style(Style::new().add_modifier(Modifier::BOLD)),
        area,
    );
}
