# Framework

The client framework choice: gpui (locked) vs. React/TS over Tauri (rejected).
Owns the rationale, the reconnaissance summary, the residual spike items, and
the framework-specific seams the other specs reference.

**Status:** Draft, 2026-06-23; lock **re-pressure-tested 2026-07-07**. Locked at
gpui per capability map decision D.
**Depends on:** the capability map's decision D (resolved: gpui).
**Feeds:** every other spec — they cite "framework divergent points" and rely
on the gpui substrate decisions pinned here.

---

## 1. Decision: gpui (LOCKED — re-pressure-tested 2026-07-07)

**Resolved: Lens is built on gpui.** Originally locked 2026-06-23; the lock was
re-pressure-tested 2026-07-07 against a fresh read of omnigent's shipped `web/`
client (capability map §0.9; memory `omnigent-web-app-state-2026-07`). That read
established the **fork path is dead** — the `web/` client is single-server,
single-warm-stream, chat-shaped, structurally incompatible with a fleet
supervisor — so the live question is purely **greenfield all-gpui vs
React/TS-on-Tauri**. Four inputs, ordered by load:

1. **N-warm-streams at fleet scale — the load-bearing pillar.** Lens's thesis is
   every session kept *warm* off-thread (zero switch latency, cards always live —
   not coarse badges). In gpui that's N `ActiveSession` actors + bounded channels
   + a foreground `SessionStore` replica: cheap, native paint, no DOM. In a Tauri
   webview it's N live React subtrees / Monaco / xterm-webgl contexts resident at
   once — a Chromium-memory ceiling, and precisely the limitation the `web/`
   client already documents (no list virtualization, one-warm-stream-at-a-time by
   construction). Picking Tauri means adopting the exact ceiling Lens exists to
   beat.
2. **The all-Rust, no-IPC win.** `lens-client`'s typed Rust enums flow straight
   into the UI — no IPC boundary, no JSON re-serialization at the seam. Under
   Tauri every `ServerStreamEvent` (or reduced `StreamUpdate`) crosses to the
   webview as JSON and must be **re-typed in TS** — two type systems, and the
   `web/` client's own hand-ported, server-parity-*untested* SSE parser
   (`web/src/lib/sse.ts:6-9`) is what you'd inherit. `lens-client` is already
   feature-complete (~137 tests); its value is realized fully only all-Rust.
3. **Greenfield removes migration cost.** No existing app to port (fork is off
   the table regardless).
4. **Widget risk — where the fresh read cuts *against* gpui, and why the markdown
   spike is now the gate.** The recon (§2) retired terminal/diff/board risk. But
   the fresh read shows the `web/` client ships mature, tested React components
   for the two *un-spiked* gpui residuals — streaming markdown (Streamdown, §4.1)
   and JSON-Schema elicitation forms (§4.3) — plus Monaco diff+comments and xterm.
   A Tauri path could **lift these as components**, collapsing gpui's residual
   build risk toward zero. This is the one honest advantage of the React
   alternative. It does **not** outweigh pillars 1–2, but it means the gpui lock
   is only as safe as the **markdown-renderer spike (§4.1)**: if that comes in
   cheap, the React case largely collapses; if it's genuinely hard, the trade
   re-opens. **De-risk §4.1 before treating this lock as truly closed.** (The
   recon itself was a read of the three apps, summarized in §2; §2 *is* the
   record.)

**Also weighed (and outweighed, not dismissed):** iteration velocity + ecosystem
favor React/TS (vast ecosystem, hot-reload, mature devtools) — material for a
solo builder, and the strongest *non-widget* argument for Tauri. Judged
outweighed by pillars 1–2.

**Bridge-webview risk is gone** — the Bridge (capability map §0.6) is rebuilt
native, no webview.

---

## 2. Reconnaissance summary

The GPUI reconnaissance cloned three apps at HEAD on 2026-06-04 and read
them in parallel via subagents: **Arbor** (`penso/arbor`, MIT), **Paneflow**
(`ArthurDEV44/paneflow`, GPL-3.0 — ideas only, reimplement), **gpui-flow**
(`pacifio/gpui-flow`, MIT). What they proved, and what Lens reuses:

### 2.1 The async-stream → state → render bridge

GPUI has no built-in SSE/WS. Arbor's pattern (the load-bearing template):

- A dedicated **OS thread** holds the blocking socket (`tungstenite` or
  `reqwest` blocking).
- Writes parsed events into an `Arc<...>` of atomics / an mpsc channel.
- A **UI-thread poller** drains it via `this.update(cx, |this, cx| { …mutate…;
  cx.notify() })` (gpui's reactive notify).
- All I/O is in `cx.background_spawn`, never on the UI thread.

This maps cleanly onto Lens's `ActiveSession`-actor→bounded-channel→foreground
`SessionStore` replica→subscribers model (state model §8). The state model's §14
"framework divergence notes" calls out this single crossing as the one
framework-specific point — gpui's `cx.spawn` + entity update on the foreground
executor is the implementation.

### 2.2 Terminal widget

`alacritty_terminal` (Zed's fork) + `portable-pty`. Bytes → bump a
`generation` counter → on render walk the grid → shape text (cached) → paint
quads+runs in a `canvas()` element. Arbor's `arbor-terminal-emulator/` is
the template (MIT — copyable into Lens with attribution).

Lens's terminal surface (workspace doc §9) uses this pattern. The ring
buffer (workspace doc §9.2) is a Lens-local addition layered on top.

### 2.3 Diff widget

Computes diffs client-side from two strings via `gix-diff` / `imara-diff`
Histogram → flat `Vec<DiffLine>`. Renders with native `uniform_list`
virtualization. Syntax highlight via `syntect`, **precomputed + cached in an
`Arc`, not per-render** (this is the load-bearing performance fix — a naive
per-render highlight is slow on large diffs).

Arbor's `arbor-gui/src/diff_engine.rs` `build_side_by_side_diff_lines` is the
template (MIT — copyable). The workspace doc's §4 diff computation is this
pattern, pinned.

### 2.4 Canvas / board

`gpui-flow` (React-Flow clone for GPUI) shows pan/zoom + viewport culling for
a large card canvas. **Lens's board is NOT a free-form canvas** (capability
map: ordinal slots, not pixels — the shell spec §4.1 is the keystone fix).
Lens's board is bounded + ordinal (not a free-form/infinite canvas), so it's a
responsive reflow grid of cards — **simpler than a free-form canvas**. No
node-graph edges/handles;
`gpui-flow`'s pan/zoom is not strictly needed, but its viewport culling
pattern is a reference if the board grows large.

### 2.5 Markdown security boundary

Paneflow's `markdown/security.rs` (GPL — ideas only, reimplement) is the
standout: treats markdown as **hostile-by-default** (it renders agent/LLM/tool
produced content, exactly Lens's transcript case).

- `validate_link_url` allows only `http(s)` before handing a click to the
  OS opener — blocks `file:`, `javascript:`, `data:`, `vbscript:`, bare-host,
  >8KiB.
- `validate_image_ref` guards path-traversal + scheme injection + symlink
  escape + remote-beacon images.

**Lens's transcript renderer must carry this boundary** before any
click-to-open or image-load ships (transcript doc §6.1).

### 2.6 Reusable widgets

Paneflow built shared primitives; confirms the "build one reusable
text/scroll component" recommendation. A custom `scrollbar` is itself a
small gpui-gap tell. Lens inherits the guideline to build custom primitives
once, not per surface.

### 2.7 Tiling layout engine (deferred)

Paneflow's `src-app/src/layout/` (~1170 LOC): N-ary
`LayoutTree { Leaf | Container{direction, children, …} }`, ratio-based flex,
recursive render, drag-to-resize dividers. Useful later if Lens wants split
terminals / side-by-side diff+editor; the shell spec's working area
(shell §7.2 — tab + split) is a fixed-4-zone layout for now and doesn't need
the full tiling tree.

---

## 3. GPUI distribution

Two distribution choices:

- **Published `gpui = "0.2.2"` from crates.io** (Arbor's path; easy on-ramp).
- Git-pin `zed/zed` or a Lens-owned fork (Paneflow / gpui-flow's path; needed
  when you must patch gpui).

**Lean: crates.io by default.** Forking is a one-way door — only take it when
a fix upstream doesn't land. The markdown-append fix (§4) is the likely
trigger; re-evaluate at the spike.

---

## 4. Residual spike items

The recon retired the terminal/diff/board risk; **two** open items remain
load-bearing — markdown (§4.1) and the JSON-Schema form renderer (§4.3):

### 4.1 Markdown rendering (the lock-gating spike item)

Per §1 pillar 4, this is the spike that actually closes (or re-opens) the gpui
lock — the fresh `web/` read means the React alternative already has a solved
streaming-markdown component, so gpui only holds if this residual is cheap.

**Ecosystem survey DONE (2026-07-07, verified vs crates.io) — the residual
shrank, and it strengthens the gpui lock (§1.4).** Liftable Apache-2.0,
gpui-native building blocks now exist:

- **Parser: `pulldown-cmark` 0.13.4 (MIT)** — event stream → gpui elements,
  confirmed the right base (not comrak's AST/HTML-emitter path). Alternates
  `comrak` 0.53, `markdown`/markdown-rs 1.0 reduce no UI risk.
- **Streaming safe-prefix: `mdstitch` 0.1 (Apache-2.0, framework-agnostic)** —
  closes unterminated tokens (`**bold`→`**bold**`) *before* pulldown on each
  accumulated chunk. This is the §5 safe-prefix well-formedness problem as a
  **liftable dep, not app code**. (Used by `tahoe-gpui` for streaming markdown —
  another gpui reference alongside Paneflow.)
- **Rendering + highlight: `gpui-component` 0.5.1 (Apache-2.0 → LIFTABLE)** —
  ships a native Markdown component with **tree-sitter** syntax highlighting,
  plus virtualized `List`/`Table` (relevant to §4.1d virtualization) and form
  inputs (relevant to §4.3). This is the input that most changes the picture:
  the widget-risk axis where the React alternative was genuinely ahead (§1.4) is
  now largely matched *on the gpui side*, permissively licensed, no IPC cost.
- **Reference-only (GPL): Zed `crates/markdown`, Paneflow** — architecture
  references, not liftable.

**Net residual is integration + the Lens-specific parts, not a from-scratch
renderer:** (a) does `gpui-component`'s markdown component accept **incremental
updates with stable element identity** (no remount on append)? — **SPIKED
2026-07-07 → PARTIAL; see the verdict block below**; (b) the link/image
**sanitization boundary** (§2.5),
still Lens-owned (reimplement from Paneflow's *spec*); (c) **variable-height
virtualization** (§4.1d) — evaluate `gpui-component`'s virtualized `List`.
**Caveat:** `gpui-component` is a large, young (0.5.x) dep that pins its own gpui
version — check compat with the §3 gpui pin, and prefer **vendoring just the
markdown module** (Apache-2.0 permits it) over taking the whole 60-component
library.

**Spike verdict (2026-07-07) — PARTIAL; the lock HOLDS.** Full findings:
[`docs/spikes/2026-07-07-markdown-streaming.md`](../spikes/2026-07-07-markdown-streaming.md).
gpui-component 0.5.1 builds on **gpui 0.2.2 (= the §3 pin — no reconciliation)**;
the *architecture* passes — state is a retained `Entity` keyed by `ElementId`
(no remount), re-parse is async/debounced off the render path (probe: flat
**~25µs/frame across a 17KB doc**, no O(n) reparse). **But three hardcoded module
behaviors break naive streaming:** (1) a 200ms *trailing* debounce that resets on
every update (`text_view.rs:628`) → fast streams render nothing until a pause;
(2) `clear_selection()` on reparse (`:610`) → selection lost mid-stream;
(3) `list_state.reset()` on content change (`node.rs:1123`) → **scroll jumps to
top on every render** (violates transcript §5). All three are single-spot fixes
in the vendorable module → this **confirms the "vendor just the markdown module"
path** over the raw dep or a from-scratch renderer. Safe-prefix/mdstitch deferred
(needs Rust 1.95; lower priority given the debounce hides intermediate states).

GPUI has no first-class markdown renderer *in the framework itself*. Paneflow
**forked gpui** for a markdown-append fix **and** built its own `pulldown-cmark`
→ element view. Two implications for Lens (the fallback if `gpui-component`'s
component can't stream with stable identity):

- **Budget a hand-rolled `pulldown-cmark` → gpui element renderer.** A
  naive dep on a "gpui markdown" crate will not give stream-append support;
  the transcript doc §5 (progressive markdown, safe-prefix) requires a parser
  that handles incremental input.
- **The link/image sanitization boundary** (§2.5) is built into the
  renderer, not a separate post-process pass.

**The spike**: stand up the renderer against a captured SSE stream and verify
(a) safe-prefix streaming works, (b) the link/image boundary holds, (c)
in-place diff with stable identity doesn't remount on finalize, (d) the
transcript's variable-height virtualization holds (transcript §16/§19 —
`uniform_list` is uniform-height, so this needs a custom virtualizer).

**Fallback ladder if the markdown spike blocks** (escalate only as far as
needed): (1) `pulldown-cmark` → gpui element renderer on published gpui; (2)
git-pin a Lens fork with the markdown-append fix (Paneflow's path, §3); (3)
last resort, degrade to non-streaming finalize-only markdown (render plain text
live, swap to formatted markdown on item finalize) — uglier but unblocks ship.

### 4.3 JSON-Schema elicitation form renderer (second spike)

The permissions form-mode elicitation (permissions §3) renders an arbitrary
`requested_schema` JSON Schema as a gpui input form. gpui has no JSON-Schema
form primitive, so this is a hand-rolled renderer (string/number/enum/boolean/
nested-object fields → gpui inputs, with validation). **Un-spiked** — but the
2026-07-07 survey shrinks it: `gpui-component` 0.5.1 (Apache-2.0) already
provides the field inputs (Input/Select/Checkbox/NumberInput/Switch/DatePicker),
so the residual is the **runtime schema→inputs mapping**, not building inputs.
(`gpui-form` 0.5.1 MIT/Apache derives forms from Rust *structs* at compile time —
wrong shape for a runtime-arbitrary schema, but confirms the input primitives.)
Spike:
render the common omnigent elicitation schemas and confirm submit produces a
valid `ElicitationResult.content`. Fallback if a schema is too complex to render
natively: fall back to the url-mode approval page (permissions §3) or a raw
key/value editor.

### 4.2 Other GPUI gaps (carried, smaller)

- **No granular subscriptions.** `cx.notify()` re-renders the whole view. The
  state model §14 mitigates by using per-entity state (`Entity::observe`);
  the shell + surface specs lean on per-session `SessionStore` entities, not
  a global store, to avoid invalidating the foreground render.
- **Custom drags need explicit `window.refresh()`** on mouse-move. gpui only
  auto-repaints during `cx.has_active_drag()`. Lens's board drag uses
  ordinal slots (no drag-physics), so this gap is less load-bearing than it
  would be for a free-form canvas.
- **Roll-your-own scrollbar.** The built-in was insufficient for Paneflow;
  expect to build a custom scrollbar primitive, reused across surfaces.
- **`canvas()` prepaint to capture laid-out pixel bounds** during interaction
  — confirmed idiom across both Arbor and Paneflow (drag clamping, container
  sizing).

---

## 5. Framework-specific seams the other specs reference

Each spec has a "framework divergence" section. What each one owns vs. here:

| Spec | Owns there | Resolved here |
|---|---|---|
| typed client | (no framework impact) | Blocking reader threads + `std::sync::mpsc`; no tokio requirement |
| state model §14 | State primitive (gpui `Entity::observe` vs alternative store); the channel→UI crossing (`cx.spawn` + entity update) | gpui's per-entity notify + `cx.spawn` is the foreground replica implementation |
| application shell §17 | The board (ordinal-slot responsive reflow vs a free-form canvas) | §2.4 of this doc — confirmed ordinal board is *simpler*, not harder, in gpui |
| transcript §19 | Progressive re-render (stable-identity in-place diff); markdown library; virtualization | §4.1 markdown spike; `uniform_list` is gpui-native **but uniform-height only** — the variable-height transcript needs a custom virtualizer (spike, §4.1d) |
| workspace §9 | Terminal widget | §2.2 — `alacritty_terminal` + `portable-pty`, Arbor template (MIT) |
| workspace §4 | Diff widget | §2.3 — `imara-diff` + `syntect` cached, Arbor template (MIT) |
| permissions | (no special widgets — form renderer uses gpui inputs; JSON-schema form renderer is a one-off build) | — |
| sub-agent topology | (no special widgets — rail/tree uses gpui list primitives) | — |
| server lifecycle | (no widgets — backend) | — |

---

## 6. Non-goals of this document

- Not the gpui *API tutorial* — that's the gpui docs.
- Not the build system — cargo workspace is implementation detail, spec'd
  in the typed client's §1 (where it's load-bearing for the seam).
- Not an exhaustive React/TS-over-Tauri comparison. The decision is locked
  (re-pressure-tested 2026-07-07, §1); the rejected alternative is documented
  only where it sharpens why gpui won — the two headlines are the fleet-scale
  N-warm-streams ceiling of a webview (§1.1) and the IPC/type-loss seam (§1.2).
  The one axis where the alternative is genuinely stronger (liftable widgets) is
  recorded in §1.4 and gated on the §4.1 spike, not buried.
- Not a performance budget. Benchmarks are a verification-pass concern
  (capability map §0.8).

---

## 7. Open questions

- **gpui version pin** — `0.2.2` from crates.io is a starting point; revisit
  at first build. Newer gpui releases may pass (Paneflow forked for the
  markdown-append fix); whether Lens can stay on a published version or
  needs a fork is gated on the §4.1 spike.
- **Hot-reload of themes** — the shell §18 ships hot-reload; gpui's support
  for swapping a `Theme` struct at runtime is assumed. Verify at build.
- **Window resize behavior for the focused-session window** — shell §7.1's
  "boards stay visible (shrunk)" needs the layout to reflow cleanly on
  resize; ordinal slots (shell §4.1) make this deterministic, but gpui's
  actual resize behavior at the element level needs a verification pass.
