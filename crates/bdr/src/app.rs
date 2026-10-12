//! The application state and main loop.

use std::time::Duration;

use bili_api::Client;
use crate::audio_remote::{RemoteAudio, RemoteMsg};
use crossterm::event::{Event as CtEvent, KeyEvent, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::room::{OpenMsg, RoomState};
use crate::ui;
use tokio::sync::mpsc;

/// The visible views of the app.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    /// The room grid.
    Grid,
    /// The add-room prompt.
    Prompt,
    /// The danmaku view for the focused room.
    Danmaku,
    /// The help screen.
    Help,
}

/// The application state.
pub struct App {
    pub rooms: Vec<RoomState>,
    /// The focused room index.
    pub focused: usize,
    /// The active view.
    pub view: View,
    /// The prompt text, for the add-room view.
    pub prompt: String,
    /// The last status or error line, shown in the status bar.
    pub status: Option<String>,
    /// The terminal size of the last draw, for key navigation.
    pub last_width: u16,
    /// Results of the async open pipeline.
    open_rx: mpsc::UnboundedReceiver<OpenMsg>,
    open_tx: mpsc::UnboundedSender<OpenMsg>,
    /// The remote audio control channel, when set.
    remote: Option<RemoteAudio>,
    /// Events from the remote audio runner.
    remote_rx: Option<mpsc::UnboundedReceiver<RemoteMsg>>,
    api: Client,
}

impl App {
    /// Create a fresh app with no rooms.
    pub fn new(
        api: Client,
        remote: Option<RemoteAudio>,
        remote_rx: Option<mpsc::UnboundedReceiver<RemoteMsg>>,
    ) -> Self {
        let (open_tx, open_rx) = mpsc::unbounded_channel();
        Self {
            rooms: Vec::new(),
            focused: 0,
            view: View::Grid,
            prompt: String::new(),
            status: None,
            last_width: 80,
            open_rx,
            open_tx,
            remote,
            remote_rx,
            api,
        }
    }

    /// The grid column count for a terminal width.
    pub fn grid_cols(width: u16) -> usize {
        let per = 28usize;
        let cols = ((width as usize).saturating_sub(4)) / per;
        cols.clamp(1, 8)
    }

    /// Request opening a room from user input.
    pub fn open_room(&mut self, input: String) {
        match bili_api::parse_room_ref(&input) {
            Ok(candidate) => {
                if self
                    .rooms
                    .iter()
                    .any(|r| r.input == input && r.resolved)
                {
                    self.status = Some("room already open".into());
                    return;
                }
                self.rooms.push(RoomState::pending(input.clone()));
                self.focused = self.rooms.len() - 1;
                let api = self.api.clone();
                let out = self.open_tx.clone();
                let remote = self.remote.as_ref();
                crate::room::spawn_open(api, candidate, input, out, remote);
            }
            Err(e) => self.status = Some(e.to_string()),
        }
    }

    /// Remove the focused room.
    pub fn close_focused(&mut self) {
        if self.rooms.is_empty() {
            return;
        }
        let idx = self.focused.min(self.rooms.len() - 1);
        let mut room = self.rooms.remove(idx);
        if let (Some(remote), Some(id)) = (&self.remote, room.room_id) {
            remote.send_stop(id.get());
        }
        room.close();
        if self.focused >= self.rooms.len() {
            self.focused = self.rooms.len().saturating_sub(1);
        }
    }

    /// Toggle mute on the focused room.
    pub fn toggle_mute(&mut self) {
        let idx = self.focused.min(self.rooms.len().saturating_sub(1));
        let Some(room) = self.rooms.get_mut(idx) else {
            return;
        };
        room.toggle_mute(!room.muted);
        if let (Some(remote), Some(id)) = (self.remote.as_ref(), room.room_id) {
            remote.send_mute(id.get(), room.muted);
        }
        let title = room.title();
        self.status = Some(format!(
            "{}: {}",
            title,
            if room.muted { "muted" } else { "unmuted" }
        ));
    }

    /// True when audio plays on a remote runner.
    pub fn is_remote_audio(&self) -> bool {
        self.remote.is_some()
    }

    /// Move the focus one step, cycling through the existing rooms.
    ///
    /// Left and right step one room in row-major order and wrap at
    /// the ends. Up and down change the row and wrap, clamped to the
    /// rooms that exist in the target row.
    pub fn move_focus(&mut self, dx: i32, dy: i32) {
        let n = self.rooms.len();
        if n == 0 {
            return;
        }
        let cols = Self::grid_cols(self.last_width).max(1) as i32;
        if dx != 0 {
            self.focused = (self.focused as i32 + dx).rem_euclid(n as i32) as usize;
            return;
        }
        let rows = (n as i32 + cols - 1) / cols;
        let cy = self.focused as i32 / cols;
        let nx = self.focused as i32 % cols;
        let ny = (cy + dy).rem_euclid(rows);
        let row_rooms = (n as i32 - ny * cols).min(cols);
        let nx = nx.min(row_rooms - 1);
        self.focused = (ny * cols + nx) as usize;
    }

    /// Apply one open-pipeline message.
    pub fn apply_open(&mut self, msg: OpenMsg) {
        match msg {
            OpenMsg::Ready {
                input,
                room_id,
                info,
                ws_handle,
                ws_rx,
                audio,
                audio_rx,
                stream_url,
                note,
            } => {
                if let Some(room) = self
                    .rooms
                    .iter_mut()
                    .find(|r| !r.resolved && r.input == input)
                {
                    room.resolved = true;
                    room.room_id = Some(room_id);
                    room.info = Some(info);
                    room.ws_handle = Some(ws_handle);
                    room.ws_rx = Some(ws_rx);
                    room.audio = audio;
                    room.audio_rx = audio_rx;
                    room.stream_url = stream_url;
                    room.note = note;
                }
            }
            OpenMsg::UpName { room_id, name } => {
                if let Some(up) = name
                    && let Some(room) =
                        self.rooms.iter_mut().find(|r| r.room_id == Some(room_id))
                {
                    room.up_name = Some(up.uname);
                }
            }
            OpenMsg::Fail { input, err } => {
                if let Some(idx) = self.rooms.iter().position(|r| !r.resolved && r.input == input) {
                    self.rooms.remove(idx);
                    if self.focused >= self.rooms.len() {
                        self.focused = self.rooms.len().saturating_sub(1);
                    }
                }
                self.status = Some(format!("open failed: {err}"));
            }
        }
    }

    /// Handle one terminal key event. Returns `false` to quit.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match &mut self.view {
            View::Grid => self.handle_grid_key(key),
            View::Prompt => self.handle_prompt_key(key),
            View::Danmaku => match key.code {
                crossterm::event::KeyCode::Esc
                | crossterm::event::KeyCode::Char('d')
                | crossterm::event::KeyCode::Char('?') => {
                    self.view = View::Grid;
                    true
                }
                _ => true,
            },
            View::Help => match key.code {
                crossterm::event::KeyCode::Esc
                | crossterm::event::KeyCode::Char('q')
                | crossterm::event::KeyCode::Char('?') => {
                    self.view = View::Grid;
                    true
                }
                _ => true,
            },
        }
    }

    fn handle_grid_key(&mut self, key: KeyEvent) -> bool {
        use crossterm::event::KeyCode;
        if key.modifiers.intersects(KeyModifiers::CONTROL) {
            if key.code == KeyCode::Char('c') {
                return false;
            }
            return true;
        }
        match key.code {
            KeyCode::Char('q') => false,
            KeyCode::Char('g') => {
                self.prompt.clear();
                self.view = View::Prompt;
                true
            }
            KeyCode::Char('x') => {
                self.close_focused();
                true
            }
            KeyCode::Char('m') => {
                self.toggle_mute();
                true
            }
            KeyCode::Char('d') => {
                if !self.rooms.is_empty() {
                    self.view = View::Danmaku;
                }
                true
            }
            KeyCode::Char('?') => {
                self.view = View::Help;
                true
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.move_focus(-1, 0);
                true
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.move_focus(1, 0);
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_focus(0, -1);
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_focus(0, 1);
                true
            }
            _ => true,
        }
    }

    fn handle_prompt_key(&mut self, key: KeyEvent) -> bool {
        use crossterm::event::KeyCode;
        // Enter arrives as \r (KeyCode::Enter) in a real terminal and as
        // \n (Ctrl+J) through a pty.
        let is_enter = key.code == KeyCode::Enter
            || (key.code == KeyCode::Char('j') && key.modifiers.contains(KeyModifiers::CONTROL));
        match key.code {
            KeyCode::Esc => {
                self.view = View::Grid;
                true
            }
            _ if is_enter => {
                let text = self.prompt.clone();
                if !text.trim().is_empty() {
                    self.open_room(text);
                }
                self.view = View::Grid;
                true
            }
            KeyCode::Backspace => {
                self.prompt.pop();
                true
            }
            KeyCode::Char(c) => {
                self.prompt.push(c);
                true
            }
            _ => true,
        }
    }

    /// Drain the async open-pipeline results.
    pub fn drain_open(&mut self) {
        while let Ok(msg) = self.open_rx.try_recv() {
            self.apply_open(msg);
        }
    }

    /// Drain the per-room event channels.
    pub fn drain_rooms(&mut self) {
        for room in &mut self.rooms {
            while let Some(ev) = room
                .ws_rx
                .as_mut()
                .and_then(|rx| rx.try_recv().ok())
            {
                room.apply_live(ev);
            }
            while let Some(ev) = room
                .audio_rx
                .as_mut()
                .and_then(|rx| rx.try_recv().ok())
            {
                room.apply_audio(ev);
            }
        }
    }

    /// Run the main loop until the user quits.
    pub async fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) {
        let poll_time = Duration::from_millis(50);
        loop {
            if let Ok(true) = crossterm::event::poll(poll_time) {
                let ev = crossterm::event::read().ok();
                if std::env::var("BDR_TRACE").is_ok() {
                    eprintln!("[bdr] event={ev:?}");
                }
                if let Some(CtEvent::Key(key)) = ev {
                    if std::env::var("BDR_TRACE").is_ok() {
                        eprintln!("[bdr] key={key:?} view={:?} rooms={}", self.view, self.rooms.len());
                    }
                    if !self.handle_key(key) {
                        break;
                    }
                }
            }
            self.drain_open();
            self.drain_rooms();
            self.drain_remote();
            let _ = ui::draw(terminal, self);
        }
        if let Some(remote) = &self.remote {
            remote.send_quit();
        }
    }

    /// Drain remote audio events into the rooms.
    pub fn drain_remote(&mut self) {
        if let Some(rx) = &mut self.remote_rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    RemoteMsg::Event { room, event } => {
                        if let Some(rs) = self
                            .rooms
                            .iter_mut()
                            .find(|r| r.room_id.map(|id| id.get() == room).unwrap_or(false))
                        {
                            rs.apply_audio(event);
                        }
                    }
                    RemoteMsg::Up => {
                        // Resync: ask the runner to play every open room.
                        if let Some(remote) = &self.remote {
                            for room in &self.rooms {
                                if let Some((id, url)) = room.resync_load() {
                                    remote.send_load(id, url);
                                }
                            }
                        }
                        self.status = None;
                    }
                    RemoteMsg::Lost => {
                        self.status = Some("audio remote not connected, retrying".into());
                    }
                }
            }
        }
    }
}

/// Re-export for the UI.
pub fn help_lines() -> Vec<&'static str> {
    vec![
        "keys (grid view)",
        "  g      add room (room id, short id, or URL)",
        "  x      close the focused room",
        "  m      mute / unmute the focused room's audio",
        "  d      show danmaku for the focused room",
        "  h/j/k/l or arrows  move focus",
        "  ?      this help",
        "  q      quit",
        "",
        "keys (prompt / help / danmaku views)",
        "  Esc    back to the grid",
        "  Enter  submit (prompt view)",
        "",
        "audio: one mpv per room; the OS mixes them.",
        "Muting a room silences only that room.",
    ]
}
