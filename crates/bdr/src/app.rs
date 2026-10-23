//! The application state and main loop.

use std::time::{Duration, Instant};

use crate::audio_remote::{RemoteAudio, RemoteMsg};
use bili_api::{Client, ClientConfig, LiveHome, QrcodeStatus, UID};
use crossterm::event::{Event as CtEvent, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::auth;
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
    /// The QR login screen.
    Login,
    /// The list of live rooms of followed livers.
    Following,
}

/// One QR login attempt, from generation to the result.
#[derive(Debug, Default)]
pub struct LoginSession {
    /// The poll key, once the session is generated.
    pub key: Option<String>,
    /// The URL to encode in the QR code, once generated.
    pub url: Option<String>,
    /// The last poll status.
    pub status: QrcodeStatus,
    /// When the next poll is due.
    pub next_poll: Option<Instant>,
    /// A generation or result error.
    pub error: Option<String>,
}

/// Messages from the QR login tasks.
#[derive(Debug)]
pub enum LoginMsg {
    /// The session is ready to show.
    Ready { url: String, key: String },
    /// QR generation failed.
    Failed(String),
    /// One poll finished.
    Polled {
        status: QrcodeStatus,
        cookies: Vec<(String, String)>,
        refresh_token: Option<String>,
    },
    /// A poll attempt failed; poll again at `next`.
    PollFailed { next: Instant },
}

/// The live-home view state.
#[derive(Debug, Default)]
pub struct Following {
    /// The rooms, once fetched.
    pub home: Option<LiveHome>,
    /// The selection index into the combined room list.
    pub sel: usize,
    /// A fetch error.
    pub error: Option<String>,
}

/// Messages from the live-home fetch task.
#[derive(Debug)]
pub enum FollowingMsg {
    /// The rooms arrived.
    Loaded(LiveHome),
    /// The fetch failed.
    Failed(String),
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
    /// The terminal width of the last draw, for key navigation.
    pub last_width: u16,
    /// Results of the async open pipeline.
    open_rx: mpsc::UnboundedReceiver<OpenMsg>,
    open_tx: mpsc::UnboundedSender<OpenMsg>,
    /// The remote audio control channel, when set.
    remote: Option<RemoteAudio>,
    /// Events from the remote audio runner.
    remote_rx: Option<mpsc::UnboundedReceiver<RemoteMsg>>,
    /// The in-flight QR login, when the login view is open.
    pub login: Option<LoginSession>,
    /// Events from the login tasks.
    login_rx: mpsc::UnboundedReceiver<LoginMsg>,
    login_tx: mpsc::UnboundedSender<LoginMsg>,
    /// The live-home view state, when the view is open.
    pub following: Option<Following>,
    /// Events from the live-home fetch task.
    following_rx: mpsc::UnboundedReceiver<FollowingMsg>,
    following_tx: mpsc::UnboundedSender<FollowingMsg>,
    /// The user id, when a login is active.
    pub logged_in: Option<UID>,
    api: Client,
}

impl App {
    /// Create a fresh app with no rooms.
    pub fn new(
        api: Client,
        remote: Option<RemoteAudio>,
        remote_rx: Option<mpsc::UnboundedReceiver<RemoteMsg>>,
        logged_in: Option<UID>,
    ) -> Self {
        let (open_tx, open_rx) = mpsc::unbounded_channel();
        let (login_tx, login_rx) = mpsc::unbounded_channel();
        let (following_tx, following_rx) = mpsc::unbounded_channel();
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
            login: None,
            login_rx,
            login_tx,
            following: None,
            following_rx,
            following_tx,
            logged_in,
            api,
        }
    }

    /// The number of rooms in one grid row.
    ///
    /// The count rule: three rooms or fewer sit in one row.
    /// Beyond that, rows of three begin. A width-based cap shrinks
    /// the row when the terminal is narrow. Each cell wants 28
    /// columns.
    pub fn grid_cols(rooms: usize, width: u16) -> usize {
        let count_cap = rooms.clamp(1, 3);
        let width_cap = ((width as usize).saturating_sub(4)) / 28;
        count_cap.min(width_cap).max(1)
    }

    /// Request opening a room from user input.
    pub fn open_room(&mut self, input: String) {
        match bili_api::parse_room_ref(&input) {
            Ok(candidate) => {
                if self.rooms.iter().any(|r| r.input == input && r.resolved) {
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

    /// The API client, for tests.
    #[cfg(test)]
    pub fn api(&self) -> &Client {
        &self.api
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
        let cols = Self::grid_cols(n, self.last_width) as i32;
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
                    && let Some(room) = self.rooms.iter_mut().find(|r| r.room_id == Some(room_id))
                {
                    room.up_name = Some(up.uname);
                }
            }
            OpenMsg::Fail { input, err } => {
                if let Some(idx) = self
                    .rooms
                    .iter()
                    .position(|r| !r.resolved && r.input == input)
                {
                    self.rooms.remove(idx);
                    if self.focused >= self.rooms.len() {
                        self.focused = self.rooms.len().saturating_sub(1);
                    }
                }
                self.status = Some(format!("open failed: {err}"));
            }
        }
    }

    /// Open the QR login view and start generating a code.
    pub fn open_login(&mut self) {
        self.view = View::Login;
        self.login = Some(LoginSession::default());
        let api = self.api.clone();
        let out = self.login_tx.clone();
        tokio::spawn(async move {
            match api.qrcode_generate().await {
                Ok(qr) => {
                    let _ = out.send(LoginMsg::Ready {
                        url: qr.url,
                        key: qr.qrcode_key,
                    });
                }
                Err(e) => {
                    let _ = out.send(LoginMsg::Failed(e.to_string()));
                }
            }
        });
    }

    /// Open the live-home view and fetch the followed rooms.
    pub fn open_following(&mut self) {
        self.view = View::Following;
        self.following = Some(Following::default());
        self.fetch_following();
    }

    /// Fetch the live-home rooms into the current view state.
    fn fetch_following(&mut self) {
        let f = match &mut self.following {
            Some(f) => f,
            None => return,
        };
        f.home = None;
        f.error = None;
        f.sel = 0;
        let api = self.api.clone();
        let out = self.following_tx.clone();
        tokio::spawn(async move {
            match api.live_home().await {
                Ok(home) => {
                    let _ = out.send(FollowingMsg::Loaded(home));
                }
                Err(e) => {
                    let _ = out.send(FollowingMsg::Failed(e.to_string()));
                }
            }
        });
    }

    /// Open the selected live-home room and return to the grid.
    pub fn confirm_following(&mut self) {
        let sel = match &self.following {
            Some(f) => f.sel,
            None => return,
        };
        let entries = self.following_entries();
        let Some(entry) = entries.get(sel) else {
            return;
        };
        let room_id = entry.room_id.get().to_string();
        self.open_room(room_id);
        self.view = View::Grid;
    }

    /// Move the live-home selection one row down or up.
    pub fn move_following_focus(&mut self, dy: i32) {
        let f = match &mut self.following {
            Some(f) => f,
            None => return,
        };
        let home = match &f.home {
            Some(home) => home,
            None => return,
        };
        let n = home.followed.len() + home.recommended.len();
        if n == 0 {
            return;
        }
        f.sel = ((f.sel as i32) + dy).clamp(0, (n - 1) as i32) as usize;
    }

    /// The combined live-home room list, in display order:
    /// followed rooms first, then recommended rooms.
    pub fn following_entries(&self) -> Vec<&bili_api::LiveRoomEntry> {
        let home = match self.following.as_ref().and_then(|f| f.home.as_ref()) {
            Some(home) => home,
            None => return Vec::new(),
        };
        home.followed
            .iter()
            .chain(home.recommended.iter())
            .collect()
    }

    /// Poll the QR login when the timer is due.
    fn tick_login(&mut self) {
        let Some(session) = self.login.as_mut() else {
            return;
        };
        if matches!(
            session.status,
            QrcodeStatus::Success | QrcodeStatus::Expired
        ) {
            return;
        }
        let key = match &session.key {
            Some(k) => k.clone(),
            None => return,
        };
        let due = match session.next_poll {
            Some(t) => t,
            None => return,
        };
        if Instant::now() < due {
            return;
        }
        session.next_poll = None;
        let api = self.api.clone();
        let out = self.login_tx.clone();
        tokio::spawn(async move {
            match api.qrcode_poll(&key).await {
                Ok(poll) => {
                    let _ = out.send(LoginMsg::Polled {
                        status: poll.status,
                        cookies: poll.cookies,
                        refresh_token: poll.refresh_token,
                    });
                }
                Err(_) => {
                    // Transient failure. Poll again in two seconds.
                    let _ = out.send(LoginMsg::PollFailed {
                        next: Instant::now() + Duration::from_secs(2),
                    });
                }
            }
        });
    }

    /// Apply one login message.
    pub fn apply_login(&mut self, msg: LoginMsg) {
        let session = match &mut self.login {
            Some(s) => s,
            None => return,
        };
        match msg {
            LoginMsg::Ready { url, key } => {
                session.key = Some(key);
                session.url = Some(url);
                session.status = QrcodeStatus::Waiting;
                session.error = None;
                session.next_poll = Some(Instant::now() + LOGIN_POLL_INTERVAL);
            }
            LoginMsg::Failed(e) => {
                session.error = Some(e);
            }
            LoginMsg::Polled {
                status,
                cookies,
                refresh_token,
            } => {
                session.status = status;
                match status {
                    QrcodeStatus::Success => {
                        self.finish_login(cookies, refresh_token);
                    }
                    QrcodeStatus::Expired => {
                        // The view offers a refresh. No more polls.
                    }
                    _ => {
                        session.next_poll = Some(Instant::now() + LOGIN_POLL_INTERVAL);
                    }
                }
            }
            LoginMsg::PollFailed { next } => {
                session.next_poll = Some(next);
            }
        }
    }

    /// A successful login: save the credentials and rebuild the
    /// client so that the later requests carry the session.
    fn finish_login(&mut self, cookies: Vec<(String, String)>, refresh_token: Option<String>) {
        let mut creds = match auth::Credentials::from_cookies(&cookies) {
            Some(c) => c,
            None => {
                if let Some(s) = &mut self.login {
                    s.error = Some("login ok, but no login cookies in the response".into());
                }
                return;
            }
        };
        creds.refresh_token = refresh_token;
        let uid = creds.uid();
        let config = ClientConfig::from_login(
            creds.cookie_string(),
            creds.buvid3.clone(),
            creds.buvid4.clone(),
        );
        self.api = Client::from_config(config);
        self.logged_in = uid;
        match auth::save(&creds) {
            Ok(()) => {
                self.status = Some(match uid {
                    Some(u) => format!("logged in as uid {}", u.get()),
                    None => "logged in".into(),
                });
            }
            Err(e) => {
                self.status = Some(format!("logged in, but saving failed: {e}"));
            }
        }
        self.login = None;
        self.view = View::Grid;
    }

    /// Drain the login task results.
    pub fn drain_login(&mut self) {
        while let Ok(msg) = self.login_rx.try_recv() {
            self.apply_login(msg);
        }
    }

    /// Drain the live-home task results.
    pub fn drain_following(&mut self) {
        while let Ok(msg) = self.following_rx.try_recv() {
            let f = match &mut self.following {
                Some(f) => f,
                None => continue,
            };
            match msg {
                FollowingMsg::Loaded(home) => f.home = Some(home),
                FollowingMsg::Failed(e) => f.error = Some(e),
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
            View::Login => self.handle_login_key(key),
            View::Following => self.handle_following_key(key),
        }
    }

    /// Keys for the QR login view.
    fn handle_login_key(&mut self, key: KeyEvent) -> bool {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Esc => {
                self.view = View::Grid;
                self.login = None;
                true
            }
            KeyCode::Char('r') => {
                self.open_login();
                true
            }
            _ => true,
        }
    }

    /// Keys for the live-home view.
    fn handle_following_key(&mut self, key: KeyEvent) -> bool {
        use crossterm::event::KeyCode;
        let is_enter = key.code == KeyCode::Enter
            || (key.code == KeyCode::Char('j') && key.modifiers.contains(KeyModifiers::CONTROL));
        match key.code {
            KeyCode::Esc => {
                self.view = View::Grid;
                true
            }
            _ if is_enter => {
                self.confirm_following();
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_following_focus(1);
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_following_focus(-1);
                true
            }
            KeyCode::Char('r') => {
                self.fetch_following();
                true
            }
            _ => true,
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
            KeyCode::Char('f') => {
                self.open_following();
                true
            }
            KeyCode::Char('L') => {
                self.open_login();
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
            while let Some(ev) = room.ws_rx.as_mut().and_then(|rx| rx.try_recv().ok()) {
                room.apply_live(ev);
            }
            while let Some(ev) = room.audio_rx.as_mut().and_then(|rx| rx.try_recv().ok()) {
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
                        eprintln!(
                            "[bdr] key={key:?} view={:?} rooms={}",
                            self.view,
                            self.rooms.len()
                        );
                    }
                    if !self.handle_key(key) {
                        break;
                    }
                }
            }
            self.drain_open();
            self.drain_rooms();
            self.drain_remote();
            self.drain_login();
            self.drain_following();
            self.tick_login();
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

/// The QR login poll interval.
const LOGIN_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Re-export for the UI.
pub fn help_lines() -> Vec<&'static str> {
    vec![
        "keys (grid view)",
        "  g      add room (room id, short id, or URL)",
        "  x      close the focused room",
        "  m      mute / unmute the focused room's audio",
        "  d      show danmaku for the focused room",
        "  f      live rooms of followed livers",
        "  L      QR login (when logged out)",
        "  h/j/k/l or arrows  move focus",
        "  ?      this help",
        "  q      quit",
        "",
        "keys (prompt / help / danmaku views)",
        "  Esc    back to the grid",
        "  Enter  submit (prompt view)",
        "",
        "keys (following view)",
        "  j/k or arrows  move the selection",
        "  Enter  open the selected room",
        "  r      refresh the list",
        "  Esc    back to the grid",
        "",
        "keys (login view)",
        "  r      refresh the QR code",
        "  Esc    back to the grid",
        "",
        "audio: one mpv per room; the OS mixes them.",
        "Muting a room silences only that room.",
        "",
        "login: credentials live in",
        "~/.config/bilibili-dd-radio/credentials.toml",
    ]
}
