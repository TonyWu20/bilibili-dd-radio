# mpv --vo=kitty tmux passthrough: patch and test kit

## Files

| File | Purpose |
|------|---------|
| `vo_kitty-tmux-passthrough.patch` | Patch for mpv commit `703e59b` (master, 2026-10), three commits (`b61a195`, `ccb73f2`, `b406975`). Moves the chunked base64 image data inside the tmux in-band DCS envelope in `flip_page()`. |
| `test-tmux.sh` | Headless tmux test. Runs an mpv binary in a tmux pane with `allow-passthrough on`, captures the pane stdout stream with `pipe-pane`, and checks the kitty image DCS against the envelope rules. |
| `analyze-client.py` | Byte-level analyzer used by `test-tmux.sh`. |
| `monitor-build.sh` | rushi monitor that blocks on the build and pokes session `video-in-kitty`. |

## Bug (verified against mpv source and on ghostty + tmux 3.8)

Stock mpv 0.41 with `--vo-kitty-auto-multiplexer-passthrough=yes`
under tmux: `flip_page()` wraps the kitty control fragments in the
tmux in-band DCS envelope but appends the base64 chunk after the
envelope. tmux keeps the control fragments and writes the payload
into the pane as text. At 720x405 that is about 2 MB of text per
frame. No image data reaches the terminal.

The patched byte layout matches the `kitty-tmux` proxy:

    ESC P tmux; ESC ESC _ G a=T,f=24,s=W,v=H,C=1,q=2,m=1; <b64> ESC ESC \ ESC \

tmux undoubles the `ESC ESC` pairs and forwards the complete kitty
DCS to the client.

## Results on this host (tmux 3.8, 2026-10-25)

| Binary | Envelopes | Complete frames | Unterminated DCS | Pane |
|--------|-----------|-----------------|------------------|------|
| stock mpv 0.41 (control) | 4680 | 0 | 2160 | base64 flood |
| patched `b406975` | 2520 | 360 | 0 | clean |

The patched base64 decodes to the exact RGB24 byte size
(7464960 bytes for 360 frames of 96x54).

## Build the patched mpv

    cd /home/tony/programming/mpv-kitty-passthrough
    git am < /home/tony/programming/bilibili-dd-radio/scripts/mpv-passthrough/vo_kitty-tmux-passthrough.patch
    bash build.sh

The binary lands in `src/build/mpv`. The build needs the nixpkgs
`ffmpeg.dev` and `libass.dev` splits. `nix shell` does not set
`PKG_CONFIG_PATH` for dev-only outputs, so `build.sh` exports it
from `nix eval --raw` store paths.

## Run the tests

    cd /home/tony/programming/bilibili-dd-radio
    URL=$(./target/debug/examples/playurl 6)

    # control: stock mpv must flood the pane (expect=fail)
    scripts/mpv-passthrough/test-tmux.sh "$(which mpv)" "$URL" fail

    # patched: the envelope must carry the image data (expect=pass)
    scripts/mpv-passthrough/test-tmux.sh /home/tony/programming/mpv-kitty-passthrough/src/build/mpv "$URL" pass

Pass criteria: every kitty image DCS is terminated and complete,
the concatenated base64 decodes to the exact RGB24 byte size, and
the pane text holds no base64 run of 100+ characters.

## Notes

- The test uses a private tmux server on a temp socket. It never
  touches a user tmux session.
- The pane starts in `/`. A direnv-managed start directory makes
  the pane shell load the devshell menu, which eats the start
  command.
- A captured tmux client attach shows no envelopes: the client
  swallows the in-band DCS on its own pty. The pane stdout
  stream is the observable byte stream.
- The analyzer walk follows tmux in-band DCS semantics: `ESC ESC`
  undoubles to one ESC, `ESC ST` ends the envelope.
