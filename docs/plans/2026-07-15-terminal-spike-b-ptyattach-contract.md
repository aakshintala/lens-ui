# Terminal Spike B — omnigent PTY-attach Contract (investigation protocol)

> **Ownership:** Phases **B0** and **B2** are Claude/Opus work (investigation +
> live divergence analysis — judgment that feeds the design). Phase **B1** (the
> mechanical capture harness) is dispatched to **composer** *after* B0, because
> its exact frame encodings are B0's output. This is a staged investigation, not a
> linear code plan — that's why B1's harness spec is finalized from B0 rather than
> pre-written here.

**Goal:** Establish the real WebSocket terminal-attach wire contract against a
running omnigent 0.5.1 — attach handshake, input framing, output framing, resize,
and same-resource reconnect / output-gap behavior — verified against **live bytes**,
not just source. Produce a contract doc + a raw byte corpus that (a) de-risks the
future attach layer and (b) feeds Spike A's fixtures.

**Architecture:** Read the omnigent attach route + tmux bridges to document the
expected shape (B0). Build a throwaway `tokio-tungstenite` harness that attaches,
drives a shell command, and dumps every frame to a corpus (B1). Run it live,
capture the required scenarios, and analyze source-vs-wire divergence (B2).

**Tech Stack:** omnigent 0.5.1 (sibling source `../omnigent` @ `08285468`; run via
the `installing-omnigent-from-source` skill), Rust throwaway harness
(`tokio-tungstenite`, `reqwest` for REST setup), a host daemon for a live runner.

## Global Constraints

- The contract doc must match **live bytes**; every place source and wire diverge
  is flagged explicitly. If terminal attach is broken/unsupported on 0.5.1, that is
  the finding — surface it, don't work around it.
- Harness is throwaway, under `spikes/terminal-attach/`, excluded from the lint
  gate. It is **not** `lens-client` and does not touch it (REST setup may shell out
  to `curl` or a tiny `reqwest` client — keep it self-contained).
- Pin: omnigent **0.5.1 / `08285468`** (matches `vendor/omnigent-0.5.1/OMNIGENT_PIN`).
- Capture corpus lands under `docs/spikes/captures/2026-07-15-pty-attach/`, same
  golden-capture discipline as the SSE captures.

---

## Source anchors (verified paths, sibling checkout `../omnigent`)

- **WS route:** `omnigent/server/routes/terminal_attach.py` —
  `@router.websocket("/sessions/{session_id}/resources/terminals/{terminal_id}/attach")`
  (line ~131). Accepts the socket, then `_shuttle_ws_frames(websocket, runner_ws)`
  (~line 202) — i.e. the app server is a **proxy** to a runner-side WS.
- **Bridges:** `omnigent/terminals/ws_bridge.py` (`bridge_tmux_pty_to_websocket`),
  `omnigent/terminals/control_bridge.py` (`bridge_tmux_control_to_websocket`) — a
  **tmux control channel + a pty data channel**, both bridged onto the WS.
- **Runner/pane side:** `omnigent/inner/terminal.py`, `omnigent/terminals/registry.py`,
  `omnigent/terminals/pane_reaper.py` (pane lifecycle / reaping → informs reconnect).
- **Terminal resource REST** (create/list/get/delete a terminal on a session): the
  routes `lens-client` already models (STATUS "resources/terminals"); confirm the
  create path + what fields yield the `{terminal_id}` used in the WS URL.

---

## Phase B0 — Source-read → documented contract shape (Claude)

Read the anchors above and answer each of these in a contract-shape doc. These are
the questions the live capture (B2) then confirms or refutes.

1. **Attach URL + scheme + prefix.** Full path (is it `/v1/...`?), `ws` vs `wss`,
   and how `{session_id}`/`{terminal_id}` are obtained (which REST call mints the
   terminal resource, what host/runner prerequisite must be satisfied).
2. **Auth.** How the WS handshake authenticates — bearer header, query-param token,
   cookie? (Check the `auth_provider`/dependency on the route + how
   `/sessions/updates` at `sessions.py:14930` does it, for contrast.)
3. **Output framing.** Are server→client frames **binary** (raw PTY bytes) or
   **text/JSON-enveloped**? Is the tmux **control** channel multiplexed onto the
   same socket as the **pty** channel, and if so how are they discriminated (frame
   opcode? a JSON `type`? a length-prefixed mux)? This is the crux — read
   `ws_bridge.py` + `control_bridge.py` + `_shuttle_ws_frames` carefully.
4. **Input framing.** How client→server keystrokes reach the PTY — raw bytes vs a
   JSON envelope. **This is the channel the `on_pty_write` back-channel (DA/DSR
   replies) uses**, so confirm it carries arbitrary bytes.
5. **Resize.** The resize control-message shape (JSON `{type:"resize",cols,rows}`?
   a tmux control command? a dedicated frame?).
6. **Reconnect / persistence.** tmux implies a **persistent pane**. On re-attach to
   the same `{terminal_id}`: does the server replay scrollback / current screen?
   Is there an output gap (bytes emitted while detached — lost or buffered)? What
   does `pane_reaper` do to a detached pane and after how long?

**B0 deliverable:** `docs/spikes/2026-07-15-pty-attach-contract.md` §"Expected
(from source)" — the documented shape with the six answers, each citing
file:line. Explicitly mark anything the source leaves ambiguous (→ B2 resolves it).

---

## Phase B1 — Capture harness (composer, dispatched after B0)

A throwaway Rust binary `spikes/terminal-attach/` that exercises the contract and
dumps a corpus. **Composer is handed B0's concrete frame encodings in the dispatch
prompt** — the harness below is the requirements skeleton.

**Files:** Create `spikes/terminal-attach/Cargo.toml`, `src/main.rs`. Add
`spikes/terminal-attach` to root `Cargo.toml` `exclude`.

Harness requirements:
- [ ] REST setup: given a base URL + auth token (env vars), ensure a session with a
      live host/runner and **create a terminal resource**; obtain `{session_id}` +
      `{terminal_id}`. (Shell to `curl` or a minimal `reqwest` call per B0's REST shape.)
- [ ] Open the WS attach per B0 (correct scheme/prefix/auth). Log the handshake
      response.
- [ ] **Dump every frame** (direction, opcode text/binary, raw bytes hex+utf8-lossy,
      wall-clock offset) to `docs/spikes/captures/2026-07-15-pty-attach/*.frames.jsonl`.
- [ ] Drive scenarios in sequence, each demarcated in the dump:
  - **attach** — capture whatever the server sends on connect (initial screen / replay?).
  - **input** — send a shell command (e.g. `printf 'MARKER_A\\n'; ls -la\\n`) per the
    input framing; capture the echoed + output frames.
  - **resize** — send a resize (e.g. 80×24 → 120×40) per B0; capture the response.
  - **forced transient drop + same-resource reconnect** — hard-close the socket
    mid-output (e.g. right after launching a command that emits for a few seconds),
    wait, re-attach to the **same** `{terminal_id}`, and capture what the server
    sends on re-attach (replay? current screen? nothing?). Emit a unique marker
    before the drop and another after, so B2 can measure the **output gap**.
- [ ] Print a run summary (frames per scenario, bytes, any errors/closes with codes).
- [ ] Commit.

---

## Phase B2 — Live run + divergence analysis (Claude)

- [ ] Stand up omnigent 0.5.1 + a host daemon (per `installing-omnigent-from-source`;
      STATUS confirms `omnigent host …` brings a runner online). Verify
      `omnigent --version` == 0.5.1.
- [ ] Run the B1 harness; collect the corpus.
- [ ] Analyze against B0's "Expected (from source)": fill a §"Actual (from wire)"
      with, for each of the six B0 questions, the confirmed shape — and **flag every
      divergence** from source.
- [ ] Characterize reconnect: does re-attach replay? Is there an output gap, and if
      so is it a **persistent** gap (bytes truly lost) or a **transient** one
      (buffered/redrawn)? This directly informs the "possible-output-gap marker"
      vertical-proof requirement.
- [ ] If attach is broken/unsupported on 0.5.1 → write that up as the finding and stop.
- [ ] Finalize `docs/spikes/2026-07-15-pty-attach-contract.md` (Expected + Actual +
      divergences + reconnect characterization) and commit the corpus.

---

## Exit criteria (the deliverable)

A contract doc grounded in live bytes that answers, for the future attach layer:
- the exact attach URL / scheme / auth;
- the exact input & output frame encodings (incl. control-vs-pty multiplexing);
- the resize message shape;
- the same-resource reconnect behavior + whether an output gap exists and its kind.

Plus the raw byte corpus, which becomes Spike A's realistic fixtures.

## What is out of scope (do not build)

- Any `lens-client` terminal WS client (`terminal.rs`/`tungstenite` in the real
  crate) — that's the future build this spike de-risks.
- The `on_pty_write` → WS wiring, the actor, the render layer.
- Modeling the contract as typed Rust — B produces a *documented* contract + corpus;
  typing it is the build.

## Handoff back

B0 + B2 are mine; I hand composer the concrete B1 spec after B0, and I review the
harness output. The whole spike is throwaway except the contract doc + corpus (kept).
