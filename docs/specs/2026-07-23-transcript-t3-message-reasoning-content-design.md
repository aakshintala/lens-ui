# Design — Transcript T-3: Message & reasoning content

**Date:** 2026-07-23
**Status:** Design approved; revised after cross-family (grok-4.5) review
2026-07-23 — ready for implementation plan. Review findings folded in; findings
file `scratchpad/t3-grok-review.md`.
**Owner:** Lens transcript workstream
**Branch:** `t3-message-reasoning-content`
**Spec sections realized:** conversation-transcript `§5` (streaming), `§6`
(content rendering — markdown vs verbatim), `§7` (reasoning).

Master spec: [`docs/design/conversation-transcript.md`](../design/conversation-transcript.md).
Prior art: [`docs/spikes/2026-07-07-markdown-streaming.md`](../spikes/2026-07-07-markdown-streaming.md)
(verdict PARTIAL — vendor the markdown module + patch), framework
[`§2.5`](../design/framework.md) (security boundary), [`§4.1`](../design/framework.md)
(gpui markdown lock).

> **Line-anchor caveat:** all `file:line` references are gpui-component `0.5.1`
> anchors, verified 2026-07-23. Reconfirm at vendor time.

---

## 1. Scope & boundaries

**In:** Rendering *content* for the transcript's message and reasoning rows —
replacing the T-2 stubs (`focused/view.rs::render_stub_row`, which paints every
row as a plain `div` + kind tag + flat `text`) with real per-channel rendering.

Concretely: assistant markdown (`§6.1`), user verbatim + backtick-gating
(`§6.2`), the link/image security boundary (`§6.3`), file-path autolink (`§6.1`),
reasoning lifecycle (`§7`), and streaming/progressive render with stable identity
and safe-prefix (`§5`).

**Out (owned by sibling tasks/docs):**

- Tool spans, native tools, resource markers — **T-4** (`§8`/`§9`/`§12`).
- Work-section collapse lifecycle, `WorkSectionMeta`, compaction, todos,
  agent-changed — **T-6** (`§4`/`§10`/`§11`/`§13`/`§14`). T-3 leaves those rows
  (`SectionChip`, `SectionRail`, `WorkChild`, `ResourceEvent`, `ReconnectBreak`,
  `LoadOlder`) as their current stubs.
- The `navigate_to_file` **click handler** (resolve path → open editor → scroll)
  — the workspace document. T-3 only *detects, paints, and emits* the event.
- Inline-image **artifact fetch** — the authenticated artifact API does not exist
  yet. T-3 renders artifact refs as link-placeholders and external images as
  links; wiring the fetch is deferred to when the API lands.
- **Outer-list scroll anchoring on row-height growth** — owned by the T-2
  `FocusedTranscript` list contracts (`§16`); T-3 must not regress them but does
  not re-implement them (see §3.7, finding-4 resolution).

---

## 2. Decisions ledger

| # | Decision | Choice | Rationale |
|---|----------|--------|-----------|
| D1 | Safe-prefix streaming | **IN**, via `mdstitch` | `§5` requires it; user decision 2026-07-23. `mdstitch` **closes** unterminated syntax token-by-token (`**wor` → `**wor**`) so the renderer always sees valid markdown. NB: this is *close-speculatively*, not master §5's literal *hold-as-plain* — see §3.7. |
| D2 | Toolchain | **bump `rust-toolchain.toml` 1.91.1 → 1.95.0** | `mdstitch` MSRV = exactly 1.95.0. Verified: workspace `cargo check` clean on 1.95 (0 err/0 warn); `mdstitch` builds clean, zero transitive deps, Apache-2.0; gate clippy needs **2 trivial fixes** (`rowsource.rs:338` collapsible_match, `reduce/mod.rs:507` useless_conversion). |
| D3 | Branch structure | **one branch, internal tasks** (like T-0/T-1/T-2) | User decision 2026-07-23. |
| D4 | Markdown library | **vendor gpui-component's `src/text/` module** into `lens-ui/src/md/`, keep `gpui-component` as a dep | Spike verdict PARTIAL. `text/`'s external coupling is all *public* gpui-component API (bucket A) + one `pub(crate)` helper that is really part of the module (bucket B). |
| D5 | Sanitization site | **paint-time in vendored `node.rs`/`inline.rs`** (not the spike's pre-parse transform) | Exact (no `to-cmark` round-trip loss); catches links/images regardless of source (markdown syntax *or* embedded HTML); the pre-parse transform misses GFM/embedded-HTML links/images. |
| D6 | Raw HTML in markdown | **escape to source** — `Node::Html` (block) → `html`-tagged `CodeBlock`; inline HTML → inline-code — mirroring the existing `Node::Math` arm | `§6.1` excludes raw HTML. Block HTML today renders **live** `<a>`/`<img>` (security vector — §7); escaping to source closes it and drops ~1600 lines of HTML renderer. |
| D7 | Link handling | route the module's `cx.open_url` sites through `validate_link_url`; failed validation **strips the link mark at paint** (inert text, no cursor), not merely a no-op click | `§6.3` / framework `§2.5`. Master wants dangerous-scheme links *never clickable*, not clickable-then-blocked. |
| D8 | File-path click | separate destination — emit `navigate_to_file(path, line?)`, not `open_url` | `§6.1`. Paths open the internal editor, not the browser. |
| **D9** | **GFM image safety** | **paint-time image gate (P5): `validate_image_ref` before every `img()`; non-artifact → link/placeholder text, never an `img`** | `§6.1`/`§6.3`. `![](http://tracker)` paints a live fetch (`node.rs:609`) with no HTML involved — the primary image threat; D6/D7 don't cover it (grok finding #1). |
| **D10** | **Transcript embedding mode** | **`MarkdownView` rows use `scrollable(false)` (fit-content)** | Nested virtualized scroll is hostile to the outer `list()`/§16 bottom-anchor. Consequence: the inner-`list_state.reset` path (P3) is **dead** for message rows (grok finding #4) — height-growth anchoring is the outer list's job. |
| **D11** | **Streaming widget identity** | **one stable content key for the whole stream→finalize lifetime** — mint at stream start, carry through finalize; `MarkdownView` ElementId = that key. **Forbid** retargeting ElementId `acc_id → item_id` | `use_keyed_state("{id}/state")` (`text_view.rs:412`) ⇒ ElementId change = new `TextViewState` = remount = the flash §5 exists to prevent (grok finding #2). Row `Entity` stability ≠ ElementId stability. |

---

## 3. Architecture — components

Each is a bounded, independently-testable module.

### 3.1 `lens-ui/src/md/` — the vendored markdown widget *(deep module)*

Vendored copy of gpui-component `0.5.1` `src/text/` — `text_view.rs` (909L),
`node.rs` (1253L), `inline.rs` (544L), `style.rs`, `utils.rs`,
`format/markdown.rs` (395L), `format/mod.rs` — **plus** a co-vendored ~30-line
`global_state` (bucket B). The `format/html.rs` (711L) + `format/html5minify/`
(899L) HTML renderer is **dropped** (D6).

**Public surface:** a thin `MarkdownView` wrapper around the vendored
`TextView::markdown(id, text, window, cx)`, keyed by the D11 stable content key.
All ~3.3k vendored lines stay behind that surface.

**Lint boundary:** the vendored code is third-party — cap its lints at the `md/`
module root (`#![allow(clippy::all, warnings)]` or equivalent), exactly as the
vendored VT crates are handled ([[terminal-vt-vendored-executed]], cap-lints).
**Our** code stays strict: `MarkdownView`, `security.rs`, and the glue *around*
each patch live under `lints.workspace`. Keep patches surgical so the
strict/capped boundary stays legible (a reviewer must see what *we* changed).

**Coupling — bucket A (external, all `pub`, rewrite `crate::` →
`gpui_component::`):** `theme::ActiveTheme`, `styled::{StyledExt, v_flex,
h_flex}`, `highlighter::{HighlightTheme, SyntaxHighlighter}`, `input::{Selection,
Copy, …}`, `scroll::ScrollableElement`, `icon::{Icon, IconName}`,
`tooltip::Tooltip`. These are the same primitives lens-ui already consumes from
gpui-component. (Verified: no proc-macro coupling inside `text/`.)

**Coupling — bucket B (co-vendor, ~30L):** `global_state::GlobalState` is
`pub(crate)`; its `text_view_state_stack` holds `TextViewState` (a `text/` type),
so it exists to serve `text/`. **Landmine:** the co-vendored `GlobalState` is a
*different* `Global` type than the dep's, so it needs its own `cx.set_global` via
a new **`md::init`** — without it, `GlobalState::global_mut` (`text_view.rs:702`)
panics at paint. (`Node::Root` in `text/` is the module's own enum variant, not
`crate::Root` — no dependency.)

**Direct external crates the vendored code uses — add to `lens-ui`
(grok finding #6):** `ropey` (`node.rs:14`), `smol` (channels + `StreamExt`,
`text_view.rs:12–14`). `Timer` is gpui (already present). `markdown` (mdast — the
module calls `markdown::to_mdast`, `markdown.rs:2`). `pulldown-cmark` **only if**
the §3.6 autolink scanner needs it. Dropping `html.rs`/`html5minify` means
`html5ever`/`markup5ever` are **not** pulled in.

**`md::init` must:** (1) `set_global` the co-vendored `GlobalState`; (2) bind the
Copy keybinding (`text_view.rs:31–37`, needed for selectable markdown). Called
once at app startup alongside `gpui_component::init` (which still runs for theme +
bucket-A globals — `cx.theme().highlight_theme` at `text_view.rs:410` is a runtime
prerequisite).

**Html-API excision (the "unlisted 5th edit" — grok finding #6):** dropping
`html.rs` requires deleting the references to it: `TextViewType::Html`
(`text_view.rs:114`), `TextView::html` (`:438`), the `parse_content` Html arm,
and `format/mod.rs`'s `mod html`/`mod html5minify`. Otherwise the vendor does not
compile.

**The patches** (the reason for vendoring):

| # | File / site | Patch | Driver | Verified concern |
|---|------|-------|--------|---|
| P1 | `text_view.rs` `UpdateFuture` reset (`~178`), delay (`~628`) | trailing-debounce-that-resets → **interval throttle** (§3.7 algorithm) | `§5` progressive render (today: nothing renders until the stream *pauses*) | |
| P2 | `text_view.rs` reparse-clear (`~610`) **+ `update_bounds` (`253–256`)** | drop `clear_selection()` on reparse **and** carve out the size-change clear while streaming/selecting | `§5` selection survives a streamed (height-growing) update | grok #3: `update_bounds` clears on any size delta; P2-at-`:610`-only is insufficient |
| P3 | `node.rs` `render_root` `list_state.reset` (`~1123`) | scroll-preserving splice/retained anchor | `§5` no scroll-jump | grok #4: **dead for `scrollable(false)`** — applies **only** to the §3.4 reasoning capped region if it uses `scrollable(true)`. Conditional, moves to T3-4. |
| P4 | `format/markdown.rs` (`159`, `297`) | `Node::Html` → escaped **source** (block → `html` `CodeBlock`, inline → inline-code); never `super::html::parse` | `§6.3` security (D6) | block only; inline HTML already mostly no-ops |
| **P5** | `node.rs` `img(...)` (`609`) | **`validate_image_ref` before `img()`; non-artifact → link/placeholder, never `img`** | `§6.1`/`§6.3` (D9) | grok #1: GFM `![]()` live-fetch |
| **P6** | `inline.rs:359`, `node.rs:620` (`open_url`) + link-mark paint | route through `validate_link_url`; **strip link mark** on failure | `§6.3` (D7) | grok #5: paint, not just click |

### 3.2 `lens-ui/src/security.rs` — the `§2.5` boundary *(shared)*

`validate_link_url` and `validate_image_ref`, **reimplemented** from framework
`§2.5` (Paneflow `markdown/security.rs` is GPL — ideas only, not copied).

- `validate_link_url`: allow only `http(s)`; block `file:`/`javascript:`/`data:`/
  `vbscript:`/bare-host/>8KiB and the mdstitch `stitch:incomplete-link` sentinel
  → inert.
- `validate_image_ref`: path-traversal + symlink-escape + scheme-injection +
  remote-beacon guards; artifact refs may inline, external/`data:` images render
  as links, never fetched.

Lives **outside** `md/` because `§6.3` reuses the *same* boundary for T-7
elicitation `params.url` (`validate_elicitation_url`) **and** for the §3.5
user-channel autolinks. Called paint-time from `md/{node,inline}.rs` (P5/P6, D5)
and from the user renderer.

### 3.3 Content channel router *(pure, in `focused/`)*

Maps a content row → render mode (`§6`, per-channel not global):

| Channel | Mode |
|---|---|
| Assistant message text, reasoning | **markdown** (`MarkdownView`) |
| User message text | **verbatim + backtick-gated** (§3.5) |
| Tool output, args, errors | **verbatim** — *T-4, not here* |

### 3.4 Reasoning region *(§7)*

Renders `Reasoning` items (`full_text`, `summary_text`, `encrypted`):

- **Live:** stream into a small auto-scrolling **capped** region, auto-expanded,
  labeled `💭 thinking…`. Body is markdown (prose) → `MarkdownView`. **Capped-region
  mechanics (grok #9):** fixed max-height container; auto-scroll to bottom while
  active. This is the one place that may use the inner `TextView::scrollable(true)`
  — so **P3 is verified here**, not on message rows.
- **On close:** collapse to `💭 thought for Ns`.
- **Expanded:** `summary_text` + `show full reasoning ↗` reveals `full_text`
  (full-only harnesses show full).
- **Encrypted** (`encrypted: true`): `🔒 thought for Ns · reasoning hidden` — no
  expand, duration still shown.
- **Duration source (grok #8/#9 gap):** `ItemKind::Reasoning` in `lens-core`
  carries **no duration field** — "Ns" must be derived from the
  `ReasoningStarted`→synthetic-`ReasoningClosed` timestamps. **First plan
  milestone must locate/plumb that source** (candidate: the reducer's per-item
  timestamps); if absent, either add it or scope "Ns" out for v1 and label
  `💭 thought` without the duration.

### 3.5 User-message renderer *(§6.2)* — a distinct segment pipeline, not a wrapper

`§6.2` is a full verbatim lexer, not a thin call into `MarkdownView`
(grok #10). Pipeline (pure): **split into segments → render per segment**:

- **Prose segments** (outside backticks): literal — implicit inline markdown
  (`*em*`, `#`, `-`/`1.`, `>`, tables) **not** honored; whitespace preserved.
  Path/URL **autolink runs on prose segments only**, through `security.rs`
  (same boundary as assistant).
- `` `inline code` `` → inline-code chip (reuse vendored inline-code style).
- ` ```lang ` → syntax-highlighted by language (reuse vendored `CodeBlock` +
  `SyntaxHighlighter`).
- ` ```markdown `/` ```md ` → nested `MarkdownView` (explicit opt-in) — **same
  security boundary applies** (P4/P5/P6).
- ` ``` ` untagged → plain monospace.

Autolink detection runs **after** the fence split (never inside code/fence
segments). Tests: one per fence form **+** an autolink-in-prose vs
autolink-suppressed-in-code test.

### 3.6 File-path autolink *(§6.1)*

The `markdown` crate (GFM) autolinks bare **URLs** for free; **paths are not GFM
autolinks**, so T-3 adds detection: scan non-code text spans for path-shaped
tokens (`src/parser.rs`, optional `:line`), paint clickable, emit
`navigate_to_file(path, line?)` — distinct from `open_url`. Handler stubbed
(§1 out). Scanner reuse-vs-standalone is open item #4.

### 3.7 Streaming / safe-prefix / identity *(§5)*

- **Identity (D11):** `MarkdownView` ElementId = a content key minted at stream
  start and stored on the row (§5 plumbing), preserved verbatim through T-2's
  `commit_stream_finalize` (`rowsource.rs:402–424` remaps `RowId::StreamTail(acc_id)`
  → `Work(item_id)` while keeping the row `Entity`). The ElementId **must not**
  change at finalize. `MessageAcc.message_id` is often `None` mid-stream, so the
  key cannot be the item id — it is a stream-scoped id. Finalize test: same
  `TextView` keyed-state EntityId before/after; no forced remount.
- **Throttle algorithm (P1, grok #7):** while streaming, parse the latest
  accumulated text at most every **N ms** (start N≈50–200, tune in verification);
  **interval/leading-edge — never reset the timer per token**. Pipeline order per
  frame: *accumulate deltas → coalesce to a ~60fps frame tick → `mdstitch`(text)
  → `TextView` update → internal throttle*. The **same stitched pipeline runs on
  the finalize string** so the last streamed parse input equals the final parse
  input (else finalize reparses and can jump — grok #2).
- **mdstitch semantics vs master wording (D1, grok #7):** master §5 says "hold
  the open trailing construct as plain"; mdstitch instead **closes** it
  speculatively (`**wor` → `**wor**`), so the open tail renders *formatted
  optimistically* and may briefly re-format if the construct resolves differently.
  Accepted under D1; the visual is "formats the tail optimistically," bounded to
  the trailing construct — **not** literally master's hold-plain. Documented so
  verification judges the right behavior.
- **Coalesce** deltas to a ~60fps frame tick — never re-render per token.
- Finalize `StreamingMessage` → `Message` is a visual no-op **given D11 + the
  same-pipeline finalize** (not "by construction" — it requires both).

---

## 4. Link/image handling matrix (the §6.3 threat model)

Every content path × lifecycle stage, and where the boundary applies
(grok #1/#5 — replaces the old prose "security findings"):

| Source | Paint | Click | Fetch | Guard |
|---|---|---|---|---|
| GFM link `[t](url)` | link mark; **stripped if `validate_link_url` fails** (P6) | `open_url` only if http(s) valid; file-path → `navigate_to_file` (D8) | — | `validate_link_url` |
| GFM image `![](url)` | **`img()` only if `validate_image_ref` passes** (P5); else link/placeholder | (as link) | **never** for non-artifact | `validate_image_ref` |
| Block HTML | escaped **source** code block (P4) — no live tree | — | — | P4 (html.rs unreachable) |
| Inline HTML | inline-code / literal (P4) | — | — | P4 |
| User autolink (§3.5) | link mark, same strip rule | same as GFM link | — | `validate_link_url` (same `security.rs`) |
| mdstitch `stitch:incomplete-link` | inert | never | — | `validate_link_url` rejects the scheme |

**Verified vectors closed:** (a) block HTML renders live `LinkMark`/`ImageNode`
today (`html.rs:306,328,430`) — closed by P4 + paint-time boundary backstop;
(b) `open_url` fires on *any* URL today (`inline.rs:359`, `node.rs:620`) — closed
by P6; (c) GFM `![](http…)`/`data:` paints a live fetch today (`node.rs:609`,
`markdown.rs:140`) — closed by P5.

---

## 5. Row content plumbing *(decided — first plan milestone, grok #8)*

`RowPresentation` (`focused/rowsource.rs:53`) carries a flat `text: String` —
insufficient. This is a **T3-1 blocker**, not a later nit: T3-1 cannot wire
content without knowing where the markdown source and the D11 content key live,
and T3-4 needs reasoning fields.

**First plan milestone decides & implements:** extend `RowPresentation` with a
typed content payload (assistant markdown source + stream content key; user raw
text; reasoning `summary`/`full`/`encrypted`/`duration`) **vs.** a read-through to
the backing item — and locates the §3.4 reasoning-duration source. Recommended:
typed payload on `RowPresentation` (keeps the row renderer pure and the content
key co-located with finalize).

---

## 6. Task sequence (one branch, subagent-driven like prior slices)

| Task | Scope | Gate emphasis |
|---|---|---|
| **T3-0 Infra + vendor** | toolchain → 1.95 + 2 clippy fixes (D2); add `mdstitch`, `markdown`, `ropey`, `smol`; vendor `text/` → `src/md/` (re-point bucket A, co-vendor bucket B, **write `md::init`**, **excise Html API**, drop HTML renderer, cap lints); apply structural patches P1–P4; smoke-render one markdown doc | **riskiest — lands first.** De-risk *before* the plan with a throwaway minimal-vendor `cargo check -p lens-ui` (grok #6). `xtask gate` green incl. clippy `-D warnings` (vendored capped, glue strict) |
| **T3-M row plumbing** | §5: typed `RowPresentation` payload + content key + reasoning-duration source (first milestone; unblocks T3-1/T3-4) | unit tests on the projection |
| **T3-1 Assistant markdown** | wire Message/StreamingMessage → `MarkdownView` keyed by D11 content key; syntax highlight; streaming (throttle P1 + coalesce + `mdstitch`); **finalize identity test** (no remount) | headless identity test + real-window streaming probe |
| **T3-2 Boundary + autolink** | build `security.rs`; **P5 image gate + P6 link-strip** paint-time; §6.1 file-path autolink detect+paint+**emit** `navigate_to_file` (stubbed) | adversarial fixture: `javascript:`/`data:` links, `![](http…)`/`![](data:…)` images, embedded-HTML `<a>`/`<img>`, path autolink, stitched incomplete link |
| **T3-3 User messages** | §3.5 segment pipeline: verbatim + backtick-gating + prose-only autolink through `security.rs` | per-fence-form + autolink-in-prose-vs-code tests |
| **T3-4 Reasoning** | §7 live capped (**P3 here** if `scrollable(true)`) → collapse → summary/full → encrypted; duration from §3.4 source | lifecycle render tests (4 states) |

Each task: ≥1 cross-family review (author = composer-2.5; reviewer = grok-4.5 or
Opus-in-CC while codex quota is out). End-of-branch: one consolidated whole-branch
review ([[whole-branch-review-needs-a-builder]], [[review-spend-policy]]).

---

## 7. Master §5–§7 → design traceability *(grok #9)*

| Master requirement | Where realized |
|---|---|
| §5 progressive safe-prefix | P1 throttle + `mdstitch` (§3.7); D1 semantics note |
| §5 stable identity, finalize no-op | D11 + same-pipeline finalize (§3.7); T3-1 identity test |
| §5 coalesce to frame tick | §3.7 |
| §5 selection survives streamed update | P2 (drop reparse-clear + `update_bounds` carve-out) |
| §5 no scroll-jump | D10 (`scrollable(false)`) → outer T-2/§16 list owns anchoring; P3 only for §3.4 |
| §6.1 GFM core + task lists + tables + fences + syntax highlight | vendored `MarkdownView` (`SyntaxHighlighter` needs theme init, §3.1) |
| §6.1 file-path autolink → `navigate_to_file` | §3.6 (emit only; handler out) |
| §6.1 artifact-only inline images | P5 gate now; fetch deferred (§8) |
| §6.1 no math / no raw HTML | math literal (§8); HTML → source (P4/D6) |
| §6.2 user verbatim + backtick-gating | §3.5 segment pipeline |
| §6.3 uniform link/image boundary | `security.rs` (P5/P6), reused by user autolink + `markdown` fence + T-7 elicitation |
| §7 live capped / collapse / summary→full / encrypted | §3.4 |
| §7 duration "Ns" | §3.4 duration source (first-milestone plumb) |

---

## 8. Deferrals & seams

- **Inline-image artifact fetch (§6.1):** artifact API absent → artifact refs =
  link-placeholders, external images = links (never fetched, P5). Wire the fetch
  when the API lands.
- **`navigate_to_file` handler:** T-3 emits only; resolve/open-editor is the
  workspace doc. Seam **PROVISIONAL** ([[premature-layer-boundary-binding]]).
- **Math/LaTeX (§6.1):** render literally, out of scope.
- **HTML highlighter grammar:** if the vendored highlighter lacks an `html`
  grammar, the escaped-source block is plain monospace (acceptable).
- **`mdstitch` API:** confirm exact interface at integration.

---

## 9. Verification

- **Gate:** `xtask gate` green on 1.95 incl. clippy `-D warnings`; `md/` lives in
  already-gated `lens-ui` (vendored body lint-capped, glue strict;
  [[per-task-gate-must-run-clippy]], [[xtask-gate-scope]]).
- **T3-0 pre-plan de-risk:** throwaway minimal-vendor `cargo check -p lens-ui`
  proving re-point + `ropey`/`smol` + `md::init` + Html-excision compile, before
  the implementation plan is finalized.
- **Streaming identity:** headless test — appended deltas + finalize swap do not
  change the `TextView` keyed-state EntityId; real-window streaming probe (held
  scroll + held selection survive the swap; per-frame build ~O(changed blocks)).
  Heed harness traps ([[terminal-realwindow-harness-pitfalls]],
  [[gpui-list-scroll-and-realwindow-probe-gotchas]], [[gpui-test-noop-text-system]]).
- **Selection:** probe asserts a held selection survives a **height-growing**
  streamed update (exercises the P2 `update_bounds` carve-out, not just `:610`).
- **Security (threat matrix §4):** adversarial-fixture tests — `javascript:`/
  `data:`/`file:` links inert *at paint* (mark stripped); `![](http…)`/`![](data:…)`
  → link, not fetch; embedded-HTML `<a>`/`<img>` render as source; path autolink
  emits `navigate_to_file`; stitched incomplete link inert.
- **Backtick-gating:** unit test per §6.2 fence form + autolink prose-vs-code.
- **Reasoning:** render tests for the four §7 states; duration formatting.
- **Demo:** on-device screenshot ([[gpui-ondevice-screenshot-workflow]]) — never
  run the demo during `xtask gate`.

---

## 10. Open items for the plan

1. Reconfirm the P1–P6 line-anchors at vendor time (0.5.1 may drift).
2. Throttle interval `N` (§3.7) — start 50–200 ms, tune in verification.
3. File-path autolink scanner: reuse a parser's span info vs. a standalone
   scanner over rendered text spans.
4. Reasoning-duration source (§3.4) — confirm the reducer timestamp path or scope
   "Ns" out for v1.
5. `security.rs` crate location if T-7 elicitations land outside `lens-ui`
   (currently `lens-ui` for reuse).
