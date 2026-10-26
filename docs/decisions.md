# Decision Log

Record of user decisions and technical findings. Newest first.

Date correction (2026-10-24): the entries written on 2026-10-12 and
2026-10-24 were mislabeled 2026-11-04/05. The serving model wrote
future dates. The dates were re-checked against the commit history
and corrected.

## 2026-10-26 session 11 (video overlay: implementation and headless test)

- bdr now has the per-room togglable video overlay. v toggles the
  focused room in the grid and the danmaku view (commit 13c1dd3).
  Grid places the image one cell inside the focused cell. Danmaku
  places it in the top-left corner. The toggle respawns the room
  mpv with the kitty video driver and the position flags. The
  video cell renders only its border. Remote audio rooms reject
  the toggle.
- Findings from the headless run:
  - The mpv children steal the TUI keys. mpv reads the same pty
    as bdr and its input handler swallows the keys. All mpv
    spawns now pass --no-input-terminal.
  - Respawn bursts: the sync pass rescheduled on every loop until
    the first Ready landed. Several mpv instances raced one
    socket. A video_pending flag on the room gates the schedule.
  - An early v (before the stream URL lands) is rejected with
    "no stream yet". The test waits for the "mpv start: Ok"
    trace before pressing v.
  - A pty made by a script cannot hold a window size on this
    host: the slave TIOCGWINSZ fails with EIO and the master
    TIOCSWINSZ does not stick. crossterm reads 0x0 and bdr
    refuses to place the image. The test runs bdr in a private
    tmux session (tmux new-session -x 120 -y 30 on a temp
    socket). tmux owns the pty and its size.
  - Frame bytes do not pass through tmux: the tmux parser
    consumes the kitty DCS. The headless test checks the mpv
    process table instead: spawn, pid, flags. Frame flow stays
    in the standalone mpv test (360 complete frames) and the Mac
    ghostty smoke test.
- New script: scripts/video-overlay-test.sh. It drives bdr in a
  private tmux server with send-keys and checks the process table
  across grid, danmaku, and off. It passes: the kitty mpv spawns
  in the grid, keeps the same pid in the danmaku view (no
  respawn when the position is unchanged), and is gone when the
  overlay turns off.
- All 43 bdr unit tests pass. cargo clippy is clean.
- Open, on the user side: the Mac ghostty smoke test, the M4
  hardware-decode harness run, the CPU tolerance for N overlays,
  and the push of the pending commits (no GH_TOKEN on this host).

## 2026-10-25 session 10 (mpv fork: the tmux passthrough patch, verified headlessly)

- The session 9 upstream candidate is now a built and tested
  patch. mpv master (703e59b) is cloned to
  /home/tony/programming/mpv-kitty-passthrough/src. Three commits
  fix video/out/vo_kitty.c:
  - b61a195: wrap the chunked base64 in the tmux in-band envelope
    (the session 9 diff).
  - ccb73f2: build control strings with talloc (av_asprintf is not
    declared in this tree).
  - b406975: drop the NUL from the doubled ST literal.
- Test kit, committed to the bdr repo:
  scripts/mpv-passthrough/ (patch, test-tmux.sh,
  analyze-client.py, README.md, monitor-build.sh).
- Test method. A private tmux 3.8 server runs on a temp socket
  with allow-passthrough on. The mpv binary is the session
  start-command, in a pane that starts in /. The pane stdout
  stream is captured with pipe-pane. The pane screen is checked
  for base64 runs. The envelope bytes are walked the way tmux
  walks them: an ESC ESC pair undoubles to one ESC, ESC ST ends
  the envelope.
- Result, patched binary (mpv 0.41.0-dev-gccb73f2fe, 15 s of
  stream at 96x54):
  - 360 complete image frames, 1800 continuation DCS.
  - 0 unterminated DCS.
  - 7464960 bytes of base64. They decode to the exact RGB24 byte
    size for 360 frames of 96x54.
  - The pane screen is clean. No base64 flood.
- Result, stock mpv 0.41 control, same harness:
  - 4680 envelopes, 2160 unterminated DCS, 0 complete frames.
  - The pane floods with base64 text.
  - The session 9 symptom is reproduced headlessly on the host.
- Harness findings worth keeping:
  - A captured tmux client attach shows no envelopes. The client
    swallows the in-band DCS on its own pty. The pane stdout is
    the observable stream.
  - Starting the tmux server from a direnv-managed repository
    makes the pane shell load the devshell menu. It eats the
    start command. The pane must start in /.
  - nixpkgs ffmpeg and libass need the .dev split for headers and
    pkg-config files. nix shell does not set PKG_CONFIG_PATH for
    dev-only outputs. build.sh exports it from nix eval --raw.
- The byte layout of the fixed envelope matches the kitty-tmux
  proxy: the tmux DCS prefix, then the kitty DCS with every ESC
  doubled, then ESC ST.
- Open decision. Three paths for bdr under tmux:
  1. Ship the patched mpv into the devshell.
  2. Keep the kitty-tmux proxy as the tmux path.
  3. File the upstream mpv issue and wait for a release.

## 2026-10-25 session 10 addendum (flakes and the v0.41.0 rebase)

- The upstream patch was rebased onto the mpv v0.41.0 tag
  (commit 41f6a645068483470267271e1d09966ca3b9f413). That tag is
  the newest release ref and matches the mpv both devshells run.
  The v0.41.0 kitty VO predates the terminal_swapchain refactor.
  The patch ports to it with the same fix shape.
- The flake now ships the patched mpv as a package:
  `packages.<system>.mpv-patched`. The devShell uses the package
  instead of the nixpkgs mpv.
- The first package design vendored the mpv v0.41.0 tree
  (vendor/mpv-0.41.0, about 17 MB) because GitHub looked
  unreachable from the build hosts. The user rejected a 17 MB
  source tree in the repository.
- The final design keeps no mpv source in the repository. The
  flake adds the patch to the nixpkgs mpv-unwrapped base.
  nixpkgs fetches the mpv source itself (fetchFromGitHub, tag
  v0.41.0, hash-checked). On this host the tarball arrives via
  cache.nixos.org, so the build works.
- The flake exposes the patch as an overlay
  (overlays.<system>.default) and applies it to its own
  packages and devShells with lib.fix. Users can also apply the
  overlay to their own nixpkgs package sets.
- Caveat: when nixpkgs bumps mpv past 0.41.0 the patch may stop
  applying. The flake lock pins the nixpkgs revision, so this
  only happens on a nix flake update.
- The flake must override `pkgs."mpv-unwrapped"`. `pkgs.mpv` is
  the with-scripts wrapper. Overriding the wrapper adds the
  attributes to the wrapper and the base build stays stock. The
  mistake is silent: the package builds and behaves exactly like
  stock mpv.
- The nix-built package passed the tmux test: 361 complete RGB24
  frames, clean pane. The stock binary floods the pane in the
  same harness.

## 2026-10-24 session 9 (tmux: mpv passthrough bug, kitty-tmux is the path)

- Mac-side investigation (user): with
  `--vo-kitty-auto-multiplexer-passthrough=yes`, tmux still renders
  raw kitty escapes. The user's `allow-passthrough` setting is
  correct. The fault is in the mpv 0.41 passthrough code path.
- Byte-level mechanism (verified on the Mac against mpv 0.41):
  - mpv wraps the kitty control sequences in the tmux in-band
    envelope `ESC P tmux; … ESC \`.
  - The base64 image payload is appended as plain text, outside
    the envelope.
  - tmux forwards the envelope and renders the base64 as pane
    text. About 2 MB of base64 per frame at 960x540 streams by.
    The client gets `m=1` controls with no data. No image.
- Source confirmation (mpv master, checked 2026-10-24):
  - `flip_page()` in video/out/vo_kitty.c: the control header goes
    out wrapped (line 315). The base64 chunks are appended raw
    (line 329). Only the end marker is wrapped (line 330).
  - The option landed in commit d9dc84c0 (2025-03-22). No fix in
    the vo_kitty.c history up to 2026-10-10. The bug is still in
    master.
- Configurations, verified on the Mac (tmux 3.8 + ghostty):
  - No tmux: plain kitty VO works (session 7).
  - tmux + `kitty-tmux mpv …` + `=no`: works. The proxy re-wraps
    each `DCS G` with the data inside.
  - tmux + `=yes`: broken. Character flood, no image.
  - tmux + wrapper off: tmux drops the whole image. Clean pane,
    no video. Not a fix on its own.
  - `--vo-kitty-use-shm=yes`: tested on Mac + ghostty, without
    tmux. No video. Ghostty's kitty library parses the `t=s`
    medium, but it is gated behind image-loading limits that
    default to direct base64 only (`Limits.direct`, shared
    memory off). Enabling it needs the host app to call
    `setKittyGraphicsLoadingLimits` with
    `allWithTempDir`. The stock app does not do that in the
    files checked (Surface, App, main). The shm path is a dead
    end on ghostty.
- Design consequence for bdr:
  - bdr in a tmux pane runs as `kitty-tmux bdr`. The mpv children
    inherit the pty, so their escapes pass through the proxy.
    bdr must not set the auto-passthrough flag on its mpv
    command lines.
  - bdr without tmux: no wrapper. Default flags are correct.
- Upstream: new-feature bug (d9dc84c0). Report it to mpv. The
  diff moves the base64 append inside the envelope in
  `flip_page()`.

## 2026-10-24 session 8 (bdr video: design and performance gate)

- Decision: terminal video enters the bdr scope. It renders through
  mpv over the kitty protocol, the engine verified in session 7.
- Design requirements (user):
  - A per-room video overlay. The overlay is togglable. This is a
    must-have.
  - The focused-room layout (the danmaku view) also carries a video
    overlay, togglable.
- Design proposal, pending user sign-off:
  - One mpv per room, as the audio path already does. A room with
    video on adds the kitty flags and its stdout goes to the
    terminal. A room without video keeps `--no-video` and null
    stdout.
  - Toggling video respawns that room's mpv. Audio restarts with
    it.
  - `v` toggles the video of the focused room, in the grid view
    and in the danmaku view.
  - The TUI leaves the overlay cells empty. It draws a border
    around them. The kitty image renders above normal text.
  - On terminal resize or grid change, the video mpv respawns so
    the image lands in the new cell position.
  - Remote audio mode: no video. The runner has no terminal.
- mpv source findings (video/out/vo_kitty.c, v0.41):
  - `--vo-kitty-top` and `--vo-kitty-left` place the image in
    cells. The image draws at the cursor position.
  - `--vo-kitty-alt-screen` and `--vo-kitty-config-clear` default
    to yes. Set both to no for in-TUI use, or mpv takes over the
    whole screen.
  - `--vo-kitty-auto-multiplexer-passthrough` (default no) claims
    to wrap the escapes in tmux or screen passthrough when $TMUX
    or $STY is set. Tested on the Mac: broken for tmux. The
    base64 payload goes out outside the envelope. See session 9.
  - On exit, mpv sends `a=d`. That deletes every kitty image in
    the terminal, not just its own. The other overlays vanish for
    one frame until each live mpv sends its next frame.
- Performance gate, run headless on this NixOS host (GTX 1070,
  44 cores, nix devshell mpv v0.41):
  - GPU hardware decode does not engage. `--hwdec=auto-safe` lands
    on software decode. `hwdec-current` reports `vulkan-copy`,
    which is GPU copy only.
  - Explicit `--hwdec=vaapi`, `cuda`, and `vulkan` all fail with
    "Could not create device". The nvidia 580.142 driver is
    present: nvidia-smi works and cuInit succeeds from a test
    program. The nixpkgs store has no nvidia libva driver. The
    nixpkgs mpv build or the container GPU access blocks device
    creation. Not root-caused further.
  - Software decode, 3 concurrent overlays, 60 s window:

    | room    | drops/60s | cpu%  |
    |---------|-----------|-------|
    | 6       | 47        | 52.7  |
    | 280446  | 39        | 84.3  |
    | 6557640 | 21        | 87.4  |

    Total CPU 224.4%. GPU utilization stays near 0%. All three
    streams play in real time. The drop rate is around 1-2.5% of
    frames at 30 fps.
  - Read: three overlays work on this host, but the software
    pipeline costs about 2.25 cores. This is the pessimistic
    case. Hardware decode is expected to cut the cost sharply.
- Open:
  - The M4 Mac target is untested. Its VideoToolbox decode is the
    likely winner. Run `scripts/video-perf.sh` on the Mac and
    record `hwdec-current` and the CPU there.
  - The pixel-to-cell mapping of the kitty image depends on the
    terminal font. Confirm it once on the target terminal before
    the cell math.
  - The user sets the CPU tolerance for N overlays.
- Harness: `scripts/video-perf.sh`. Headless host or terminal
  host. It spawns N mpv instances, waits 5 s, measures the window
  given by `-d`, then reports per stream: `hwdec-current`,
  `frame-drop-count`, CPU. It kills the mpv instances on exit.

## 2026-10-24 session 7 (video in the terminal: mpv + kitty protocol)

- After several test rounds, `mpv` renders live stream video in the
  terminal smoothly with `--vo=kitty`. Verified on the local ghostty.
  The 720x405 frame fits the current UI design and purpose.
- This supersedes session 1 answer 2 ("Audio only. No video display."):
  terminal video is viable.
- Verified command shape, playing a signed bilibili HLS URL:

  ```bash
  mpv --vo=kitty --vo-kitty-use-shm=no \
      --profile=low-latency \
      --hwdec=auto-safe \
      --vd-lavc-fast \
      --vo-kitty-width=720 \
      --vo-kitty-height=405 \
      --msg-level=all=no --term-status-msg="" --term-osd-bar=no \
      --really-quiet \
      --referrer=https://live.bilibili.com/ \
      "https://<cdn-host>/live-bvc/<room>/<stream>/index.m3u8?<signed query>"
  ```

- Flag notes:
  - `--vo=kitty`: video out through the kitty graphics protocol.
    ghostty supports that protocol.
  - `--vo-kitty-width=720`, `--vo-kitty-height=405`: a fixed 16:9
    frame, in pixels.
  - `--profile=low-latency`, `--hwdec=auto-safe`, `--vd-lavc-fast`:
    a low-latency path. Hardware decode when safe. Fast lavc decode.
  - `--vo-kitty-use-shm=no`: no POSIX shared-memory image transport.
  - `--msg-level=all=no`, `--term-status-msg=""`, `--term-osd-bar=no`,
    `--really-quiet`: no mpv text or OSD on the terminal, so the
    TUI grid stays intact.
  - `--referrer=https://live.bilibili.com/`: the bilibili CDN checks
    the referer. The URL is the signed play URL from `Room/playUrl`,
    shortened here because it expires.
- tmux: mpv plus the kitty protocol has a separate known issue inside
  tmux. It involves `allow-passthrough on`. The tested workaround
  wraps the mpv command in `kitty-tmux` (tmux 3.3 or later):

  ```bash
  cargo install --git https://github.com/gergesh/kitty-tmux
  # in ~/.tmux.conf:
  set -g allow-passthrough on
  kitty-tmux mpv --vo=kitty ...
  ```

  `kitty-tmux` runs the command on a pty and re-wraps the kitty
  graphics escapes in tmux passthrough. The image reaches the
  terminal outside tmux. Outside tmux it execs the command directly.
- Tested on the MacBook Air M4: a local tmux server plus ghostty.
- Open: video is not in the bdr scope yet. Candidates: one video
  area for the focused room, or a per-room overlay. `bili-audio`
  starts mpv with `--no-video`, so a video mode changes the spawn
  flags or adds a second mpv process.

## 2026-10-24 session 6 (login + following rooms)

- Scope: user asked for login and for fetching/navigating the live rooms
  of the livers the user follows (M3 login and M2 following list from
  the milestone plan, done together).
- **Correction to the M2 protocol finding**: `live_user/v1/Following/list`
  is dead. It answers `code=1000002, controller
  live_user\\frontend\\controller\\v1\\C_Following not found`.
  The live home is the working source: WBI-signed
  `GET https://api.live.bilibili.com/xlive/web-interface/v1/index/getList?platform=web&web_location=444.7`.
  The "我的关注" (my follows) module has `module_info.type == 8` and is
  only sent to logged-in sessions. Its rooms carry `roomid`, `uid`,
  `uname`, `title`, `online`. Anonymous sessions get category and
  recommendation modules only. `bdr` shows the followed rooms when
  logged in; otherwise it shows the recommended modules and a hint.
- QR login (verified on this machine):
  - `GET https://passport.bilibili.com/x/passport-login/web/qrcode/generate`
    (anonymous) returns `data.url` (the scan URL) and `data.qrcode_key`.
  - `GET .../qrcode/poll?qrcode_key=…` returns `data.code`: 86101
    waiting, 86090 scanned, 0 success, 86038 expired. On success the
    `Set-Cookie` headers carry `SESSDATA`, `bili_jct`, `DedeUserID`
    (plus `DedeUserID__ckMd5`, and `buvid3`/`buvid4` when set), and
    `data.refresh_token` is present.
- Login UX: TUI view opened with `L` in the grid. The QR code is
  rendered as Unicode block characters (two QR module rows packed into
  one half-block row, two-cell quiet zone) using the `qrcode` crate
  (`default-features = false`). No image-protocol dependencies: this
  is a deliberate simplification of the MareDevi reference, which uses
  `ratatui_image` plus a character fallback. The poll runs every two
  seconds. `r` refreshes an expired or failed code. On success the
  credentials are saved, the API client is rebuilt with the login
  cookie and fingerprint, and the app returns to the grid with a
  `logged in as uid N` status.
- Following UX: TUI view opened with `f` in the grid. It lists the
  followed live rooms, then the recommended rooms. `j/k` or arrows
  move the selection, `Enter` opens the selected room through the
  normal open pipeline, `r` refreshes, `Esc` goes back.
- New CLI: `bdr login` prints the QR code in a plain terminal and
  polls until success, expiry, or Ctrl-C. It works headless.
  `bdr login --cookie "SESSDATA=…; bili_jct=…; DedeUserID=…"` saves a
  pasted cookie string (the manual-paste fallback from decision 3).
  `bdr logout` removes the credentials file.
- Credentials live in `~/.config/bilibili-dd-radio/credentials.toml`
  (TOML per decision 7), mode `0600`. Fields: `sessdata`, `bili_jct`,
  `dede_user_id`, `dede_user_id_ckmd5`, `buvid3`, `buvid4`,
  `refresh_token`. All commands load them at start; the status bar
  shows `uid N` when a login is active.
- A fake or expired login cookie does not break the following view.
  The server answers as anonymous, the recommended modules still
  arrive, and the user sees the login hint.

## 2026-10-12 session 1

- Language and UI: Rust, `ratatui` TUI.
- Product: listen to multiple bilibili live rooms together. Audio is mixed in the OS audio stack.
  Each room can be muted. Rooms form a dynamic growing grid.
- Danmaku: user can receive danmaku of each room. After OAuth login, user can send danmaku.
- Rooms can be bookmarked. User can browse on-air streams of followed livers.
  User can open any room by searching a UP name or typing a room URL/ID.

## 2026-10-12 session 1 (answers)

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

## Protocol findings (verified from this machine, 2026-10-12)

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
- Following list (M2): `https://api.live.bilibili.com/live_user/v1/Following/list` (requires login cookies). **Superseded 2026-10-24: this endpoint is dead** (`code=1000002`, controller not found). Use the live home `getList` module type 8 instead; see the session 6 entry.
- QR login (M3): `POST /x/passport-login/web/qrcode/generate` then poll
  `GET /x/passport-login/web/qrcode/poll?kq={kv}`. Success returns `cookie_info` (SESSDATA, bili_jct, DedeUserID).
- Danmaku send (M3): protobuf `com.app.live.chat.proto.DmReq`/`DmSend` over `SEND_MSG` (op=4).
  Reference protos live in public bilibili-live-danmaku projects. Protocol to be confirmed during M3.

## 2026-10-12 session 2 (M1: bdr TUI)

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

## 2026-10-12 session 2 (remote audio, option A)

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


## 2026-10-12 session 2 (remote audio, option A, runner and tunnel)

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


## 2026-10-12 session 3 (UI fixes, snapshot tests)

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


## 2026-10-12 session 4 (danmaku separator, multi-room start)

- One line renderer for both views. The grid ticker and the
  danmaku view both use `Line::short`. The grid trims to 26 chars.
  The danmaku view trims to 60. Danmaku lines read `user: text`
  in both views.
- `bdr room` now takes several room refs at once:
  `bdr room 6 7734200 https://...`. Each one is opened through
  the normal pipeline.


## 2026-10-12 session 5 (count-driven grid layout)

- The grid is driven by the room count, not the terminal width.
  Three rooms or fewer sit in one row. Each row holds at most
  three rooms. 1 room takes 100% width. 2 take 50:50. 3 take
  33:33:33. From 4 rooms, a second row begins.
- Rows split the vertical span evenly. 1 row takes the full
  height. 2 rows take 50:50. The last row and the last cell take
  the rounding remainder. A lone room in a row takes the full row
  width.
- A width cap sits on top of the count rule. A row holds at most
  `(width - 4) / 28` rooms, each cell wanting 28 columns. So a
  narrow terminal shrinks the rows: 80 columns give rows of two.
  Wide terminals keep the count rule.
- `move_focus` uses the same count and width caps. Left/right
  cycle rooms in row-major order. Up/down wrap rows and clamp to
  the rooms in the target row.
- Snapshots: `snap_grid_four_rooms_two_rows` (wide) and
  `snap_grid_narrow_four_rooms` (80 columns, rows of two).


## Testing commands

- `cargo test` (whole workspace, 40 tests).
- `cargo insta test -p bdr && cargo insta review` (snapshot gate).
- `cargo run -p bdr -- audio-local --port 7891` (run the audio
  runner, then point a TUI at it with
  `cargo run -p bdr -- --audio-remote 127.0.0.1:7891`.
- `cargo run -p bdr -- probe 6 -n 3 -t 30` (headless end-to-end check).
- `scripts/video-perf.sh -q 6 280446 6557640` (headless video
  performance gate; needs the nix devshell for mpv; session 8).
- `scripts/video-perf.sh -d 60 6 280446 6557640` (the same gate in
  a terminal host: ghostty or kitty; session 8).
- `kitty-tmux bash scripts/video-perf.sh -d 60 6 280446 6557640`
  (the gate inside tmux; session 9).
- `script -qec "stty rows 30 cols 120; ./target/debug/bdr" /dev/null`
  (pty-driven TUI test; a pty without a real terminal has a 0x0 size,
  so set it with `stty`).
- `XDG_CONFIG_HOME=$(mktemp -d) cargo run -p bdr -- login --cookie "SESSDATA=x; bili_jct=y; DedeUserID=1"`
  (save credentials into a throwaway config dir; check the file is
  `0600`, then `cargo run -p bdr -- logout`).
- `cargo run -p bdr -- login` prints a scannable QR code in a plain
  terminal. Point the bilibili app at it to log in from a headless
  machine.
- `BDR_COOKIE="SESSDATA=…; bili_jct=…" cargo run -p bili-api --example home_smoke`
  (followed rooms need a real login session; anonymous shows the
  recommended rooms).


## Known risk

- Bilibili web APIs are unofficial. Endpoints and risk control can change.
  The code must fail with clear error messages when an endpoint changes.
