# Design — Transcript T-3: Message & reasoning content

**Date:** 2026-07-23
**Status:** Design approved; ready for implementation plan
**Owner:** Lens transcript workstream
**Branch:** `t3-message-reasoning-content`
**Spec sections realized:** conversation-transcript `§5` (streaming), `§6`
(content rendering — markdown vs verbatim), `§7` (reasoning).

Master spec: [`docs/design/conversation-transcript.md`](../design/conversation-transcript.md).
Prior art: [`docs/spikes/2026-07-07-markdown-streaming.md`](../spikes/2026-07-07-markdown-streaming.md)
(verdict PARTIAL — vendor the markdown module + patch), framework
[`§2.5`](../design/framework.md) (security boundary), [`§4.1`](../design/framework.md)
(gpui markdown lock).

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

---

## 2. Decisions ledger

| # | Decision | Choice | Rationale |
|---|----------|--------|-----------|
| D1 | Safe-prefix streaming | **IN**, via `mdstitch` | `§5` requires it; user decision 2026-07-23. `mdstitch` closes unterminated syntax token-by-token so the renderer always sees valid markdown. |
| D2 | Toolchain | **bump `rust-toolchain.toml` 1.91.1 → 1.95.0** | `mdstitch` MSRV = exactly 1.95.0. Verified: whole workspace `cargo check` clean on 1.95 (0 err/0 warn); `mdstitch` builds clean, zero transitive deps, Apache-2.0; gate clippy needs **2 trivial fixes** (`rowsource.rs:338` collapsible_match, `reduce/mod.rs:507` useless_conversion). |
| D3 | Branch structure | **one branch, internal tasks** (like T-0/T-1/T-2) | User decision 2026-07-23. |
| D4 | Markdown library | **vendor gpui-component's `src/text/` module** into `lens-ui/src/md/`, keep `gpui-component` as a dep | Spike verdict PARTIAL. `text/`'s external coupling is all *public* gpui-component API (bucket A) + one `pub(crate)` helper that is really part of the module (bucket B). |
| D5 | Sanitization site | **paint-time in vendored `node.rs`** (not the spike's pre-parse transform) | Exact (no `to-cmark` round-trip loss); catches links/images regardless of source (markdown syntax *or* embedded HTML); the pre-parse transform misses embedded-HTML links/images. |
| D6 | Raw HTML in markdown | **escape to source** — `Node::Html` (block) → `html`-tagged `CodeBlock`; inline HTML → inline-code — mirroring the existing `Node::Math` arm | `§6.1` excludes raw HTML. Block HTML today renders **live** `<a>`/`<img>` elements (security vector — see `§8`); escaping to source closes it and drops ~1600 lines of HTML renderer. |
| D7 | Web-link click | route the module's `cx.open_url` sites through `validate_link_url`; non-`http(s)` → inert | `§6.3` / framework `§2.5`. The module currently opens *any* URL unvalidated. |
| D8 | File-path click | separate destination — emit `navigate_to_file(path, line?)`, not `open_url` | `§6.1`. Paths open the internal editor, not the browser. |

---

## 3. Architecture — components

Each is a bounded, independently-testable module.

### 3.1 `lens-ui/src/md/` — the vendored markdown widget *(deep module)*

Vendored copy of gpui-component `0.5.1` `src/text/` — `text_view.rs` (909L),
`node.rs` (1253L), `inline.rs` (544L), `style.rs`, `utils.rs`,
`format/markdown.rs` (395L), `format/mod.rs` — **plus** a co-vendored ~30-line
`global_state` (bucket B, below). The `format/html.rs` (711L) +
`format/html5minify/` (899L) HTML renderer is **dropped** (D6).

**Public surface:** a thin `MarkdownView` wrapper around the vendored
`TextView::markdown(id, text, window, cx)`, keyed by a stable `ElementId`. All the
~3.3k vendored lines stay behind that surface.

**Lint boundary:** the vendored code is third-party — cap its lints at the `md/`
module root (module-level `#![allow(clippy::all, warnings)]` or equivalent) so the
gate's `-D warnings` + clippy do not drown in vendored-code findings, exactly as
the vendored VT crates are handled ([[terminal-vt-vendored-executed]], cap-lints).
**Our** code stays strict: the `MarkdownView` wrapper, `security.rs`, and the glue
*around* each patch live under `lints.workspace`; only the untouched vendored body
is capped. Keep the four patches surgical so the strict/capped boundary stays
legible (a reviewer must be able to see what *we* changed).

**Coupling re-point (bucket A — external, all `pub`, rewrite `crate::` →
`gpui_component::`):**

- `theme::ActiveTheme`, `styled::{StyledExt, v_flex, h_flex}`
- `highlighter::{HighlightTheme, SyntaxHighlighter}`
- `input::{Selection, …}`, `scroll::ScrollableElement`
- `icon::{Icon, IconName}`, `tooltip::Tooltip`

These are the same primitives lens-ui already consumes from gpui-component, so
sharing them is correct, not a compromise.

**Co-vendor (bucket B):** `global_state::GlobalState` is `pub(crate)` in
gpui-component, so it is not reachable externally — but its entire job is a
`text_view_state_stack` of `TextViewState` (a `text/` type). It exists to serve
`text/`; vendor its ~30 lines into `md/`. (The `Root` references in `text/` are
the module's own `Node::Root` enum variant, **not** `crate::Root` — no
dependency.)

**The four patches** (the whole reason for vendoring):

| # | File | Patch | Driver |
|---|------|-------|--------|
| P1 | `text_view.rs` (`~628` delay, `~168` reset) | trailing-debounce-that-resets → **throttle** (render mid-stream at a bounded cadence) | `§5` progressive render (today: nothing renders until the stream *pauses*) |
| P2 | `text_view.rs` (`~610`) | **drop `clear_selection()`** on reparse | `§5` selection survives a streamed update |
| P3 | `node.rs` (`~1123`) | `list_state.reset(len)` → **scroll-preserving** splice/retained anchor | `§5` no scroll-jump-to-top on each render |
| P4 | `format/markdown.rs` (`159`, `297`) | `Node::Html` → escaped **source** (block → `html` `CodeBlock`, inline → inline-code); **never** `super::html::parse` | `§6.3` security (D6) |

Line numbers are 0.5.1 anchors — reconfirm at vendor time.

### 3.2 `lens-ui/src/security.rs` — the `§2.5` boundary *(shared)*

`validate_link_url` and `validate_image_ref`, **reimplemented** from framework
`§2.5` (Paneflow `markdown/security.rs` is GPL — ideas only, not copied).

- `validate_link_url`: allow only `http(s)`; block `file:`/`javascript:`/`data:`/
  `vbscript:`/bare-host/>8KiB → inert.
- `validate_image_ref`: path-traversal + symlink-escape + scheme-injection +
  remote-beacon guards; artifact refs may inline, external images render as
  links.

Lives **outside** `md/` because `§6.3` reuses the *same* boundary for T-7
elicitation `params.url` (`validate_elicitation_url`). Called paint-time from
`md/node.rs` (D5) and later from the elicitation dock.

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
  labeled `💭 thinking…`. Reasoning body is markdown (prose) → routes through
  `MarkdownView`.
- **On close:** collapse to `💭 thought for Ns`.
- **Expanded:** `summary_text` + `show full reasoning ↗` reveals `full_text`
  (full-only harnesses show full).
- **Encrypted** (`encrypted: true`): `🔒 thought for Ns · reasoning hidden` — no
  expand, duration still shown.

### 3.5 User-message renderer *(§6.2)*

Verbatim, backtick-gated (deliberately asymmetric from assistant markdown):

- Outside backticks: literal — implicit inline markdown (`*em*`, `#`, `-`/`1.`,
  `>`, tables) **not** honored; whitespace preserved. **Paths/URLs autolinked**
  (on prose spans only).
- `` `inline code` `` → inline-code chip.
- ` ```lang ` → syntax-highlighted by language.
- ` ```markdown `/` ```md ` → rendered as formatted markdown (explicit opt-in).
- ` ``` ` untagged → plain monospace.

### 3.6 File-path autolink *(§6.1)*

The `markdown` crate (GFM) autolinks bare **URLs** for free; **paths are not GFM
autolinks**, so T-3 adds detection: scan non-code text spans for path-shaped
tokens (`src/parser.rs`, optional `:line`), paint clickable, and on click **emit
`navigate_to_file(path, line?)`** — distinct from `open_url`. Handler stubbed
(§1 out).

### 3.7 Streaming / safe-prefix *(§5)*

- `mdstitch` runs on the accumulated text before handoff to `MarkdownView`.
- **Coalesce deltas to a ~60fps frame tick** — never re-render per token.
- **Stable widget identity:** the `MarkdownView` entity is keyed by
  response/item id; diff in place, never unmount/remount. `StreamingMessage` →
  canonical `Message` finalize is a **visual no-op** by construction (same id,
  full text).

---

## 4. Link-click routing (post-T-3)

The vendored module dispatches clicks via `cx.open_url(&link.url)` at
`inline.rs:359` and `node.rs:620`, **unvalidated**. T-3 makes the click site a
3-way discriminator:

1. **Validated web URL** (`http(s)`, passed `validate_link_url`) → `open_url`
   (OS browser).
2. **File path** → emit `navigate_to_file(path, line?)` (internal editor).
3. **Everything else** (bad scheme, HTML-derived — those are now source text) →
   **inert**.

Same behavior in user messages, on prose spans only (URLs/paths inside
inline-code or fences stay literal).

---

## 5. Row content plumbing

`RowPresentation` (`focused/rowsource.rs:53`) currently carries a flat
`text: String`. Content rows need structured content (markdown source; reasoning
`summary`/`full`/`encrypted`/`duration`; user raw text + gating). **Decision
deferred to the plan:** extend `RowPresentation` with a typed content payload
enum vs. have the row renderer read structured content from the backing
item/entity. Either way the flat `text` string is insufficient for T-3 content
rows.

---

## 6. Task sequence (one branch, subagent-driven like prior slices)

| Task | Scope | Gate emphasis |
|---|---|---|
| **T3-0 Infra** | toolchain → 1.95 + the 2 clippy fixes (D2); add `mdstitch` + the `markdown` crate (mdast — the vendored module calls `markdown::to_mdast` directly), `pulldown-cmark` **only if** the §3.6 autolink scanner needs it (open item #4); vendor `text/` → `src/md/` (re-point bucket A, co-vendor bucket B, drop HTML renderer — so `html5ever`/`markup5ever` are *not* pulled in); apply the 4 patches; cap vendored-module lints (§3.1); smoke-render one markdown doc through the gate | riskiest — lands first; `xtask gate` green incl. clippy `-D warnings` (vendored body capped, our glue strict) |
| **T3-1 Assistant markdown** | wire `Message`/`StreamingMessage` rows → `MarkdownView` keyed by id; syntax highlight; streaming (coalesce + `mdstitch`); stable-identity + finalize-no-op tests | headless identity test + real-window streaming probe |
| **T3-2 Boundary + autolink** | build `security.rs`; paint-time sanitization in `md/node.rs`; route the two `open_url` sites through `validate_link_url`; §6.1 file-path autolink detect+paint+**emit** `navigate_to_file` (stubbed sink) | adversarial fixture: `javascript:`/`data:` links, external image, embedded-HTML `<a>`/`<img>`, path autolink |
| **T3-3 User messages** | §6.2 verbatim + backtick-gating + autolink on prose spans | gating unit tests (each fence form) |
| **T3-4 Reasoning** | §7 live capped → collapse → summary/full expand → encrypted placeholder | lifecycle render tests |

Each task: ≥1 cross-family review (author = composer-2.5). End-of-branch: one
consolidated **codex** whole-branch review (per [[whole-branch-review-needs-a-builder]],
[[review-spend-policy]]).

---

## 7. Security findings (verified against the 0.5.1 source)

1. **Block HTML embedded in markdown renders LIVE elements.**
   `markdown.rs:297` → `html.rs` (an `html5ever`-based HTML→node renderer)
   produces real `LinkMark { url: <href verbatim> }` (`html.rs:306`) and
   `ImageNode { url: <src verbatim> }` (`html.rs:328,430`). `html5minify` is a
   size **minifier**, *not* a security sanitizer. So a markdown block containing
   `<img src="http://tracker/?leak=…">` or `<a href="javascript:…">` renders a
   live external-image fetch / live `javascript:` link with **zero**
   sanitization. **Closed by P4** (HTML never reaches `html.rs`) with **paint-time
   sanitization (D5) as defense-in-depth backstop**.
2. **All links open unvalidated.** `cx.open_url` is called on *any* URL
   (`inline.rs:359`, `node.rs:620`). **Closed by D7** (route through
   `validate_link_url`).

---

## 8. Deferrals & seams

- **Inline-image artifact fetch (§6.1):** artifact API absent → render artifact
  refs as link-placeholders, external images as links (never fetched). Wire the
  fetch when the API lands. Behavior shipped now (external = link) is correct
  regardless.
- **`navigate_to_file` handler:** T-3 emits only; resolve/open-editor is the
  workspace doc. Mark the seam **PROVISIONAL** ([[premature-layer-boundary-binding]])
  until the workspace side is concrete.
- **mdstitch API:** confirm its exact interface at integration (it "closes
  unterminated syntax token-by-token").
- **Math/LaTeX (§6.1):** render literally, out of scope.
- **HTML highlighter grammar:** if the vendored highlighter lacks an `html`
  grammar, the escaped-source block is plain monospace (acceptable).

---

## 9. Verification

- **Gate:** `xtask gate` green on 1.95 incl. clippy `-D warnings`; add `md/` to
  the gate's explicit `-p` scope ([[xtask-gate-scope]], [[per-task-gate-must-run-clippy]]).
- **Streaming identity:** headless test — appended deltas + finalize swap do not
  change the row's `EntityId`; plus a real-window streaming probe (held scroll +
  held selection survive the swap; per-frame build cost ~O(changed blocks), not
  O(doc)). Heed the real-window harness traps ([[terminal-realwindow-harness-pitfalls]],
  [[gpui-list-scroll-and-realwindow-probe-gotchas]], [[gpui-test-noop-text-system]]).
- **Security:** adversarial-fixture tests — `javascript:`/`data:`/`file:` links
  inert; external image → link, not fetch; embedded-HTML `<a>`/`<img>` render as
  source, not live; path autolink emits `navigate_to_file`.
- **Backtick-gating:** unit test per §6.2 fence form.
- **Reasoning:** render tests for the four §7 states (live / collapsed / expanded
  full / encrypted).
- **Demo:** on-device screenshot ([[gpui-ondevice-screenshot-workflow]]) — never
  run the demo during `xtask gate`.

---

## 10. Open items for the plan

1. Row content plumbing shape (`§5` here) — extend `RowPresentation` vs. read the
   item.
2. Reconfirm the four patch line-anchors at vendor time (0.5.1 may drift).
3. `mdstitch` integration point relative to the throttle (P1) — safe-prefix runs
   on accumulated text *before* the throttled handoff.
4. Whether file-path autolink detection reuses `pulldown-cmark` span info or a
   standalone scanner over rendered text spans.
