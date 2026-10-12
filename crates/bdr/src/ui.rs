//! Rendering.


use ratatui::backend::Backend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};
use ratatui::{Frame, Terminal};

use crate::app::{help_lines, App, View};
use crate::room::{AudioState, Line as RoomLine};

/// Draw one frame on any backend.
pub fn draw<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<(), B::Error> {
    let size = terminal.size()?;
    app.last_width = size.width;
    terminal.draw(|f| {
        let area = f.area();
        let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(1)])
            .split(area);
        match app.view {
            View::Grid => draw_grid(f, app, chunks[0]),
            View::Prompt => {
                draw_grid(f, app, chunks[0]);
                draw_prompt(f, app, chunks[0]);
            }
            View::Danmaku => draw_danmaku(f, app, chunks[0]),
            View::Help => draw_help(f, chunks[0]),
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
        f.render_widget(Paragraph::new(text).block(Block::bordered().title(" rooms ")), area);
        return;
    }
    let cols = App::grid_cols(area.width);
    let rows = app.rooms.len().div_ceil(cols);
    let cell_h = (area.height / rows as u16).clamp(5, 7);

    // Build the row rects by hand so the last row can grow to fill.
    let mut rows_layout: Vec<Rect> = Vec::new();
    let mut y = area.y;
    for r in 0..rows {
        let h = if r == rows - 1 {
            (area.y + area.height).saturating_sub(y)
        } else {
            cell_h
        };
        rows_layout.push(Rect::new(area.x, y, area.width, h));
        y += h;
    }

    for (row_idx, row_area) in rows_layout.iter().enumerate() {
        let cell_constraints: Vec<Constraint> = (0..cols)
            .map(|_| Constraint::Min(28))
            .collect();
        let cells: Vec<Rect> = Layout::horizontal(cell_constraints)
            .direction(Direction::Horizontal)
            .split(*row_area)
            .to_vec();
        for (col_idx, cell_area) in cells.iter().enumerate() {
            let idx = row_idx * cols + col_idx;
            if idx >= app.rooms.len() {
                break;
            }
            let focused = idx == app.focused && app.view == View::Grid;
            draw_cell(f, &app.rooms[idx], *cell_area, focused);
        }
    }
}

/// One room cell.
fn draw_cell(f: &mut Frame, room: &crate::room::RoomState, area: Rect, focused: bool) {
    let live = room
        .info
        .as_ref()
        .map(|i| i.live_status.is_live())
        .unwrap_or(false);
    let dot = if live { "●" } else { "○" };
    let title = room.title();
    let owner = room.owner();
    let online = room.online.map_or_else(
        || room.info.as_ref().map(|i| i.online).unwrap_or(0).to_string(),
        |n| n.to_string(),
    );
    let audio = audio_text(&room.audio_state, room.muted);

    let mut spans: Vec<Line> = Vec::new();
    spans.push(Line::raw(format!("{dot} {}", crate::room::truncate(&title, 24))));
    spans.push(Line::raw(format!("up {}", crate::room::truncate(&owner, 24))));
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

    let block = Block::bordered()
        .title(crate::room::truncate(
            &room.input,
            20,
        ));
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
    let Some(room) = app.rooms.get(app.focused.min(app.rooms.len().saturating_sub(1)))
        .filter(|_| !app.rooms.is_empty())
    else {
        return;
    };
    let mut lines: Vec<Line> = Vec::new();
    for line in room.last_lines(60) {
        lines.push(match line {
            RoomLine::Danmaku { user, text } => Line::raw(format!(
                "{user} {}",
                crate::room::truncate(text, 60)
            )),
            RoomLine::Gift { user, gift, num } => {
                Line::raw(format!("[gift] {user} x{num} {gift}"))
            }
            RoomLine::System(s) => Line::raw(format!("* {s}")),
        });
    }
    let title = format!(
        "danmaku: {} — {} [audio: {}]",
        room.title(),
        room.owner(),
        audio_text(&room.audio_state, room.muted)
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
    let block = Block::bordered()
        .title(" add room: id, short id, or URL · Enter add · Esc cancel ");
    let text = Line::from(vec![
        Span::raw(format!(" {prompt}", prompt = app.prompt)),
        Span::styled("▌", Style::default().fg(Color::Yellow)),
    ]);
    f.render_widget(
        Paragraph::new(text)
            .block(block)
            .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
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

/// The bottom status bar.
fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let mut status = app
        .status
        .as_deref()
        .map(|s| format!(" {s}"))
        .unwrap_or_else(|| " q quit · g add · m mute · d danmaku · x close · ? help ".into());
    if app.is_remote_audio() {
        status = format!("{status} · remote audio");
    }
    let text = Line::raw(format!(
        " rooms: {} |{status}",
        app.rooms.len()
    ));
    f.render_widget(
        Paragraph::new(text).style(Style::new().add_modifier(Modifier::BOLD)),
        area,
    );
}
