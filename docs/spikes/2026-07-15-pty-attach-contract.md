# omnigent PTY-attach WebSocket contract (Spike B)

**Status:** B0 (source-read) **DONE** 2026-07-15. B2 (live-verify) **DONE**
2026-07-16 — driven against a live omnigent 0.5.1 (`08285468`) local server
(`:6767`) attaching a real `claude` native terminal. **Every source-derived
claim confirmed on the wire; no material divergence.** See "Actual (from wire)"
below; raw corpus in `docs/spikes/captures/2026-07-15-pty-attach/`.

Source: sibling checkout `../omnigent` @ `v0.5.1` / `08285468` (matches
`vendor/omnigent-0.5.1/OMNIGENT_PIN`). All citations are `omnigent/…` paths there.

---

## Headline finding (the source-only trap this spike caught)

The design assumed "WS bytes → `vt_write`" on a single raw byte stream. Two things
the source revealed:

1. **There are two server-side transports** — `pty`
   (`terminals/ws_bridge.py::bridge_tmux_pty_to_websocket`, forks `tmux attach`,
   streams the rendered screen) and `control`
   (`terminals/control_bridge.py::bridge_tmux_control_to_websocket`, consumes tmux
   `-C` control-mode `%output` and decodes it server-side). **`control` is the
   default** (`inner/terminal.py:103,108`: "Control mode is the default"; anything
   but a PTY-alias config value → control).

2. **But the browser-facing wire protocol is identical for both**
   (`control_bridge.py:31-34`): binary frames out = raw pane bytes, text frames in
   = JSON control, binary frames in = input bytes — "the two transports are
   interchangeable behind the same `/attach` WebSocket and a client cannot tell
   which one served it." The control-mode `%output`/`%begin`/`%layout` protocol is
   consumed **server-side**; the client never sees it.

**Consequence for the Lens design:** the attach client feeds libghostty a **raw
VT byte stream** regardless of transport. **No tmux-control-mode parser is needed
in the client** — the earlier assumption that we'd design around one is wrong.
Transport selection is an omnigent-server concern, not a client contract concern.
(We still capture both transports live in B2 to confirm they present identically.)

---

## Attach endpoint

- **URL:** `WS /v1/sessions/{session_id}/resources/terminals/{terminal_id}/attach`
  (`server/routes/terminal_attach.py:131`; mounted under `/v1` per the module
  docstring + `create_app`).
- **Query params** (`terminal_attach.py:136-137`):
  - `read_only` (bool, default `false`) — read-only view; drops binary input at
    server *and* runner, and passes `tmux attach -r`.
  - `transport` (`"control"` | `"pty"` | omitted) — per-attach override; omitted
    defers to terminal spec → global config default (**control**).
- **`{terminal_id}`** is an opaque resource id, e.g. `"terminal_bash_s1"`
  (`terminal_attach.py:148-149`), minted by the terminal-resource CRUD under
  `/v1/sessions/{id}/resources/terminals` (NOT this route — docstring lines
  114-117). Obtaining one is a B2 prerequisite.

## Auth

- Interactive (write) attach requires `LEVEL_OWNER`; read-only requires
  `LEVEL_READ` (`terminal_attach.py:334`, `_authorize_terminal_attach`).
- `permission_store is None` → **all checks skipped** (single-user / dev)
  (`terminal_attach.py:319-320`). So a local dev server needs no WS auth. Multi-user
  auth mechanism (header/query/cookie via `auth_provider.get_user_id(websocket)`)
  is **B2-verify** if we test an authed deployment; not needed for the dev spike.

## Wire protocol (transport-independent)

Server→client and client→server framing, identical under both bridges
(`terminal_attach.py:32-47`, `ws_bridge.py:17-27`, `control_bridge.py:31-34`):

| Direction | Frame type | Meaning |
|-----------|-----------|---------|
| **Server → client** | **binary** | Raw PTY/pane bytes (ANSI VT). Feed verbatim to `vt_write`. |
| **Client → server** | **text** (JSON) | Control message. Only `{"type":"resize","cols":N,"rows":M}` today → `ioctl(TIOCSWINSZ)` (`ws_bridge.py:644-652`). Unknown shapes ignored (forward-compat). |
| **Client → server** | **binary** | Raw input bytes written verbatim to the PTY (keystrokes, paste, mouse reports). Dropped when `read_only`. |

**The `on_pty_write` back-channel** (libghostty's DA/DSR replies) rides the
**binary client→server** frame — confirmed it carries arbitrary bytes verbatim
(`ws_bridge.py:653,679` `_write_all_nonblocking(master_fd, data)`).

**Output framing is a byte stream, not messages:** the server coalesces queued PTY
reads into bounded frames (flood cap 64 KiB, interactive cap 2 KiB within 0.75 s of
input) and splits reads larger than the cap across frames (`ws_bridge.py:64-73,
408-468`). So one binary frame may merge several reads and a big read may span
frames — the client MUST treat the concatenation as a raw byte stream (which
feeding `vt_write` does). `TERM` advertised to tmux is `xterm-256color`
(`ws_bridge.py:159`), which libghostty handles.

## Connect seed, reconnect, and the output gap

- **On (re)attach the client gets the CURRENT screen, not a replay of missed
  bytes:**
  - `pty` transport: forks `tmux attach`, which redraws the current pane on attach.
  - `control` transport: seeds once with `capture-pane -e -p` (current screen,
    escapes preserved), then streams subsequent `%output` (`control_bridge.py:22-24`;
    "a control client only receives `%output` produced *after* it attaches").
- **The output gap** (the "possible-output-gap marker" requirement): bytes emitted
  by the program **while no client is attached** are applied to the tmux pane but
  are **not** replayed byte-for-byte on re-attach — you receive a fresh snapshot of
  the resulting screen. So it's a **transient gap** (intermediate scroll lost;
  end-state screen current), not a persistent state loss. **B2 must confirm** this
  live and measure what exactly arrives on re-attach.
- **Pane persistence:** tmux panes outlive detach; the native-pane idle reaper
  reclaims an unused pane after **30 min** default (`terminals/pane_reaper.py:69`
  `_DEFAULT_IDLE_TIMEOUT_S = 30*60`; env `OMNIGENT_NATIVE_PANE_IDLE_TIMEOUT_S`,
  `0` disables). So same-resource reconnect works within that window.

## Close codes (the typed reconnect contract)

Application close codes in RFC-6455 4xxx range (`ws_bridge.py:79-89`) — the client's
reconnect loop MUST branch on these:

| Code | Meaning | Client action |
|------|---------|---------------|
| **4404** `TERMINAL_NOT_FOUND` | Pre-attach lookup miss, or PTY EOF with tmux session genuinely gone (agent exited / killed). | **Stop reconnecting.** |
| **4405** `TERMINAL_DETACHED` | The `tmux attach` child exited but the session is still alive (a detach). | **Reconnect OK** — must NOT be read as terminal-gone (that would tear the whole session/runner down). |
| **4500** `INTERNAL_ERROR` | Bridge/proxy failure. | Retry with backoff. |

Runner-side close codes are mirrored to the browser through the proxy
(`terminal_attach.py:203-213`, `_RunnerWSClosed`).

## Runner topology (informational)

In-process runner → server bridges the PTY locally. Out-of-process runner → server
**proxies frames verbatim** over a WS tunnel (`terminal_attach.py:170-202`,
`_shuttle_ws_frames` forwards bytes↔bytes / text↔text). Either way the
browser-facing contract above is identical. So the spike can target a single
local/in-process server and the contract generalizes.

---

## Actual (from wire) — live-verified 2026-07-16

Driven with the B1 harness (`spikes/terminal-attach`) against a local omnigent
0.5.1 server, attaching the `claude` native terminal `terminal_claude_main` on
session `conv_0b31…`. Corpus: `attach`/`input`/`resize`/`reconnect`.frames.jsonl.

1. **URL / scheme / prefix / auth — CONFIRMED.** `ws://127.0.0.1:6767/v1/sessions/
   {sid}/resources/terminals/{tid}/attach?transport=…&read_only=false` →
   `101 Switching Protocols`. **No auth** on the local dev server (`permission_store
   is None`). *Precision add:* the handshake upgrades to `101` **before** the
   terminal lookup, so even a bad `{tid}` gets `101` then an app-level close (see 6).

2. **Terminal resource — CONFIRMED.** `GET …/resources/terminals` → `{object:"list",
   data:[{id:"terminal_claude_main", type:"terminal", metadata:{terminal_name,
   session_key, running:true, tmux_socket, tmux_target, terminal_transport:
   "control"}}], has_more:false}`. Create body `{"terminal","session_key"}` → resource
   with `id` (matches source); reusing an existing `{sid,tid}` sidesteps it.
   `metadata.terminal_transport:"control"` **confirms control is the default**.

3. **Output framing — CONFIRMED binary raw VT.** Attach seed = one binary frame
   `\x1b[H\x1b[2J` + the Claude Code TUI in SGR truecolor/256 + box-drawing, then a
   mode-setup frame (`\x1b[?1000h`… mouse/kitty). Straight into `vt_write`.

4. **Transport-independent — CONFIRMED (the load-bearing B0 finding).** Both
   `?transport=control` (default) and `?transport=pty` deliver **raw VT binary**. Only
   difference is the seed *mechanism*: control seeds via `capture-pane` (~1.4 KB), pty
   via a full `tmux attach` redraw (~3.1 KB) — both raw VT; a client cannot tell them
   apart. **No tmux-control-mode protocol reaches the client.**

5. **Input framing — CONFIRMED binary.** A 28-byte command sent as a binary frame
   reaches the PTY (terminal reacts/redraws). Arbitrary bytes pass verbatim → the
   `on_pty_write` DA/DSR back-channel rides this path.

6. **Resize — CONFIRMED.** Text frame `{"type":"resize","cols":120,"rows":40}` (38 B)
   → full reflow (`\x1b[2J\x1b[H` + repaint at the new width).

7. **Reconnect / output gap — CONFIRMED transient.** Forced abrupt drop mid-output,
   then re-attach to the **same** `{tid}`: succeeds, and leg-2's seed is a fresh
   `\x1b[H\x1b[2J` + **current-screen redraw** — **no byte-replay** of leg-1's
   intermediate frames. The tmux pane persisted across the drop. So the gap is
   transient (current state delivered; missed intermediate output not replayed), as
   source predicted.

8. **Close codes — 4404 CONFIRMED live.** Attaching a non-existent `{tid}` → `101`
   then close **`4404` reason `"terminal resource not found or not running"`**.
   `4405` (detach) and `4500` (internal) are source-documented (`ws_bridge.py`) but
   need server-side triggers (tmux-attach-child exit / bridge failure) not cleanly
   forced from a WS client — left source-derived, not live-triggered.

**Divergences from source: none material.** The `terminal_attach.py` /
`ws_bridge.py` / `control_bridge.py` docstrings were accurate. Only precision adds:
(1) `101` upgrade precedes the terminal lookup; (2) control vs pty seed sizes differ
(mechanism), both raw VT.

**Net for the build:** the future attach client feeds `{tid}`'s binary frames
straight to `vt_write`, sends keystrokes/DA-DSR as binary + resize as JSON text,
branches its reconnect loop on 4404/4405/4500, and treats re-attach as a
current-screen repaint (no gap-fill needed). No transport negotiation, no tmux
parser.
