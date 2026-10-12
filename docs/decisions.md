# Decision Log

Record of user decisions and technical findings. Newest first.

## 2026-11-04 session 1

- Language and UI: Rust, `ratatui` TUI.
- Product: listen to multiple bilibili live rooms together. Audio is mixed in the OS audio stack.
  Each room can be muted. Rooms form a dynamic growing grid.
- Danmaku: user can receive danmaku of each room. After OAuth login, user can send danmaku.
- Rooms can be bookmarked. User can browse on-air streams of followed livers.
  User can open any room by searching a UP name or typing a room URL/ID.

## 2026-11-04 session 1 (answers)

1. Audio engine: external `mpv` binary, controlled from Rust over mpv JSON IPC.
   One mpv process per room. Mixing happens in PipeWire/PulseAudio.
   mpv is added to the nix devshell.
2. Audio only. No video display.
3. Login: QR code login (bilibili passport). Manual cookie paste as fallback.
4. Danmaku client: in-house, in a separated lib crate (`crates/bili-live`).
5. HTTP API client: in-house, in a separated lib crate (`crates/bili-api`).
6. Grid cells show title, UP name, online count, mute state, and a recent-danmaku ticker.
   Column count auto-fits terminal width.
7. Bookmarks and settings: `~/.config/bilibili-dd-radio/`, TOML.
8. TLS backend: `rustls`. No system OpenSSL.
9. Platform: Linux first, macOS later, Windows later or never.
10. Binary name: `bdr`. UI language: English.
11. Milestone order: M1 single room open + info + danmaku + audio grid.
    M2 multi-room grid, mute, bookmarks, search, following list. M3 QR login, send danmaku.

## Coding standards (user)

- Workspace style: lib crates separated from the bin crate.
- Error handling: `thiserror` in lib crates, `anyhow` in the bin crate.
- Builders: `bon` crate.
- Newtypes distinguish same-shape primitive types. For example `RoomID` and `UID`.

## Protocol findings (verified from this machine, 2026-11-04)

- Room info: `GET https://api.live.bilibili.com/room/v1/Room/get_info?room_id={id}`.
  Accepts short id and real room id. Returns `room_id`, `uid`, `live_status`, `online`, `title`.
- Play URL: `GET https://api.live.bilibili.com/room/v1/Room/playUrl?cid={room_id}&platform=web&protos=http-flv,http-m3u8`.
  Returns `data.durl[].url` (HTTP-FLV or HLS).
- Danmaku server config: `GET https://api.live.bilibili.com/xlive/web-room/v1/index/getDanmuInfo?id={room_id}&type=0`
  with WBI signing (`w_rid`, `wts`). Returns `host_list` and `token`.
- WBI signing works anonymously (no login needed).
- WS URL form: `wss://{host}:{wss_port}/sub`.
- WS framing: 16-byte big-endian header `pack_len I, raw_header_size H, ver H, operation I, seq_id I`.
  Operations: AUTH=7, AUTH_REPLY=8, HEARTBEAT=2, HEARTBEAT_REPLY=3, SEND_MSG=4, SEND_MSG_REPLY=5.
  Auth body: `{"uid":0,"roomid":N,"protover":2,"platform":"web","type":2,"buvid":"","key":token}`.
  `AUTH_REPLY` body `{"code":0}` means success.
  Business packets: `ver=0` plain JSON, `ver=2` deflate stream, `ver=3` brotli stream.
  A deflate/brotli payload contains one or more inner 16-byte-header frames (ver=0).
  Commands use a `message` array or a single command object. `DANMU_MSG.danmus[i] = [text, info[], color]`,
  `info[5]` is the user name.
- UP name: `GET https://api.bilibili.com/x/space/acc/info?mid={uid}&platform=web`.
  Risk control applies to data center IPs. Use `buvid3`/`buvid4` from
  `GET https://api.bilibili.com/x/frontend/finger/spi`. Fall back to uid display on failure.
- Following list (M2): `https://api.live.bilibili.com/live_user/v1/Following/list` (requires login cookies).
- QR login (M3): `POST /x/passport-login/web/qrcode/generate` then poll
  `GET /x/passport-login/web/qrcode/poll?kq={kv}`. Success returns `cookie_info` (SESSDATA, bili_jct, DedeUserID).
- Danmaku send (M3): protobuf `com.app.live.chat.proto.DmReq`/`DmSend` over `SEND_MSG` (op=4).
  Reference protos live in public bilibili-live-danmaku projects. Protocol to be confirmed during M3.

## 2026-11-04 session 2 (M1: bdr TUI)

- M1 done: `bdr` opens the ratatui TUI. Views: room grid, add-room prompt,
  danmaku view, help. Keys: `g` add room, `x` close, `m` mute, `d` danmaku,
  `?` help, arrows/`hjkl` focus, `q` quit.
- `bdr probe <room-ref>` runs headless: room info, stream URL, then N danmaku
  lines from the live WS. Verified against room short id 6.
- Architecture: `App` owns a `Vec<RoomState>`. Opening a room is an async
  pipeline (`room::spawn_open`): resolve room, start the danmaku WS task,
  start mpv for audio. Results come back on one `OpenMsg` channel; per-room
  danmaku/audio events drain from per-room channels in the UI loop.
  The UI loop polls crossterm events with a 50 ms timeout and redraws.
- UI rendering is backend-generic (`ui::draw<B: Backend>`); tests render on
  ratatui's `TestBackend` in `ui_test.rs`.
- Debug: `BDR_TRACE=1` prints key events and pipeline messages to stderr.

## Toolchain findings (session 2)

- crossterm 0.29: raw mode is a free function `crossterm::terminal::enable_raw_mode`
  (no `EnableRawMode` command struct). Enter arrives as `KeyCode::Enter`
  (`\r`) in a real terminal but as Ctrl+J (`\n`) through a pty; the prompt
  accepts both.
- ratatui 0.30: the crossterm backend moved to the `ratatui-crossterm` crate.
  `TestBackend` lives in `ratatui::backend` (no feature flag).
  `Terminal::draw` returns `Result<CompletedFrame>`. `Layout::split` returns
  `Rc<[Rect]>`, so the grid builds row rects by hand.
  `Terminal::clear()` queries the cursor position and fails under `script`
  pty without a responding terminal; bdr does not call it.
- mpv in this container has no audio device. It starts and answers IPC
  (verified: `get_property idle` returns `true`) but emits no `file_loaded`
  or `idle` events, so the room cell shows `audio:` until an event arrives.
  On a machine with an audio device the events flow normally.
- mpv `--log-msg-to-file` crashes this mpv build at startup (exit 1, no
  file created). Do not use that flag; use `--really-quiet` instead.

## 2026-11-04 session 2 (remote audio, option A)

- The TUI can play audio on a machine with speakers. New subcommand
  `bdr audio-local` runs on that machine. It listens on 127.0.0.1 (default
  port 7888), starts one mpv per room, and applies `load`, `mute`, `stop`,
  `quit` commands. It keeps running after a TUI disconnects.
- New flag on the TUI: `--audio-remote HOST:PORT`. The TUI sends the same
  JSON commands instead of starting local mpv. Audio state events come back
  on the same channel and update the room cells.
- The user bridges the two machines with an SSH reverse forward, e.g.
  `ssh -R 7888:localhost:7888` style, so the TUI machine reaches the
  runner over loopback. The control channel is plain JSON over the user's
  own SSH tunnel. Do not bind it wide open.
- Stream URLs are public signed FLV links, so the laptop plays them
  directly. No bilibili API access is needed on the laptop.
- Protocol: JSON lines both ways. Commands carry a `room` key as a string
  room id. Events are `playing`, `idle`, `ended`, `gone`.
- Race guard: a `stop` may arrive while the room's mpv is still starting.
  The runner marks stopped rooms. A still-starting mpv drops itself.


## 2026-11-05 session 2 (remote audio, option A)

- The TUI can play audio on a machine that has speakers, and run
  headless on another. New subcommand `bdr audio-local [--port 7888]`
  is that runner: it listens on 127.0.0.1, starts one mpv per room,
  and stays running after a TUI disconnects.
- New TUI flag `--audio-remote HOST:PORT` (e.g. 127.0.0.1:7888).
  Open/mute/close are forwarded to the runner instead of local mpv.
  Audio state comes back on the same channel and drives the room
  cells. The status bar suffix shows `remote audio`.
- Bridge the two machines with an SSH tunnel so that port 7888 on
  the TUI machine reaches port 7888 on the laptop.
  Recommended for a headless TUI machine: run the tunnel from the
  laptop, where the user already logs in. Add `-R 7888` to the
  laptop's ssh command:
  `ssh -R 7888 tony@server`. That opens 127.0.0.1:7888 on the
  server and forwards it to 127.0.0.1:7888 on the laptop.
  The other form is `ssh -L 7888:127.0.0.1:7888 tony@laptop`,
  run from the server. It needs the server to reach the laptop.
  Then run `bdr audio-local` on the laptop and
  `bdr --audio-remote 127.0.0.1:7888` on the TUI machine.
  On one machine, no tunnel is needed: just run both.
  Check before starting the TUI: `ss -ltn | rg 7888` on the TUI
  machine must show the listener.
- `Connection refused` means nothing listens on 127.0.0.1:7888 on
  the TUI machine. Check the runner process, the port, and the
  tunnel. The TUI no longer dies from this. It keeps working for
  danmaku and retries the audio link in the background.
- Protocol: JSON lines over TCP, both ways. Commands:
  `load` (room id + signed FLV url), `mute`, `stop`, `quit`.
  Events: `playing`, `idle`, `ended`, `gone`. Room ids are strings.
- The runner keeps rooms between TUI sessions. A `stop` may arrive
  while the room's mpv is still starting, so the runner marks the
  room stopped. A late-started mpv for a stopped room drops itself.
- Tests: 3 round-trip tests. A TUI-side `RemoteAudio` talks to a real
  `TcpListener` in the test, and `audio_local`'s `Runner` logic is
  tested through the same JSON.


## 2026-11-05 session 3 (UI fixes, snapshot tests)

- Heart-click danmaku carry no text and no user name. The parser
  falls back to the uid, which renders as `0` or `10000` in the
  ticker. Empty-text danmaku are now filtered in `cmd.rs` (both
  layouts) and in `RoomState::apply_live`.
- Mute state is visible now: the cell shows `muted` when the room
  is muted, and toggling writes a status line `<room>: muted` or
  `unmuted`.
- `move_focus` cycles through existing rooms. Left/right step in
  row-major order and wrap at the ends. Up/down change the row and
  wrap, clamped to the rooms in the target row.
- The add-room prompt is a three-row box. The title row carries the
  hint `Enter add, Esc cancel`. The input row shows the typed text
  in bold with a yellow cursor block.
- UI regression gate: `crates/bdr/src/snapshot_tests.rs` follows
  the rushi-tui insta method (`docs/tui-insta-snapshot-testing.md`
  over there). Ten snapshots in `crates/bdr/src/snapshots/`.
  Review with `cargo insta test -p bdr && cargo insta review`.
- The audio runner logs `load`, `stop`, `quit`, and mpv start
  failures on stderr for tunnel debugging.


## 2026-11-05 session 4 (danmaku separator, multi-room start)

- One line renderer for both views. The grid ticker and the
  danmaku view both use `Line::short`. The grid trims to 26 chars.
  The danmaku view trims to 60. Danmaku lines read `user: text`
  in both views.
- `bdr room` now takes several room refs at once:
  `bdr room 6 7734200 https://...`. Each one is opened through
  the normal pipeline.


## 2026-11-05 session 5 (count-driven grid layout)

- The grid is driven by the room count, not the terminal width.
  Three rooms or fewer sit in one row. Each row holds at most
  three rooms. 1 room takes 100% width. 2 take 50:50. 3 take
  33:33:33. From 4 rooms, a second row begins.
- Rows split the vertical span evenly. 1 row takes the full
  height. 2 rows take 50:50. The last row and the last cell take
  the rounding remainder. A lone room in a row takes the full row
  width.
- `move_focus` uses the same count-based rows. Left/right cycle
  rooms in row-major order. Up/down wrap rows and clamp to the
  rooms in the target row.
- New snapshot: `snap_grid_four_rooms_two_rows`.


## Testing commands

- `cargo test` (whole workspace, 39 tests).
- `cargo insta test -p bdr && cargo insta review` (snapshot gate).
- `cargo run -p bdr -- audio-local --port 7891` (run the audio
  runner, then point a TUI at it with
  `cargo run -p bdr -- --audio-remote 127.0.0.1:7891`.
- `cargo run -p bdr -- probe 6 -n 3 -t 30` (headless end-to-end check).
- `script -qec "stty rows 30 cols 120; ./target/debug/bdr" /dev/null`
  (pty-driven TUI test; a pty without a real terminal has a 0x0 size,
  so set it with `stty`).


## Known risk

- Bilibili web APIs are unofficial. Endpoints and risk control can change.
  The code must fail with clear error messages when an endpoint changes.
