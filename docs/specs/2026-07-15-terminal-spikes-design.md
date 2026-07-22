# Design — Terminal-VT spikes: render viability + omnigent PTY-attach contract

**Status:** design approved 2026-07-15. Scopes the two de-risking spikes that must
land *before* the terminal render layer, threading model, or `crates/lens-terminal`
boundary are designed.

## Context

The mechanical half of the terminal-VT workstream is done: `libghostty-vt` is
vendored, builds-from-source (patched `zig@0.15`), and links inside Lens (bytes →
cell proven by `spikes/libghostty-link`). Commits `ae1f385`/`014f9a9`/`e155230`/
`fa268db` on `terminal-ws`, unpushed. See memories `terminal-vt-vendored-executed`,
`terminal-vt-adoption-model`.

The design half (GPUI render layer + omnigent PTY-attach on the safe
`Terminal`/`RenderState`/`Cell` API) has two load-bearing unknowns. Everything else —
the `lens-terminal` crate shape, the off-thread actor wiring — is derivable
engineering once these are answered, and the actor pattern is already proven by the
state-model single-writer-actor + gpui-replica design. So we resolve the two unknowns
first, as two **independent, parallel, throwaway** spikes.

The safe API we design against (`vendor/libghostty-rs/libghostty-vt`):
`Terminal::new(TerminalOptions{cols,rows,max_scrollback})`, `vt_write(&[u8])`,
`on_pty_write(|term,data| …)` (sync back-channel, must not block),
`RenderState::update(&terminal, |update| …)` → `Snapshot`
(`cols`/`rows`/`dirty()` bitset, `cursor_*`, `colors()`), `RowIterator` →
`CellIterator` → `Cell` (`codepoint`/`has_text`/`wide`/style/sgr).

## Non-goals (deferred until both spikes land)

- `crates/lens-terminal` deep-module boundary (Q4) — no Ghostty type escapes.
- Threading / foreground-safety wiring (Q3) — `Terminal` off-thread, snapshot crosses
  to paint; reuses the state-model actor+replica pattern.
- Cursor rendering, selection overlay, scrollback scrolling (render-contract features,
  not lock-decision features).
- The kept standalone GPUI demo host — a deliberate build on the *locked* render
  contract, not a spike.

---

## Spike A — Render/paint viability (throwaway probe)

**Question:** does Ghostty's dirty bitset buy partial repaint under GPUI's paint
model, or is the render contract built around full-snapshot repaint?

**Harness:** standalone GPUI 0.2.2 window. Byte fixtures → `libghostty-vt` `Terminal`
→ `RenderState` snapshot → native GPUI paint (background quads + shaped glyph runs).
No `lens-terminal` crate, no actor, no input handling. Throwaway scaffold; the
**cell → (background quad + shaped glyph run) paint mapping is written to be liftable**
into the real render contract.

Native GPUI only — **not** gpui-component. A fixed cell grid is neither markdown nor a
form; gpui-component won those surfaces and does not apply here.

**Fixtures (perf-relevant only):**
- full-screen redraw (`clear` / alt-screen swap) — the all-dirty worst case;
- small partial update (a few rows, log-tail / typing) — the typical case;
- wide chars (CJK/emoji) + SGR color runs — stresses double-width handling and per-row
  glyph-shaping cost / cache-keying.

Synthetic to start; upgraded to Spike B's real captured corpus when it lands (the two
spikes are independent — Spike A does not block on B, because `libghostty` parses any
bytes into cells and the render side only consumes the cell grid).

**Exit criteria — the spike produces this decision:**
- full-repaint p95 ≤ 8.3ms across realistic grid sizes up to ~200×50 (plus one
  oversized stress grid) → dirty tracking is *optional*; render contract =
  full-snapshot repaint (simpler);
- full-repaint over budget but cached-partial (per-row shape cache keyed by row
  content) ≤ 8.3ms on the typical case → dirty tracking is *load-bearing*; contract
  built around the bitset;
- even cached-partial misses budget → deeper problem (GPU path / different approach),
  escalate.
- **input → first-paint measured separately** (distinct constraint).

Measured: release build, Apple Silicon (matches the lens-client bench baseline).

**Deliverable:** findings doc with the decision + numbers + the liftable paint-mapping
code. Scaffold discarded.

---

## Spike B — omnigent PTY-attach contract (source → live capture)

**Question:** what is the real WS attach/input/output/resize/reconnect wire shape, and
does terminal attach even work against pinned omnigent 0.5.1? The WS attach has never
been built or driven on our side (`lens-client` Plan 7 deferred it — no `terminal.rs`/
`tungstenite`), and `terminal.activity` surfaced over **SSE, not WS** in the recapture,
so the WS contract is entirely unverified against a running server.

**Step 1 — source-read.** Document from omnigent source (`server/routes/
terminal_attach.py`, `terminals/ws_bridge.py`, `terminals/control_bridge.py`,
`server/routes/sessions.py`): attach handshake (URL, auth), input framing (raw bytes
vs enveloped), output framing, resize control-message shape, reconnect / same-resource
semantics, and where the `on_pty_write` back-channel (DA/DSR replies) goes on the wire.

**Step 2 — live attach + capture.** Throwaway `tungstenite` harness (not `lens-client`).
Stand up omnigent 0.5.1, attach a real terminal, run a shell command, capture the raw
byte corpus:
- input frames + output frames (→ Spike A's real fixtures);
- a resize round-trip;
- a **forced transient WS drop + same-resource reconnect**, observing whether the
  server retains the engine / replays / leaves an output gap (the persistent
  "possible-output-gap marker" constraint).

**Exit criteria:** a contract doc that matches *live bytes*, not just source — every
source/wire divergence flagged, reconnect gap behavior characterized. If attach is
broken or unsupported on 0.5.1, that is itself the finding, surfaced now rather than at
build time.

**Deliverable:** contract findings doc + raw byte corpus under `docs/spikes/captures/`,
same golden-capture discipline as the SSE work.

---

## Shared conventions

- Both harnesses live under `spikes/` (code) + `docs/spikes/` (findings/corpus), both
  throwaway (like `spikes/libghostty-link` and the transport-stability harness).
- Neither enters the production lint gate (vendored crates + spikes are excluded; the
  real `crates/lens-terminal` will be gated later).
- Surviving constraints (from the superseded roadmap, still binding): never block the
  GPUI foreground thread; UI never panics (errors are modeled values); typed
  end-to-end (no generic WebSocket / `serde_json::Value` / Ghostty type escapes);
  perf is the prime objective (benchmark every layer).

## What these spikes unblock

Together they de-risk and feed: the render contract (Spike A's decision), the
`crates/lens-terminal` deep-module boundary (Q4), the off-thread threading model (Q3),
and the first accepted vertical proof (real pinned server: attach/input/output/resize/
forced-drop/same-resource-reconnect/gap-marker — Spike B characterizes the wire it runs
on). None of those are designed until both spikes land.
