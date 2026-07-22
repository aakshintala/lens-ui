# Handoff — Terminal render layer + omnigent PTY-attach (design pass)

Resume artifact for the **design half** (tasks 3–4) of the terminal-VT workstream.
The mechanical half (vendored binding + build + link proof) is **done**; this is the
"now design the real thing" checklist. Start a fresh session.

## State (one line)

The vendored `libghostty-vt` builds-from-source and links inside Lens (proven by
`spikes/libghostty-link`: bytes → cell). Tasks 1–2 landed on `terminal-ws` (commits
`ae1f385`/`014f9a9`/`e155230`/`fa268db`, **unpushed**). Next = brainstorm → design →
plan the **GPUI render layer** + **omnigent PTY-attach** on the safe API.

## Read first

- Memories: `[[terminal-vt-vendored-executed]]` (what's built + wiring gotchas),
  `[[terminal-vt-adoption-model]]`, `[[zig-ghostty-macos26-scissor]]`.
- Render/streaming spike learnings (do they transfer? — see open question 1):
  `[[transcript-virtualization-spike-2026-07]]`, `[[markdown-streaming-spike-2026-07]]`,
  `[[large-transcript-latency-spike-2026-07]]`.
- `docs/STATUS.md` (SSOT, ACTIVE block) + the **superseded** design/roadmap
  (`docs/specs/2026-07-14-terminal-workstream-design.md`,
  `docs/plans/2026-07-15-terminal-workstream-roadmap.md`) — the VT-adoption sections are
  dead, but the **model-independent** parts still hold (see Constraints below).
- Process: this is creative/design work → **`superpowers:brainstorming` FIRST**, then
  `superpowers:writing-plans`. Do not jump to code.

## The safe API you're designing against (`vendor/libghostty-rs/libghostty-vt`)

- **Feed:** `Terminal::new(TerminalOptions{cols:u16, rows:u16, max_scrollback:usize})`,
  `terminal.vt_write(&[u8])`. Grid coords are `PointCoordinate{x:u16, y:u32}`.
- **Render (double-buffered):** `RenderState::update(&terminal, |update| …)` →
  `Snapshot` exposing `cols()/rows()/dirty()` (`Dirty` bitset), `cursor_viewport()`,
  `cursor_visible/blinking/visual_style/color`, `colors()`. Iterate via
  `RowIterator`→`RowIteration` (`dirty()`, `raw_row()→Row`, `selection()`) and
  `CellIterator`→`CellIteration`→`Cell` (`codepoint()`, `has_text()`, `wide()→CellWide`,
  style/sgr). **Dirty tracking is first-class** → design partial repaint around it.
- **PTY output back-channel:** `Terminal::on_pty_write(|term, data| …)` — a callback the
  terminal invokes **synchronously during `vt_write`** for responses it must send back to
  the PTY (e.g. DA/DSR replies). **Must not block.** This is the seam to the omnigent WS
  uplink.

## Open design questions (resolve in brainstorming, don't assume)

1. **Terminal grid ≠ scrolling transcript.** The transcript-virtualization + markdown
   spikes solved a *variable-height, append-mostly, bottom-anchored* list. A terminal is a
   *fixed cell grid + scrollback* with dirty-row invalidation. Verify which spike learnings
   actually transfer (retained id-keyed state? `list()` bottom-anchoring?) **before** locking
   the render contract — don't cargo-cult them.
2. **RenderState → GPUI paint mapping.** How does the dirty-row snapshot become GPUI
   elements/quads/glyphs? Per-cell quad vs shaped run? Cursor + selection overlay? Wide-char
   + scrollback viewport handling.
3. **Threading / foreground-safety.** `vt_write` (parse) and `on_pty_write` (sync callback)
   must run **off** the GPUI foreground thread. Where does the `Terminal` live, how does a
   render snapshot cross to the paint thread, and how does WS I/O feed `vt_write`?
4. **Crate home + typed boundary.** The link-proof is a **throwaway spike**
   (`spikes/libghostty-link`) — the real home is a new first-party **`crates/lens-terminal`**
   that wraps `libghostty-vt` so **no Ghostty type escapes** (typed-boundary rule). It enters
   the production lint gate; scope its deep-module interface.
5. **Omnigent PTY-attach contract.** WS bytes → `vt_write`; `on_pty_write` → WS send.
   `lens-client` already has (or plans) terminal list/get/create/delete + authenticated WS
   attach (STATUS "WS terminal attach client"). The terminal WS truth is partly outside
   `vendor/omnigent-0.5.1/openapi.json` — the audit leaned on omnigent
   `server/routes/terminal_attach.py`, `terminals/ws_bridge.py`, `terminals/control_bridge.py`,
   `server/routes/sessions.py`. Confirm the attach/input/output/resize/reconnect shape live.

## Constraints that survive the pivot (from the superseded roadmap — still binding)

- Never block the GPUI foreground thread (parse/I/O/reflow/alloc off-thread).
- UI never panics — errors are modeled values.
- Typed end-to-end: no generic WebSocket, `serde_json::Value`, stringly dispatch, or Ghostty
  type escapes its owning boundary.
- Perf is the prime objective (p95 frame ≤ 8.3ms; measure input→first-paint separately);
  benchmark every layer.
- Owner-write / viewer-read-only; standalone GPUI demo is the first host (not `lens-ui`).
- Gated, typed, serializable inspection + fixed-capacity event ring, zero hot-path cost when off.
- First accepted vertical proof uses a **real pinned omnigent server**: attach, input, output,
  resize, forced transient WS drop, same-resource reconnect with retained engine, persistent
  possible-output-gap marker.

## Gotchas

- `spikes/libghostty-link` is throwaway proof — supersede it with `crates/lens-terminal`, don't
  build on it.
- Build prereq unchanged: `brew install zig@0.15` (keg-only; wired via `.cargo/config.toml`
  `ZIG` + the `build.rs` patch). Re-apply the 1-line `build.rs` patch on every pin bump.
- Branch `terminal-ws` is **unpushed** (4 commits ahead of `631b361`).
- The `--workspace` clippy gate line in the old roadmap is stale — the real `xtask gate` is
  scoped to production crates; vendored crates + spikes are excluded. `lens-terminal` (new)
  WILL be in the gate.
