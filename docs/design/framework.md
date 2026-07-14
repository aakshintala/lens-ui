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

The recon retired the terminal/diff/board risk. Markdown (§4.1) — including its
§4.1c/d variable-height virtualization sub-item — the transcript virtualization
(§4.1c/d), and the JSON-Schema form renderer (§4.3) are **all now spiked and
resolved**. **No load-bearing framework residual remains.** (Historical spike
detail retained in the subsections below.) The one remaining *widget* residual —
the **editable code surface** (§4.4, the File-tab editor) — is **scoped, not
spiked**, and is explicitly **not lock-gating**: it does not reopen the
gpui-vs-React decision because both substrates hit the identical remote-file /
no-LSP wall (§4.4). It is a product-scope decision, not a framework-lock spike.

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
virtualization** (§4.1d) — **SPIKED 2026-07-08 → GO on native gpui `list()`; see
the §4.1c/d verdict block below**.
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

**§4.1c/d virtualization verdict (2026-07-08) — GO on native gpui `list()`.**
Full findings:
[`docs/spikes/2026-07-07-transcript-virtualization.md`](../spikes/2026-07-07-transcript-virtualization.md).
The residual "the variable-height transcript needs a *custom* virtualizer"
(transcript §19 note 3) is **retired**: gpui's native **`list()` / `ListState`**
is a measure-and-cache variable-height virtualizer purpose-built for chat logs
(`ListAlignment::Bottom`), and it **satisfies all four transcript §16 contracts**
on the head-to-head harness — windowing (`renders ≪ N`), variable heights,
**off-screen-above anchoring** (`logical_scroll_top()` held under a height
mutation above the viewport — the true go/no-go), jump-to-bottom, plus stable
identity across recycle and markdown-component nesting (`.scrollable(false)` rows)
and the stick-don't-yank UX demo. **7/7.** `uniform_list` was simply the wrong
primitive (uniform-height); `list()` is the right one — **no custom virtualizer,
no fork, no extra dep.** gpui-component's virtualized `v_virtual_list` was tested
side-by-side and **does not fit** the transcript: it windows fine and preserves
identity, but structurally lacks bottom-anchoring and a logical-anchor readout
(1b anchor drifted, opens at top, stick-to-bottom unreliable). Net: **native
`list()` for the transcript scroll surface; gpui-component reserved for markdown
(§4.1) + the §4.3 form inputs.**

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

### 4.3 JSON-Schema elicitation form renderer — SPIKED 2026-07-08 → GO

**Verdict: GO on native gpui + `gpui-component` inputs. 6/6 probes.** Full findings:
[`docs/spikes/2026-07-08-elicitation-form.md`](../spikes/2026-07-08-elicitation-form.md).
This was **the last load-bearing un-spiked framework residual**; the framework
spike series closes.

**Ground-truth reframe (the prior framing was wrong in both directions).** The
permissions elicitation surface is **not** an arbitrary/nested JSON-Schema form:

- **"Arbitrary/nested" is not real.** MCP elicitation is a **flat object of
  primitive properties** (`string | number | integer | boolean`, `enum`,
  `oneOf:[{const}]`, optional `default`); `content` values ∈
  `str | int | float | bool | list[str] | null`. omnigent's own auto-fill helper
  (`tools/_elicitation_schema.py`) never recurses. No nesting, no arrays-of-objects.
- **The real surface is a discriminated set,** not one form: omnigent's own client
  (`web/.../ApprovalCard.tsx`) has **no general JSON-Schema renderer** — it resolves
  URL / ExitPlanMode / AskUserQuestion / Codex-command / (dormant) enum-options /
  binary. The genuine runtime-schema case fires only for **third-party MCP servers**
  (omnigent forwards their `requestedSchema`).

So the build is a **bounded flat-primitive schema→inputs mapper** over
`gpui-component` 0.5.1 inputs (`Input/NumberInput/Switch/Select/Radio/Checkbox`) +
a handful of structured-payload cards — **not** a hand-rolled arbitrary renderer.
(`gpui-form` 0.5.1 is compile-time struct-derive — wrong shape for a runtime
schema; it only confirms the input primitives exist.) The spike proved the load-
bearing unknown — a **runtime**, heterogeneous, runtime-sized collection of
`gpui-component` input Entities builds from a parsed schema, reads back into a
valid flat `ElicitationResult.content`, with `required` gating, `default` prefill,
`enum`/`oneOf`, and no panic — plus that it composes with the discriminated surface
(AskUserQuestion carousel; binary/url/plan cards; nested schema → raw key/value
fallback). ⚠ Fixtures were source-derived, **not byte-verified** from a live
form-mode capture (both live captures were url-mode) — byte-verify one real
`requestedSchema` at consumer-build time.

**Fallback ladder (unchanged, now the exception not the rule):** a schema with a
non-flat-primitive property degrades to a **raw key/value editor**; url-mode routes
to the approval page (permissions §3).

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

### 4.4 Editable code surface — the File tab (DECIDED 2026-07-14, scoped not spiked)

The File-tab editor (shell §8.5, the "raw file tab"; capability map "file tree +
editor") was never specified beyond a stub. There is no Monaco/CodeMirror/Zed
equivalent shipped by gpui, which raised the question of whether Lens must adopt
Zed's editor stack. **Decision: target the top of a "comfortable editor" tier,
build it in-repo by vendor-and-patch, and do not build language-server
machinery.** Rationale below; this subsection is the SSOT.

**Capability bands (where the cost cliff actually is):**

| Band | Capabilities | Computed from | Needs the server? |
|---|---|---|---|
| **2a — editing core** | rope buffer, cursors, multi-cursor, selection, undo/redo, viewport virtualization, IME/keyboard/mouse | file bytes | No |
| **2b — comfortable** | tree-sitter highlight, bracket match, auto-indent/close, find/replace, line numbers, folding, soft-wrap | file bytes | No |
| **3 — IDE intelligence** | LSP completions, diagnostics, hover, go-to-def, find-refs, rename/refactor, signature help | a **language server** | **Yes** |

**Target = top of band 2b.** Not a compromise — a forced boundary. **Lens is a
pure REST/SSE/WS client (AGENTS.md); it never touches the filesystem directly.**
File contents arrive over the env-scoped `filesystem/{relative_path}` endpoints
(workspace §3) and the worktree lives on the omnigent *host* — often a remote
managed sandbox. Band-3 language intelligence therefore needs a language server
running **where the files are (the host)** plus an **LSP-proxy protocol over the
wire**, and **omnigent exposes no such endpoint**. So band 3 is blocked by the
contract, **not by editor-widget effort** — no matter how capable the widget, you
cannot light up completions/diagnostics/go-to-def against a worktree the server
won't proxy an LSP for. The 2b/3 line coincides exactly with the
local-computation / needs-a-server line, which is why top-of-2b is the *correct*
target rather than a partial one. The user's real IDE, one alt-tab away on the
same worktree, is the band-3 escape hatch.

**Build approach — vendor-and-patch, in-repo, no separate library.**
`gpui-component` 0.5.1 ships an **editable code input** (tree-sitter highlight,
line numbers) under Apache-2.0 — the *same vendor-and-patch play already
validated for its markdown module* (§4.1). Plan: vendor that input, spike how far
it reaches against the band-2b list, and build custom internals **only for the
specific gaps** — not a from-scratch buffer/layout/shaping engine. Do **not**
spin this out as a general-purpose "Monaco-in-Rust" library: that is
speculative-generality scope (API design, docs, versioning) that serves no Lens
goal; extract a crate only if a concrete reuse case ever earns it. Zed's
`crates/editor` is an **architecture reference only** (rope + display-map +
multi-buffer) — **GPL-3.0, ~40k LOC coupled to `language`/`project`/`multi_buffer`
/LSP**, i.e. effectively forking Zed; ruled out, consistent with §3 (crates.io
default, forking is a one-way door) and §4.1's "Zed crates = reference-only (GPL)."

**Write path** is not a widget concern: edits persist via workspace §3 verbs —
`PUT {content}` (full-file write) / `PATCH {old_text,new_text}` (search-replace).

**Parked dependency (band 3, if ever wanted):** an omnigent-side **LSP-proxy
contract** (a contract request, sibling to the deferred `client-message-id` ask)
— or a deliberate decision to break the pure-client boundary and run local
language servers against *local* worktrees only. Both are separate, larger calls;
recorded in `SPEC-GAPS.md`, not scheduled. Neither is an editor-widget problem.

---

## 5. Framework-specific seams the other specs reference

Each spec has a "framework divergence" section. What each one owns vs. here:

| Spec | Owns there | Resolved here |
|---|---|---|
| typed client | (no framework impact) | Blocking reader threads + `std::sync::mpsc`; no tokio requirement |
| state model §14 | State primitive (gpui `Entity::observe` vs alternative store); the channel→UI crossing (`cx.spawn` + entity update) | gpui's per-entity notify + `cx.spawn` is the foreground replica implementation |
| application shell §17 | The board (ordinal-slot responsive reflow vs a free-form canvas) | §2.4 of this doc — confirmed ordinal board is *simpler*, not harder, in gpui |
| transcript §19 | Progressive re-render (stable-identity in-place diff); markdown library; virtualization | §4.1 markdown spike; **virtualization SPIKED 2026-07-08 → native gpui `list()`** (variable-height, `ListAlignment::Bottom`) satisfies all four §16 contracts — `uniform_list` was the wrong primitive, `list()` is the right one (§4.1c/d verdict) |
| workspace §9 | Terminal widget | §2.2 — `alacritty_terminal` + `portable-pty`, Arbor template (MIT) |
| workspace §4 | Diff widget | §2.3 — `imara-diff` + `syntect` cached, Arbor template (MIT) |
| workspace §3 / shell §8.5 | File-tab editing (data + container) | §4.4 — editable code surface, **top of band 2b** (highlight/find-replace/multi-cursor/fold; no LSP — blocked by omnigent's no-LSP-proxy contract); vendor-and-patch `gpui-component` code input, in-repo |
| permissions | (form renderer uses gpui-component inputs; a bounded flat-primitive schema→inputs mapper + structured-payload cards) | §4.3 form spike **SPIKED 2026-07-08 → GO** (6/6) — runtime schema→`gpui-component` inputs → valid flat `ElicitationResult.content`; discriminated surface, not an arbitrary renderer |
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
