# mpv --vo=kitty tmux passthrough: patch and test kit

## Files

| File | Purpose |
|------|---------|
| `vo_kitty-tmux-passthrough.patch` | Patch for mpv v0.41.0 (tag `v0.41.0`, commit `41f6a64506`). Moves the chunked base64 image data inside the tmux in-band DCS envelope in `flip_page()`. |
| `test-tmux.sh` | Headless tmux test. Runs an mpv binary in a tmux pane with `allow-passthrough on`, captures the pane stdout stream with `pipe-pane`, and checks the kitty image DCS against the envelope rules. |
| `analyze-client.py` | Byte-level analyzer used by `test-tmux.sh`. |
| `monitor-build.sh` | rushi monitor that blocks on the build and pokes session `video-in-kitty`. |

The bdr flake ships the patched mpv as a package:

    nix profile install github:bilibili-dd-radio#mpv-patched

The flake devShell uses the same package.

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

| Binary | Envelopes | Complete frames | Tail cut | Pane |
|--------|-----------|-----------------|----------|------|
| stock mpv 0.41 (control) | 4680 | 0 | n/a | base64 flood |
| patched, nix package | ~2500 | 361 | allowed | clean |

The patched payload decodes to complete RGB24 frames
(7485696 bytes = 361 frames of 96x54). The analyzer allows one
unterminated DCS only as the last item of the capture: the pipe
window can end mid-frame.

## Build the patched mpv

    nix build .#packages.x86_64-linux.mpv-patched

The package builds the vendored mpv v0.41.0 source
(`vendor/mpv-0.41.0` in the repository root) with the patch from
this directory.

The mpv source is vendored, not fetched. GitHub is not reliably
reachable from the build hosts: codeload 404s the mpv tarballs
and in-sandbox git fetch fails. The vendored tree is the exact tag
`41f6a645068483470267271e1d09966ca3b9f413`.

## Run the tests

    cd /home/tony/programming/bilibili-dd-radio
    URL=$(./target/debug/examples/playurl 6)

    # control: stock mpv must flood the pane (expect=fail)
    scripts/mpv-passthrough/test-tmux.sh "$(which mpv)" "$URL" fail

    # patched: the envelope must carry the image data (expect=pass)
    scripts/mpv-passthrough/test-tmux.sh "$(nix build .#packages.x86_64-linux.mpv-patched --print-out-paths | head -1)/bin/mpv" "$URL" pass

Pass criteria: at least one complete RGB24 frame in the captured
envelopes, no base64 run of 100+ characters in the pane text, and
at most one unterminated DCS as the last item of the capture.

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
- The flake overrides `mpv-unwrapped`, not `mpv`. `pkgs.mpv` is
  the with-scripts wrapper and ignores `src`/`patches` overrides.
