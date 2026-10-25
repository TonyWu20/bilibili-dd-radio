#!/usr/bin/env python3
"""Analyze the captured pane stdout byte stream against the
tmux in-band DCS envelope rules, for the mpv --vo=kitty passthrough test.

usage: analyze-client.py <workdir> <expect:pass|fail>

Reads <workdir>/pane-stdout.raw and <workdir>/pane.txt, prints the
diagnostics, exits 0 on the expected outcome.
"""
import base64, re, sys

def main() -> int:
    w, expect = sys.argv[1], sys.argv[2]
    raw = open(f"{w}/pane-stdout.raw", "rb").read()
    pane = open(f"{w}/pane.txt", "r", errors="replace").read()

    # A base64 run of 100+ chars in the pane text means tmux wrote
    # the raw payload into the pane as text.
    flood = re.search(r"[A-Za-z0-9+/]{100,}={0,2}", pane) is not None

    # Walk each "ESC P tmux;" envelope the way tmux does: an ESC
    # ESC pair in the payload undoubles to one ESC, and ESC ST
    # terminates the envelope.
    PREFIX = b"\x1bPtmux;"
    payloads = []
    i = raw.find(PREFIX)
    while i >= 0:
        i += len(PREFIX)
        out = bytearray()
        terminated = False
        while i < len(raw):
            b = raw[i]
            if b == 0x1b and i + 1 < len(raw) and raw[i + 1] == 0x1b:
                out.append(0x1b)
                i += 2
                continue
            if b == 0x1b and i + 1 < len(raw) and raw[i + 1] == 0x5c:
                i += 2
                terminated = True
                break
            out.append(b)
            i += 1
        payloads.append((bytes(out), terminated))
        i = raw.find(PREFIX, i)

    # Parse the undoubled payloads as the kitty image protocol.
    # Each image DCS is: ESC _ G <ctrl> ESC \ where ctrl ends with
    # ';' and the base64 data sits between ';' and the terminator.
    parsed = []
    for p, terminated in payloads:
        if not terminated:
            parsed.append((b"<unterminated>", b"", True))
            continue
        j = 0
        while True:
            k = p.find(b"\x1b_G", j)
            if k < 0:
                break
            end = p.find(b"\x1b\\", k)
            if end < 0:
                parsed.append((b"<unterminated>", p[k + 3:], True))
                break
            body = p[k + 3:end]  # k + 3 skips ESC _ G
            sc = body.find(b";")
            if sc >= 0:
                ctrl, data = body[:sc], body[sc + 1:]
            else:
                ctrl, data = body, b""
            parsed.append((ctrl, data, False))
            j = end + 2

    headers = [d for c, d, bad in parsed if c.startswith(b"a=T")]
    conts = [d for c, d, bad in parsed if c.startswith(b"m=")]
    deletes = [c for c, d, bad in parsed if c.startswith(b"a=d")]
    bad = [1 for c, d, bad in parsed if bad]
    total_b64 = b"".join(d for c, d, bad in parsed if not c.startswith(b"a=d") and not bad)

    expected = 96 * 54 * 3
    decoded_ok = False
    n = len(headers)
    if total_b64 and n:
        try:
            decoded = base64.b64decode(total_b64, validate=True)
            # Every frame had the same size in this test.
            decoded_ok = len(decoded) == expected * n
        except Exception:
            decoded_ok = False

    print(f"envelopes: {len(payloads)}  unterminated: {len(bad)}")
    print(f"image frames (a=T headers): {n}")
    print(f"continuation DCS (m=): {len(conts)}")
    print(f"delete-all DCS (a=d): {len(deletes)}")
    print(f"base64 total: {len(total_b64)} bytes; decodes to exact RGB24 size: {decoded_ok}")
    print(f"pane base64 flood: {flood}")

    if expect == "pass":
        ok = bool(n) and decoded_ok and not flood and not bad
    else:
        ok = flood or not n
    print(f"RESULT: {'PASS' if ok else 'FAIL'} (expect={expect})")
    print(f"artifacts: {w}")
    return 0 if ok else 1

if __name__ == "__main__":
    sys.exit(main())
