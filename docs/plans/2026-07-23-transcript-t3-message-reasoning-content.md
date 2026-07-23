# Transcript T-3: Message & Reasoning Content — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace T-2 transcript row stubs with real per-channel content rendering — assistant markdown (streaming + safe-prefix), user verbatim + backtick-gating, the §2.5 link/image security boundary, file-path autolink emission, and reasoning lifecycle UI — without remounting on finalize.

**Architecture:** Vendor gpui-component `0.5.1` `src/text/` into `lens-ui/src/md/` behind a thin `MarkdownView` wrapper keyed by a D11 stable `ContentKey`; apply surgical patches P1–P6 at paint/parse sites. Row plumbing (`RowContent` on `RowPresentation`) routes channels in `focused/`; `security.rs` (outside `md/`) is the shared boundary for markdown paint-time gates and user autolinks. Streaming uses mdstitch close-speculatively (D1) on the same pipeline at finalize.

**Tech Stack:** Rust 1.95.0, gpui 0.2.2, gpui-component 0.5.1 (bucket-A imports only), mdstitch 0.1, markdown 1.0.0, ropey =2.0.0-beta.1, smol 2.x, TDD with `#[gpui::test]` + real-window probes (`focused_finalize_probe`, `focused_scroll_probe`).

**Design source:** `docs/specs/2026-07-23-transcript-t3-message-reasoning-content-design.md` (D1–D11 are frozen — do not re-decide).

## Global Constraints

- **Toolchain:** bump `rust-toolchain.toml` `1.91.1` → `1.95.0`; bump workspace `rust-version` in root `Cargo.toml` to `1.95`.
- **Clippy fixes (D2):** `rowsource.rs:338` `collapsible_match`; `reduce/mod.rs:507` `useless_conversion`.
- **mdstitch MSRV:** exactly `1.95.0` (zero transitive deps).
- **Dependency additions (lens-ui `Cargo.toml`):** `mdstitch = "0.1"`, `markdown = "1.0.0"`, `ropey = "=2.0.0-beta.1"`, `smol = "2"`.
- **Vendored-code lint boundary:** cap lints at `crates/lens-ui/src/md/mod.rs` (`#![allow(clippy::all, warnings)]`); **our** glue (`MarkdownView`, `security.rs`, patch sites) stays under `lints.workspace` strict.
- **Security boundary reuse rule:** `crates/lens-ui/src/security.rs` is shared by markdown paint-time (P5/P6), user autolink (T3-3), and future T-7 elicitation — one implementation, multiple call sites.
- **D11 stable-ElementId rule:** ONE `ContentKey` for the whole stream→finalize lifetime; `MarkdownView` ElementId = `content_key.as_element_id()`; **FORBID** retargeting `acc_id → item_id`; identity test asserts same `TextView` keyed-state `EntityId`.
- **D10 `scrollable(false)`:** message rows use `MarkdownView::scrollable(false)`; P3 `list_state.reset` path is **dead** for message rows; P3 applies only to §3.4 reasoning capped region if `scrollable(true)`.
- **Gate:** `cargo run -p xtask -- gate` green incl. clippy `-D warnings` (there is **no** `cargo xtask` alias; gate runs workspace clippy `-D warnings`); vendored `md/` body lint-capped, glue strict.
- **Review diversity (MANDATORY):** each non-trivial task gets ≥1 cross-family review — grok-4.5 via `cursor-delegate`; end-of-branch codex (`codex exec -s read-only`, gpt-5.6). `composer-2.5` authors, cannot review.
- **Open item #1:** P1–P6 line anchors are gpui-component `0.5.1` — **reconfirm at vendor time** before patching.

---

## File structure

**Toolchain + workspace (T3-0):**
- `rust-toolchain.toml` — channel `1.95.0`.
- `Cargo.toml` (workspace root) — `rust-version = "1.95"`.
- `crates/lens-core/src/reduce/mod.rs:507` — clippy `useless_conversion` fix (D2).
- `crates/lens-ui/src/focused/rowsource.rs:338` — clippy `collapsible_match` fix (D2).

**Vendored markdown module (T3-0):**
- `crates/lens-ui/src/md/mod.rs` — lint cap + re-exports `MarkdownView`, `init`, `safe_prefix`, `markdown_state_entity_id`.
- `crates/lens-ui/src/md/text_view.rs` — vendored; P1 throttle (~178, ~628), P2 reparse-clear (~610) + `update_bounds` carve-out (253–256).
- `crates/lens-ui/src/md/node.rs` — vendored; P3 `render_root` `list_state.reset` (~1123, Task 5 only); P5 `img()` gate (~609).
- `crates/lens-ui/src/md/inline.rs` — vendored; P6 link click (~359) + link-mark strip at paint.
- `crates/lens-ui/src/md/format/markdown.rs` — vendored; P4 `Node::Html` escape (159, 297).
- `crates/lens-ui/src/md/format/mod.rs` — drop `html`/`html5minify` mods.
- `crates/lens-ui/src/md/global_state.rs` — co-vendored from gpui-component `global_state.rs`, rewritten for `md::TextViewState`.
- `crates/lens-ui/Cargo.toml` — new deps.
- `crates/lens-app/src/main.rs:~97` — `lens_ui::md::init(cx)` next to `gpui_component::init`.

**Row plumbing (T3-M):**
- `crates/lens-core/src/domain/item.rs` — `ReasoningAcc.started_at_ms: Option<i64>`.
- `crates/lens-core/src/reduce/mod.rs` — stamp `started_at_ms` on `ReasoningStarted`.
- `crates/lens-core/src/reduce/scratch.rs` — propagate stamp on late-open accumulator.
- `crates/lens-ui/src/focused/content_key.rs` — `ContentKey`.
- `crates/lens-ui/src/focused/rowsource.rs:53` — `RowContent` replaces flat `text`.
- `crates/lens-ui/src/focused/rowsource.rs:402-424` — finalize keeps `content_key` (D11).
- `crates/lens-ui/src/focused/mod.rs` — `stream_presentation`, `commit_pending_disk_rows` use typed content.

**Assistant markdown (T3-1):**
- `crates/lens-ui/src/focused/view.rs` — `render_content_row` dispatches `AssistantMarkdown`.
- `crates/lens-ui/src/focused/streaming.rs` — frame coalesce + `safe_prefix` pipeline.
- `crates/lens-ui/tests/markdown_identity.rs` — headless D11 identity test.
- `crates/lens-ui/src/bin/focused_finalize_probe.rs` — extend for streaming markdown identity.

**Security + autolink (T3-2):**
- `crates/lens-ui/src/security.rs` — `validate_link_url`, `validate_image_ref`.
- `crates/lens-ui/src/lib.rs` — `pub mod security`.
- `crates/lens-ui/src/focused/content_events.rs` — `ContentUiEvent`, `NavigateToFile`.
- `crates/lens-ui/src/focused/autolink.rs` — `scan_prose_autolinks`, `AutolinkHit`.
- `crates/lens-ui/src/md/node.rs`, `inline.rs` — P5/P6 paint-time patches.
- `crates/lens-ui/tests/security_adversarial.rs` — threat-matrix fixtures.

**User messages (T3-3):**
- `crates/lens-ui/src/focused/user_content.rs` — `split_user_segments`, prose renderer.
- `crates/lens-ui/src/focused/view.rs` — `UserVerbatim` dispatch.

**Reasoning (T3-4):**
- `crates/lens-ui/src/focused/reasoning.rs` — four §7 states, capped scroll region.
- `crates/lens-ui/src/md/node.rs` — P3 scroll-preserving splice (conditional on `scrollable`).

---

### Task 0 — T3-0 Infra + vendor

Riskiest first. Proves vendoring compiles before the rest of the plan depends on it.

**Files:**
- Modify: `rust-toolchain.toml`, `Cargo.toml` (workspace root)
- Modify: `crates/lens-ui/Cargo.toml`, `crates/lens-ui/src/lib.rs`
- Modify: `crates/lens-core/src/reduce/mod.rs:507`, `crates/lens-ui/src/focused/rowsource.rs:338`
- Modify: `crates/lens-app/src/main.rs:~97`
- Create: `crates/lens-ui/src/md/` (full vendor tree)
- Test: `crates/lens-ui/src/md/mod.rs` (`#[cfg(test)]`), `crates/lens-ui/tests/md_smoke.rs`

**Interfaces:**
- Produces (consumed by T3-M onward):
  ```rust
  // Task 0 smoke: id is SharedString/ElementId-compatible.
  // Task 2 upgrades the public wrapper to take `&ContentKey`.
  pub fn md::init(cx: &mut App);
  pub fn safe_prefix(text: &str) -> String;
  pub struct MarkdownView;
  impl MarkdownView {
      pub fn new(id: impl Into<SharedString>, markdown: impl Into<SharedString>, window: &mut Window, cx: &mut App) -> Self;
      pub fn scrollable(self, scrollable: bool) -> Self;
      pub fn selectable(self, selectable: bool) -> Self;
      pub fn into_inner(self) -> impl IntoElement;
  }
  pub fn markdown_state_entity_id(id: &str, window: &mut Window, cx: &mut App) -> Option<EntityId>;
  ```
  (`ContentKey` lands in Task 1. Task 2 Step 4 changes `new` / `markdown_state_entity_id` to take `&ContentKey`.)

### 0A — Pre-plan de-risk (throwaway compile)

- [ ] **Step 1: Throwaway vendor smoke — expect compile probe.** Copy vendor tree to a temp branch path and run the de-risk check **before** committing vendor files:
  ```bash
  GPUI_SRC="$HOME/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-component-0.5.1"
  mkdir -p crates/lens-ui/src/md
  cp -R "$GPUI_SRC/src/text/"* crates/lens-ui/src/md/
  cp "$GPUI_SRC/src/global_state.rs" crates/lens-ui/src/md/global_state.rs
  ```
  Apply minimal `sed` re-point (`crate::` → `gpui_component::`, `crate::text::` → `crate::md::`) on the copy, add deps to `Cargo.toml`, write stub `md/mod.rs` with lint cap + `init`. **Do not** land P1–P4 yet — only prove deps + Html excision skeleton compile.
  ```bash
  cargo check -p lens-ui 2>&1 | tail -20
  ```
  **Expected:** either PASS (de-risk green) or a bounded error list (missing `ropey`/`smol`/`markdown`, `html` mod refs) — fix until PASS. If FAIL persists after dep + Html-excision stubs, stop and revise vendor mapping before writing remaining plan steps.
  **Also pin here (blocks the D11 identity test):** confirm the real gpui 0.2.2 keyed-state EntityId accessor — how `window.use_keyed_state::<T>(key, cx, init)` returns the `Entity<T>` and how to read its `EntityId` (`.entity_id()`). `markdown_state_entity_id` (Step 9) and the identity test (Task 2 Steps 2/4) both depend on this exact shape; resolve it before Task 2 or the no-remount guarantee is untestable.

### 0B — Toolchain + clippy gate prep (D2)

- [ ] **Step 2: Failing test — mdstitch requires 1.95.** Add `mdstitch = "0.1"` to `crates/lens-ui/Cargo.toml` only; run:
  ```bash
  cargo check -p lens-ui 2>&1 | rg -i "rustc|MSRV|mdstitch" | head -5
  ```
  **Expected on 1.91.1:** MSRV error mentioning `1.95.0`.

- [ ] **Step 3: Bump toolchain.** In `rust-toolchain.toml`:
  ```toml
  [toolchain]
  channel = "1.95.0"
  components = ["rustfmt", "clippy", "rust-analyzer"]
  profile = "minimal"
  ```
  In root `Cargo.toml` workspace `[workspace.package]`:
  ```toml
  rust-version = "1.95"
  ```
  Run `rustc --version` → **Expected:** `rustc 1.95.0`.

- [ ] **Step 4: Fix clippy `collapsible_match` at `rowsource.rs:338`.** Replace the nested `if !any(...)` arm inside `RowKind::StreamingMessage` with a single `matches!` guard (reconfirm line at edit time):
  ```rust
  RowKind::StreamingMessage => {
      let absent = !self.structure.iter().any(|e| {
          matches!(e, StructureEntry::Sibling(row) if row == &id)
      });
      if absent {
          self.structure.push(StructureEntry::Sibling(id));
          changed = true;
      }
  }
  ```
  Run: `cargo clippy -p lens-ui --all-targets -- -D warnings 2>&1 | rg collapsible_match` → **Expected:** empty (no match).

- [ ] **Step 5: Fix clippy `useless_conversion` at `reduce/mod.rs:507`.** In the test helper `reduce_batch`, replace:
  ```rust
  all.extend(reduce(state, ev, clock).into_iter());
  ```
  with:
  ```rust
  all.extend(reduce(state, ev, clock));
  ```
  Run: `cargo clippy -p lens-core --all-targets -- -D warnings 2>&1 | rg useless_conversion` → **Expected:** empty.

- [ ] **Step 6: Commit toolchain + clippy fixes.**
  ```bash
  git add rust-toolchain.toml Cargo.toml crates/lens-core/src/reduce/mod.rs crates/lens-ui/src/focused/rowsource.rs
  git commit -m "chore(toolchain): bump to 1.95.0 + T3 clippy fixes (D2)"
  ```

### 0C — Vendor `text/` → `md/` + deps

- [ ] **Step 7: Add lens-ui deps.** In `crates/lens-ui/Cargo.toml` `[dependencies]`:
  ```toml
  mdstitch = "0.1"
  markdown = "1.0.0"
  ropey = "=2.0.0-beta.1"
  smol = "2"
  ```
  Run: `cargo fetch -p lens-ui` → **Expected:** PASS.

- [ ] **Step 8: Copy vendor sources (reconfirm paths).**
  ```bash
  GPUI_SRC="$HOME/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-component-0.5.1"
  rm -rf crates/lens-ui/src/md
  mkdir -p crates/lens-ui/src/md/format
  cp "$GPUI_SRC/src/text/"{inline,node,style,text_view,utils}.rs crates/lens-ui/src/md/
  cp "$GPUI_SRC/src/text/mod.rs" crates/lens-ui/src/md/vendor_mod.rs
  cp "$GPUI_SRC/src/text/format/"{markdown,mod}.rs crates/lens-ui/src/md/format/
  cp "$GPUI_SRC/src/global_state.rs" crates/lens-ui/src/md/global_state.rs
  rm -f crates/lens-ui/src/md/format/html.rs
  rm -rf crates/lens-ui/src/md/format/html5minify
  ```

- [ ] **Step 9: Create `md/mod.rs` (lint cap + public surface).**
  ```rust
  #![allow(clippy::all, warnings)]

  mod format;
  mod global_state;
  mod inline;
  mod node;
  mod style;
  mod text_view;
  mod utils;

  use gpui::{App, EntityId, SharedString, Window};
  use mdstitch::{stitch, StitchOptions};

  pub use style::*;
  pub use text_view::TextView;

  pub fn init(cx: &mut App) {
      global_state::init(cx);
      text_view::init(cx);
  }

  pub fn safe_prefix(text: &str) -> String {
      stitch(text, &StitchOptions::default()).into_owned()
  }

  pub struct MarkdownView {
      inner: TextView,
  }

  impl MarkdownView {
      pub fn new(
          id: impl Into<SharedString>,
          markdown: impl Into<SharedString>,
          window: &mut Window,
          cx: &mut App,
      ) -> Self {
          Self {
              // TextView::markdown takes `impl Into<ElementId>`; SharedString: Into<ElementId>.
              inner: TextView::markdown(id.into(), markdown, window, cx),
          }
      }

      pub fn scrollable(mut self, scrollable: bool) -> Self {
          self.inner = self.inner.scrollable(scrollable);
          self
      }

      pub fn selectable(mut self, selectable: bool) -> Self {
          self.inner = self.inner.selectable(selectable);
          self
      }

      pub fn into_inner(self) -> TextView {
          self.inner
      }
  }

  // NOTE: the exact gpui 0.2.2 keyed-state EntityId accessor is PINNED in 0A
  // (de-risk). `use_keyed_state` returns an `Entity<T>`; read its `.entity_id()`.
  // Signature is canonical here AND in Task 2 Step 4: `id: &str -> Option<EntityId>`.
  pub fn markdown_state_entity_id(
      id: &str,
      window: &mut Window,
      cx: &mut App,
  ) -> Option<EntityId> {
      let key = SharedString::from(format!("{id}/state"));
      let state = window.use_keyed_state::<text_view::TextViewState>(
          key,
          cx,
          |_, cx| text_view::TextViewState::new(cx),
      );
      Some(state.entity_id())
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn safe_prefix_closes_bold() {
          assert_eq!(safe_prefix("**wor"), "**wor**");
      }
  }
  ```
  Wire `pub mod md;` in `crates/lens-ui/src/lib.rs`.

- [ ] **Step 10: Rewrite bucket-A imports in every vendored file.** Mechanical replacements (run from repo root):
  ```bash
  rg -l 'crate::(theme|styled|highlighter|input|scroll|icon|tooltip)' crates/lens-ui/src/md \
    | xargs sed -i '' 's/crate::theme::/gpui_component::theme::/g; s/crate::styled::/gpui_component::styled::/g; s/crate::highlighter::/gpui_component::highlighter::/g; s/crate::input::/gpui_component::input::/g; s/crate::scroll::/gpui_component::scroll::/g; s/crate::icon::/gpui_component::icon::/g; s/crate::tooltip::/gpui_component::tooltip::/g'
  sed -i '' 's/crate::text::/crate::md::/g' crates/lens-ui/src/md/global_state.rs
  sed -i '' 's/use crate::text_view/use crate::md::text_view/g' crates/lens-ui/src/md/global_state.rs
  ```
  In `global_state.rs`, change `TextViewState` import path to `crate::md::text_view::TextViewState` and keep `init` calling `cx.set_global(GlobalState::new())`.

- [ ] **Step 11: Html API excision (D6).** In `md/format/mod.rs` **after** copy, replace entire file:
  ```rust
  pub(super) mod markdown;
  ```
  In `md/text_view.rs`:
  - Delete `TextViewType::Html` variant (was ~114).
  - Delete `TextView::html` constructor (was ~438).
  - In `parse_content`, remove the `TextViewType::Html` match arm.
  - Keep only `TextViewType::Markdown`.
  Run: `cargo check -p lens-ui 2>&1 | rg "html::|TextViewType::Html|html5minify"` → **Expected:** empty (no references).

- [ ] **Step 12: Wire `md::init` at app startup.** In `crates/lens-app/src/main.rs` inside the `Application::new().run` closure, immediately after `gpui_component::init(cx);`:
  ```rust
  lens_ui::md::init(cx);
  ```
  Run: `cargo check -p lens-app` → **Expected:** PASS.

- [ ] **Step 13: Run safe_prefix unit test.**
  ```bash
  cargo test -p lens-ui md::tests::safe_prefix_closes_bold -- --nocapture
  ```
  **Expected:** `test md::tests::safe_prefix_closes_bold ... ok`

### 0D — Structural patches P1–P4

**Reconfirm line anchors in `text_view.rs` / `format/markdown.rs` at edit time (open item #1).**

#### P1 — Interval throttle (was trailing debounce reset ~178, delay ~628)

- [ ] **Step 14: Patch `UpdateFuture` fields + constructor.** In `md/text_view.rs`, extend `UpdateFuture`:
  ```rust
  struct UpdateFuture {
      type_: TextViewType,
      highlight_theme: Arc<HighlightTheme>,
      current_style: TextViewStyle,
      current_text: SharedString,
      timer: Timer,
      rx: Pin<Box<smol::channel::Receiver<Update>>>,
      tx_result: smol::channel::Sender<Result<ParsedContent, SharedString>>,
      delay: Duration,
      code_block_actions: Option<Arc<CodeBlockActionsFn>>,
      throttle_armed: bool,
      pending_parse: bool,
  }
  ```
  In `UpdateFuture::new`, initialize `throttle_armed: false, pending_parse: false`.

- [ ] **Step 15: Replace `UpdateFuture::poll` debounce body.** **Before (vendor ~178–181):**
  ```rust
  if changed {
      let delay = self.delay;
      self.timer.set_after(delay);
  }
  ```
  **After (interval throttle — never reset per token):**
  ```rust
  if changed {
      self.pending_parse = true;
      if !self.throttle_armed {
          self.throttle_armed = true;
          self.timer.set_after(self.delay);
      }
  }
  ```
  And in the timer-ready arm (~188–198), **after** `try_send(res)`:
  ```rust
  if self.pending_parse {
      self.pending_parse = false;
      self.throttle_armed = true;
      self.timer.set_after(self.delay);
  } else {
      self.throttle_armed = false;
  }
  ```

- [ ] **Step 16: Set throttle delay to 100 ms (tunable 50–200).** In `TextView` init spawn (~628), change:
  ```rust
  Duration::from_millis(200),
  ```
  to:
  ```rust
  Duration::from_millis(100),
  ```

#### P2 — Drop reparse `clear_selection` (~610) + `update_bounds` carve-out (253–256)

- [ ] **Step 17: Remove reparse selection clear.** In `text_view.rs` async reparse handler (~604–611), delete the line:
  ```rust
  state.clear_selection();
  ```
  so the block becomes:
  ```rust
  _ = state.update(cx, |state, cx| {
      state.parsed_result = Some(parsed_result);
      if let Some(parent_entity) = state.parent_entity {
          let app = &mut **cx;
          app.notify(parent_entity);
      }
  });
  ```

- [ ] **Step 18: Carve out `update_bounds` size-change clear.** **Before (vendor 253–256):**
  ```rust
  fn update_bounds(&mut self, bounds: Bounds<Pixels>) {
      if self.bounds.size != bounds.size {
          self.clear_selection();
      }
      self.bounds = bounds;
  }
  ```
  **After:**
  ```rust
  fn update_bounds(&mut self, bounds: Bounds<Pixels>, preserve_selection_on_resize: bool) {
      if self.bounds.size != bounds.size && !preserve_selection_on_resize {
          self.clear_selection();
      }
      self.bounds = bounds;
  }
  ```
  At the `TextViewElement` paint site that calls `update_bounds`, pass `preserve_selection_on_resize: true` when the parent `TextView` has active text updates. Concretely: add `streaming: Arc<AtomicBool>` to `TextViewState`, set it `true` in the `Update::Text` arm of `UpdateFuture::poll` and `false` once the reparse result is applied; the paint site reads `state.streaming.load(Ordering::Relaxed)` and forwards it as the flag. **This is the least-specified patch — confirm the actual `TextViewElement`→`update_bounds` call path at vendor time, and keep the `AtomicBool` plumbing in a small strict (non-lint-capped) helper so a reviewer can see it. The Step 8 unit test + Step 9 probe are the proof it works.**

#### P4 — `Node::Html` → escaped source (D6)

- [ ] **Step 19: Patch block HTML arm in `format/markdown.rs` (~297).** **Before:**
  ```rust
  Node::Html(val) => match super::html::parse(&val.value, cx) {
      Ok(el) => el,
      Err(err) => {
          if cfg!(debug_assertions) {
              tracing::warn!("error parsing html: {:#?}", err);
          }
          node::Node::Paragraph(Paragraph::new(val.value))
      }
  },
  ```
  **After:**
  ```rust
  Node::Html(val) => node::Node::CodeBlock(CodeBlock::new(
      val.value.clone().into(),
      Some("html".into()),
      style,
      highlight_theme,
  )),
  ```

- [ ] **Step 20: Patch inline HTML arm (~159).** **Before:**
  ```rust
  Node::Html(val) => match super::html::parse(&val.value, cx) {
      Ok(el) => {
          if el.is_break() {
              text = "\n".to_owned();
              paragraph.push(InlineNode::new(&text));
          } else if cfg!(debug_assertions) {
              tracing::warn!("unsupported inline html tag: {:#?}", el);
          }
      }
      Err(err) => {
          if cfg!(debug_assertions) {
              tracing::warn!("failed parsing html: {:#?}", err);
          }
          text.push_str(&val.value);
      }
  },
  ```
  **After:**
  ```rust
  Node::Html(val) => {
      let start = text.len();
      text.push_str(&val.value);
      paragraph.push(
          InlineNode::new(&text).marks(vec![(start..text.len(), TextMark::default().code())]),
      );
  }
  ```

- [ ] **Step 21: Unit test — block HTML parse path no longer calls html module.** In `crates/lens-ui/src/md/format/markdown.rs` `#[cfg(test)]` (or `md/mod.rs` tests), assert `safe_prefix` + markdown parse of a block-HTML doc yields a `CodeBlock` (language `html`) rather than panicking on missing `html` module:
  ```rust
  #[test]
  fn block_html_becomes_html_codeblock_source() {
      use crate::md::safe_prefix;
      let src = safe_prefix("<a href=\"javascript:alert(1)\">x</a>\n\nparagraph");
      assert!(src.contains("<a href="));
      // Compile-time: format::html is gone — this test exists so a regressive
      // re-add of `super::html::parse` fails the module build before runtime.
      let _ = src;
  }
  ```
  Create `crates/lens-ui/tests/md_smoke.rs` that only checks `safe_prefix` + `md::init` compile-link:
  ```rust
  #[test]
  fn safe_prefix_and_init_link() {
      let s = lens_ui::md::safe_prefix("**wor");
      assert_eq!(s, "**wor**");
  }
  ```
  Run: `cargo test -p lens-ui --test md_smoke` → **Expected:** PASS after patches.

- [ ] **Step 22: Run `cargo check -p lens-ui`.** **Expected:** 0 errors, 0 warnings in glue; vendored body under lint cap.

- [ ] **Step 23: Commit vendor + P1–P4.**
  ```bash
  git add crates/lens-ui/src/md crates/lens-ui/Cargo.toml crates/lens-ui/src/lib.rs crates/lens-app/src/main.rs crates/lens-ui/tests/md_smoke.rs
  git commit -m "feat(md): vendor gpui-component text module + P1-P4 streaming/html patches"
  ```

- [ ] **Step 24: Cross-family review** T3-0 diff (grok-4.5 via `cursor-delegate`): bucket-A import correctness, Html excision completeness, throttle semantics, lint-cap boundary legibility. Address findings; run `cargo run -p xtask -- gate`.

---

### Task 1 — T3-M Row plumbing

First functional milestone: typed row payload + D11 content key + reasoning duration source (**PLUMB duration — do not scope Ns out for v1**).

**Files:**
- Create: `crates/lens-ui/src/focused/content_key.rs`
- Modify: `crates/lens-core/src/domain/item.rs` (`ReasoningAcc.started_at_ms` + `ItemKind::Reasoning.duration_ms`)
- Modify: `crates/lens-core/src/reduce/mod.rs` (`ReasoningStarted` handler)
- Modify: `crates/lens-core/src/reduce/scratch.rs` (`accumulate_reasoning` late-open path)
- Modify: `crates/lens-core/src/reduce/items.rs` (`finalize_reasoning` computes `duration_ms`)
- Modify: `crates/lens-ui/src/focused/rowsource.rs:53, 775-878, 402-424`
- Modify: `crates/lens-ui/src/focused/mod.rs` (`stream_presentation`, `handle_retired`, `commit_pending_disk_rows`)
- Modify: `crates/lens-ui/src/focused/view.rs` (stub reads `RowContent`)
- Modify: `crates/lens-ui/src/focused/mod.rs` (test fixtures)
- Test: `crates/lens-ui/src/focused/content_key.rs`, `crates/lens-ui/src/focused/rowsource.rs`, `crates/lens-core/src/reduce/items.rs`

**Interfaces:**
- Consumes: `AccId`, `Item`, `ReasoningAcc`, `MessageAcc`.
- Produces:
  ```rust
  #[derive(Clone, Debug, PartialEq, Eq, Hash)]
  pub struct ContentKey(String);
  impl ContentKey {
      pub fn from_acc(acc_id: &AccId) -> Self;
      pub fn from_label(label: impl Into<String>) -> Self; // non-stream keys (user ```md fences)
      pub fn as_element_id(&self) -> SharedString;
  }
  #[derive(Clone, Debug, PartialEq)]
  pub enum RowContent {
      Stub { text: String },
      AssistantMarkdown { source: String, content_key: ContentKey },
      UserVerbatim { text: String },
      Reasoning {
          summary: String,
          full: String,
          encrypted: bool,
          duration_secs: Option<u32>,
          content_key: ContentKey,
          live: bool,
      },
  }
  pub struct RowPresentation {
      pub kind: RowKind,
      pub content: RowContent,
      pub collapsed: bool,
      pub height_hint: Option<f32>,
  }
  // ReasoningAcc gains (transient stream scratch — DROPPED at finalize):
  pub started_at_ms: Option<i64>;
  // ItemKind::Reasoning gains (DURABLE — the finalized row's only duration source):
  pub duration_ms: Option<i64>;
  ```

### 1A — Duration source decision + lens-core stamp

- [ ] **Step 1: Failing test — duration_secs from timestamps.** In `crates/lens-core/src/reduce/items.rs` tests, add:
  ```rust
  #[test]
  fn reasoning_duration_secs_from_started_to_finalize() {
      use crate::clock::ManualClock;
      use crate::reduce::reduce;
      use lens_client::stream::{ResponseEvent, ServerStreamEvent};

      let mut s = empty_state();
      let clock = ManualClock::new(1_000_000);
      reduce(
          &mut s,
          &ServerStreamEvent::Response(ResponseEvent::ReasoningStarted),
          &clock,
      );
      let started = s
          .stream
          .open_reasoning
          .as_ref()
          .and_then(|a| a.started_at_ms)
          .expect("started_at_ms stamped on ReasoningStarted");
      assert_eq!(started, 1_000_000);
      clock.advance_ms(4_500);
      reduce(
          &mut s,
          &ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta {
              delta: "thought".into(),
          }),
          &clock,
      );
      reduce(
          &mut s,
          &ServerStreamEvent::Response(ResponseEvent::ReasoningClosed {
              full_text: "thought".into(),
              summary_text: "t".into(),
          }),
          &clock,
      );
      assert!(s.stream.open_reasoning.is_none());
      let item = s.items.last().expect("reasoning item");
      let ItemKind::Reasoning { duration_ms, .. } = &item.kind else {
          panic!("expected reasoning, got {:?}", item.kind);
      };
      // The acc (and its started_at_ms) is CONSUMED by finalize_reasoning, so a
      // finalized/collapsed row has no acc to read. Duration MUST be persisted on
      // the durable Item at close = created_at - started_at_ms.
      assert_eq!(item.created_at, 1_004_500);
      assert_eq!(*duration_ms, Some(4_500));
      let _ = started;
  }
  ```
  Run: `cargo test -p lens-core reasoning_duration_secs_from_started_to_finalize` → **Expected:** FAIL (`started_at_ms` field missing / field not stamped).
  **Note:** `ReasoningClosed { full_text, summary_text }` is the live wire shape (`lens_client::stream::event.rs`).

- [ ] **Step 2: Add `started_at_ms` to `ReasoningAcc`.** In `crates/lens-core/src/domain/item.rs`:
  ```rust
  #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
  pub struct ReasoningAcc {
      pub acc_id: AccId,
      pub full_text: String,
      pub summary_text: String,
      pub encrypted: bool,
      pub started_at_ms: Option<i64>,
  }
  ```
  In the SAME file, add `duration_ms` to the durable `ItemKind::Reasoning` variant — the acc is dropped at finalize, so the item must carry the duration itself:
  ```rust
  ItemKind::Reasoning {
      full_text: String,
      summary_text: String,
      encrypted: bool,
      duration_ms: Option<i64>,
  }
  ```
  Then fix every existing `ItemKind::Reasoning { .. }` construction and match for the new field (`rg 'ItemKind::Reasoning'` — `finalize_reasoning` in `reduce/items.rs`, snapshot/serde, and any test literals).

- [ ] **Step 3: Stamp on `ReasoningStarted`.** In `crates/lens-core/src/reduce/mod.rs` `ResponseEvent::ReasoningStarted` arm:
  ```rust
  ResponseEvent::ReasoningStarted => {
      if state.stream.open_reasoning.is_none() {
          let acc_id = state.mint_acc_id();
          state.stream.open_reasoning = Some(ReasoningAcc {
              acc_id,
              started_at_ms: Some(clock.now_millis()),
              ..Default::default()
          });
      }
      smallvec![StreamUpdate::ScratchChanged(Arc::new(state.stream.clone()))]
  }
  ```

- [ ] **Step 4: Late-open reasoning inherits stamp.** Change `accumulate_reasoning` signature and body:
  ```rust
  pub(crate) fn accumulate_reasoning(
      scratch: &mut StreamScratch,
      kind: ReasoningKind,
      delta: &str,
      new_acc_id: Option<AccId>,
      clock: &dyn crate::clock::Clock,
  ) -> Updates {
      let now = clock.now_millis();
      let acc = scratch.open_reasoning.get_or_insert_with(|| ReasoningAcc {
          acc_id: new_acc_id.expect("pre-minted acc_id required when opening reasoning acc"),
          started_at_ms: Some(now),
          ..Default::default()
      });
      match kind {
          ReasoningKind::Full => acc.full_text.push_str(delta),
          ReasoningKind::Summary => acc.summary_text.push_str(delta),
      }
      smallvec![StreamUpdate::ScratchChanged(Arc::new(scratch.clone()))]
  }
  ```
  Update every call site in `reduce/mod.rs` to pass `clock`. Run `cargo check -p lens-core` → green.

- [ ] **Step 4b: Compute + persist `duration_ms` in `finalize_reasoning`.** In `crates/lens-core/src/reduce/items.rs` (`finalize_reasoning`, ~139), capture `started_at_ms` off the acc BEFORE it is moved into `ItemKind::Reasoning`, and set the durable field:
  ```rust
  pub(crate) fn finalize_reasoning(state: &mut SessionState, clock: &dyn Clock) -> Updates {
      let Some(acc): Option<ReasoningAcc> = state.stream.open_reasoning.take() else {
          return smallvec![];
      };
      let acc_id = acc.acc_id.clone();
      let id = local_id("reasoning", state);
      let duration_ms = acc.started_at_ms.map(|start| (clock.now_millis() - start).max(0));
      let kind = ItemKind::Reasoning {
          full_text: acc.full_text,
          summary_text: acc.summary_text,
          encrypted: acc.encrypted,
          duration_ms,
      };
      let response_id = state.active_response.clone();
      let mut u = push_item(state, id.clone(), kind, None, response_id, clock);
      u.push(StreamUpdate::Retired {
          acc_id,
          disposition: RetireDisposition::Finalizing { item_id: id },
      });
      u
  }
  ```
  `push_item` stamps `created_at` from the same `clock`, so `created_at - started_at_ms == duration_ms` (Step 1 asserts both). This closes the acc-dropped-at-finalize gap — without it, finalized reasoning rows have no duration source and always render bare "💭 thought".

- [ ] **Step 5: Run duration test — PASS.** `cargo test -p lens-core reasoning_duration_secs` → green.

- [ ] **Step 6: Commit.**
  ```bash
  git add crates/lens-core
  git commit -m "feat(reduce): stamp ReasoningAcc.started_at_ms for duration plumbing"
  ```

### 1B — `ContentKey` + `RowContent`

- [ ] **Step 7: Failing test — `ContentKey::from_acc` stable.** In `crates/lens-ui/src/focused/content_key.rs`:
  ```rust
  use lens_core::domain::ids::AccId;

  #[derive(Clone, Debug, PartialEq, Eq, Hash)]
  pub struct ContentKey(String);

  impl ContentKey {
      pub fn from_acc(acc_id: &AccId) -> Self {
          Self(format!("md:{}", acc_id.as_str()))
      }

      pub fn from_label(label: impl Into<String>) -> Self {
          Self(format!("md:{}", label.into()))
      }

      pub fn as_element_id(&self) -> gpui::SharedString {
          gpui::SharedString::from(self.0.as_str())
      }
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn from_acc_format() {
          let key = ContentKey::from_acc(&AccId::new("acc_1"));
          assert_eq!(key.0, "md:acc_1");
          assert_eq!(key.as_element_id().as_str(), "md:acc_1");
      }

      #[test]
      fn from_label_format() {
          let key = ContentKey::from_label("user-md-0");
          assert_eq!(key.0, "md:user-md-0");
      }
  }
  ```
  Run: `cargo test -p lens-ui focused::content_key` → **Expected:** PASS (after `pub mod content_key` in `focused/mod.rs`).

- [ ] **Step 8: Failing test — `RowPresentation` carries `RowContent`.** In `rowsource.rs` tests, update a `RowPresentation` literal — **Expected:** compile FAIL (`text` field removed).

- [ ] **Step 9: Replace flat `text` with `RowContent`.** In `rowsource.rs:53`:
  ```rust
  #[derive(Clone, Debug, PartialEq)]
  pub struct RowPresentation {
      pub kind: RowKind,
      pub content: RowContent,
      pub collapsed: bool,
      pub height_hint: Option<f32>,
  }
  ```
  Add `RowContent` enum to `focused/rowsource.rs` (or `focused/content.rs` re-exported). Add helper:
  ```rust
  impl RowContent {
      pub fn stub_text(&self) -> &str {
          match self {
              RowContent::Stub { text } => text.as_str(),
              RowContent::AssistantMarkdown { source, .. } => source.as_str(),
              RowContent::UserVerbatim { text } => text.as_str(),
              RowContent::Reasoning { full, .. } => full.as_str(),
          }
      }
  }
  ```

- [ ] **Step 10: Update materializers.** `materialize_streaming_message`:
  ```rust
  let content_key = ContentKey::from_acc(&acc.acc_id);
  let pres = RowPresentation {
      kind: RowKind::StreamingMessage,
      content: RowContent::AssistantMarkdown {
          source: acc.text.clone(),
          content_key,
      },
      collapsed: false,
      height_hint: None,
  };
  ```
  `materialize_streaming_reasoning`:
  ```rust
  let content_key = ContentKey::from_acc(&acc.acc_id);
  // At stream time duration is None; live UI shows "thinking…". Duration is
  // stamped at finalize from item.created_at - acc.started_at_ms (Step 14).
  let pres = RowPresentation {
      kind: RowKind::StreamingReasoning,
      content: RowContent::Reasoning {
          summary: acc.summary_text.clone(),
          full: acc.full_text.clone(),
          encrypted: acc.encrypted,
          duration_secs: None,
          content_key,
          live: true,
      },
      collapsed: false,
      height_hint: None,
  };
  ```
  For finalized assistant messages, `presentation_for_item` must **not** mint a
  new key from `item.id`. Finalize carries the stream key via
  `commit_stream_finalize` (Step 11): the disk projection for a message that
  never streamed (user paste / snapshot) may mint `ContentKey::from_acc(&AccId::new(item.id.as_str()))`
  as a stable non-stream key. Streaming→finalize always preserves the stream key.

- [ ] **Step 11: Preserve `content_key` through finalize (D11).** Extend `RowStore::commit_stream_finalize` (~402-424): when swapping `tail_id → durable_id`, merge durable presentation fields but **keep** the prior `content_key` inside `RowContent::AssistantMarkdown` / `Reasoning`. Add failing test in `rowsource.rs`:
  ```rust
  #[gpui::test]
  fn finalize_preserves_content_key(cx: &mut gpui::TestAppContext) {
      let acc = AccId::new("acc_stream_1");
      let key = ContentKey::from_acc(&acc);
      let item_id = ItemId::new("item_1");
      let mut store = RowStore::new();
      cx.update(|cx| {
          store.stage_stream_finalize(
              &acc,
              RowPresentation {
                  kind: RowKind::StreamingMessage,
                  content: RowContent::AssistantMarkdown {
                      source: "hello".into(),
                      content_key: key.clone(),
                  },
                  collapsed: false,
                  height_hint: None,
              },
              None,
              None,
              cx,
          );
          store.commit_stream_finalize(
              &acc,
              &item_id,
              RowPresentation {
                  kind: RowKind::Message,
                  content: RowContent::AssistantMarkdown {
                      source: "hello world".into(),
                      // Naive implementers would retarget to item id here — forbidden.
                      content_key: ContentKey::from_acc(&AccId::new(item_id.as_str())),
                  },
                  collapsed: false,
                  height_hint: None,
              },
              true,
              None,
              cx,
          );
      });
      cx.read(|cx| {
          let id = RowId::Sibling(item_id);
          let final_pres = &store.entity(&id).unwrap().read(cx).presentation;
          match &final_pres.content {
              RowContent::AssistantMarkdown { source, content_key } => {
                  assert_eq!(source, "hello world");
                  assert_eq!(content_key, &key, "D11: content_key must stay the stream key");
              }
              other => panic!("unexpected {other:?}"),
          }
      });
  }
  ```
  Implement by having `commit_stream_finalize` take the stream presentation's `content_key` when the new `pres` also carries markdown/reasoning content (overwrite source/kind/collapsed, preserve key). Run: `cargo test -p lens-ui finalize_preserves_content_key` → **Expected:** FAIL until preserve logic lands, then PASS.

- [ ] **Step 12: Update stub renderer.** In `view.rs:130-145`, replace `pres.text` with `pres.content.stub_text()`.

- [ ] **Step 13: Fix all `RowPresentation { text: ... }` literals** in `focused/mod.rs` tests, `view.rs` tests, probes — grep `text:` under `RowPresentation` and migrate. Run: `cargo test -p lens-ui` → green.

- [ ] **Step 14: Add projection test for reasoning duration.** Unit test: projecting a finalized `Item` whose `ItemKind::Reasoning.duration_ms = Some(4_500)` yields `RowContent::Reasoning { duration_secs: Some(4), .. }` where `duration_secs = (duration_ms / 1000) as u32`. The projection reads the **durable item field** — never the (dropped) acc. (`materialize_streaming_reasoning` keeps `duration_secs: None` for live rows; the finalized-reasoning projection that reads `duration_ms` is wired in Task 5.)

- [ ] **Step 15: Commit.**
  ```bash
  git add crates/lens-ui/src/focused crates/lens-core/src/domain/item.rs
  git commit -m "feat(focused): typed RowContent + ContentKey + reasoning duration plumbing"
  ```

- [ ] **Step 16: Cross-family review** T3-M diff (grok-4.5): finalize key stability, exhaustive `text` field removal, duration arithmetic edge (negative clamp). Re-gate.

---

### Task 2 — T3-1 Assistant markdown

Wire assistant + streaming message rows to `MarkdownView` with coalesced mdstitch pipeline and D11 identity guarantees.

**Files:**
- Create: `crates/lens-ui/src/focused/streaming.rs`
- Modify: `crates/lens-ui/src/md/mod.rs` (`MarkdownView::new` stays on `SharedString` ids; callers pass `content_key.as_element_id()`)
- Modify: `crates/lens-ui/src/focused/view.rs` — real markdown renderer
- Modify: `crates/lens-ui/src/focused/mod.rs` — delta coalesce hook
- Create: `crates/lens-ui/tests/markdown_identity.rs`
- Modify: `crates/lens-ui/src/bin/focused_finalize_probe.rs`
- Test: above + inline `streaming.rs` tests

**Interfaces:**
- Consumes: `ContentKey`, `RowContent::AssistantMarkdown`, `md::{MarkdownView, safe_prefix, markdown_state_entity_id}`.
- Produces:
  ```rust
  pub struct StreamCoalescer {
      pending: String,
      last_frame: Option<std::time::Instant>,
  }
  impl StreamCoalescer {
      pub fn push_delta(&mut self, delta: &str) -> Option<String>;
      pub fn finalize(&self, final_text: &str) -> String;
  }
  pub fn render_assistant_markdown(
      content: &RowContent,
      window: &mut Window,
      cx: &mut App,
  ) -> gpui::AnyElement;
  ```

- [ ] **Step 1: Failing test — `StreamCoalescer` coalesces to frame tick.** In `streaming.rs`:
  ```rust
  use std::time::{Duration, Instant};

  pub struct StreamCoalescer {
      pending: String,
      last_emit: Option<Instant>,
      frame_budget: Duration,
  }

  impl StreamCoalescer {
      pub fn new() -> Self {
          Self {
              pending: String::new(),
              last_emit: None,
              frame_budget: Duration::from_millis(16),
          }
      }

      pub fn push_delta(&mut self, delta: &str) -> Option<String> {
          self.pending.push_str(delta);
          let now = Instant::now();
          let should_emit = self
              .last_emit
              .map(|t| now.duration_since(t) >= self.frame_budget)
              .unwrap_or(true);
          if should_emit {
              self.last_emit = Some(now);
              let stitched = crate::md::safe_prefix(&self.pending);
              Some(stitched)
          } else {
              None
          }
      }

      pub fn finalize(&self, final_text: &str) -> String {
          crate::md::safe_prefix(final_text)
      }
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn finalize_uses_same_pipeline_as_stream() {
          let mut c = StreamCoalescer::new();
          let _ = c.push_delta("**bo");
          let streamed = c.finalize("**bold**");
          assert_eq!(streamed, "**bold**");
      }
  }
  ```
  Run: `cargo test -p lens-ui streaming::tests::finalize_uses_same_pipeline` → PASS.

- [ ] **Step 2: Failing test — headless identity stable across finalize.** Create `crates/lens-ui/tests/markdown_identity.rs`:
  ```rust
  use gpui::{EntityId, TestAppContext};
  use lens_core::domain::ids::{AccId, ItemId};
  use lens_ui::focused::{
      ContentKey, RowContent, RowId, RowKind, RowPresentation, RowStore,
  };
  use lens_ui::md::{init as md_init, markdown_state_entity_id, MarkdownView};

  #[gpui::test]
  async fn markdown_entity_id_stable_across_finalize(cx: &mut TestAppContext) {
      cx.update(|cx| {
          gpui_component::init(cx);
          md_init(cx);
      });
      let acc = AccId::new("acc_id_test");
      let key = ContentKey::from_acc(&acc);
      let item_id = ItemId::new("item_final_1");
      let mut store = RowStore::new();

      let before = cx
          .add_window(|window, cx| {
              store.stage_stream_finalize(
                  &acc,
                  RowPresentation {
                      kind: RowKind::StreamingMessage,
                      content: RowContent::AssistantMarkdown {
                          source: "hi".into(),
                          content_key: key.clone(),
                      },
                      collapsed: false,
                      height_hint: None,
                  },
                  None,
                  None,
                  cx,
              );
              // Force keyed-state mint by constructing MarkdownView with the stream key.
              let _ = MarkdownView::new(key.as_element_id(), "hi", window, cx)
                  .scrollable(false)
                  .selectable(true);
              markdown_state_entity_id(key.as_element_id().as_str(), window, cx).expect("state before finalize")
          })
          .root(cx)
          .unwrap();

      let after = cx
          .add_window(|window, cx| {
              store.commit_stream_finalize(
                  &acc,
                  &item_id,
                  RowPresentation {
                      kind: RowKind::Message,
                      content: RowContent::AssistantMarkdown {
                          source: "hi there".into(),
                          content_key: ContentKey::from_acc(&AccId::new(item_id.as_str())),
                      },
                      collapsed: false,
                      height_hint: None,
                  },
                  true,
                  None,
                  cx,
              );
              // Read the element id from the store's FINALIZED row — NOT the literal
              // `key`. This is what makes the test exercise the product: if
              // commit_stream_finalize retargeted the key to the item id (the D11
              // violation), `final_key` differs from `before` and the assertion
              // fails. Hand-passing `key` here would tautologically test gpui's
              // keyed-state, not our finalize path (false-green-probe trap).
              let final_content = store
                  .entity(&RowId::Sibling(item_id.clone()))
                  .unwrap()
                  .read(cx)
                  .presentation
                  .content
                  .clone();
              let final_key = match &final_content {
                  RowContent::AssistantMarkdown { content_key, .. } => content_key.clone(),
                  other => panic!("unexpected finalized content {other:?}"),
              };
              let _ = MarkdownView::new(final_key.as_element_id(), "hi there", window, cx)
                  .scrollable(false)
                  .selectable(true);
              markdown_state_entity_id(final_key.as_element_id().as_str(), window, cx)
                  .expect("state after finalize")
          })
          .root(cx)
          .unwrap();

      // Same preserved stream key → same use_keyed_state slot → same EntityId.
      // `after` derives its key from the finalized store row, so a retarget bug
      // surfaces as a mismatch here.
      let _: EntityId = before;
      assert_eq!(before, after, "D11: finalize must preserve the stream key (no remount)");
  }
  ```
  Run: `cargo test -p lens-ui --test markdown_identity` → **Expected:** FAIL until `commit_stream_finalize` preserves `content_key` (Task 1) and `MarkdownView` keys on `ContentKey` (Step 4). Adjust window/root boilerplate to the project's live `TestAppContext` helpers if the add_window shape differs — keep the before/after `EntityId` equality assertion.

- [ ] **Step 3: Implement `render_assistant_markdown` in `view.rs`.**
  ```rust
  use crate::focused::{ContentKey, RowContent};
  use crate::md::MarkdownView;

  pub(crate) fn render_assistant_markdown(
      content: &RowContent,
      window: &mut gpui::Window,
      cx: &mut gpui::App,
  ) -> gpui::AnyElement {
      let RowContent::AssistantMarkdown { source, content_key } = content else {
          return gpui::div().into_any_element();
      };
      MarkdownView::new(content_key.as_element_id(), source.clone(), window, cx)
          .scrollable(false)
          .selectable(true)
          .into_inner()
          .into_any_element()
  }
  ```
  Update list renderer to call `render_assistant_markdown` when `matches!(pres.kind, RowKind::Message | RowKind::StreamingMessage)`.

- [ ] **Step 4: Keep `MarkdownView::new` on `SharedString` ids; callers pass `content_key.as_element_id()`.** Avoid a `md` → `focused` dependency cycle. In `md/mod.rs`:
  ```rust
  pub fn new(
      id: impl Into<SharedString>,
      markdown: impl Into<SharedString>,
      window: &mut Window,
      cx: &mut App,
  ) -> Self {
      Self {
          inner: TextView::markdown(id, markdown, window, cx),
      }
  }

  pub fn markdown_state_entity_id(
      id: &str,
      window: &mut Window,
      cx: &mut App,
  ) -> Option<EntityId> {
      let key = SharedString::from(format!("{id}/state"));
      window
          .use_keyed_state(key, cx, |_, cx| TextViewState::new(cx))
          .entity_id()
          .into() // adjust to the live keyed-state accessor — return the Entity's EntityId
  }
  ```
  Call sites:
  ```rust
  MarkdownView::new(content_key.as_element_id(), source.clone(), window, cx)
  markdown_state_entity_id(content_key.as_element_id().as_str(), window, cx)
  ```
  Confirm `use_keyed_state` return shape against gpui 0.2.2 at edit time; the identity test only needs a stable `EntityId` comparable before/after finalize.

- [ ] **Step 5: Wire streaming deltas.** This **replaces** the Task 1 Step 10 `materialize_streaming_message` body — it now runs `safe_prefix` on the accumulated text and `upsert`s the row. In `focused/mod.rs`, when scratch updates open `MessageAcc`, update the stream row in place:
  ```rust
  fn materialize_streaming_message(acc: &MessageAcc, into: &mut RowStore, cx: &mut App) {
      let content_key = ContentKey::from_acc(&acc.acc_id);
      let source = crate::md::safe_prefix(&acc.text);
      let id = RowId::StreamTail(acc.acc_id.clone());
      let pres = RowPresentation {
          kind: RowKind::StreamingMessage,
          content: RowContent::AssistantMarkdown {
              source,
              content_key,
          },
          collapsed: false,
          height_hint: None,
      };
      into.upsert(id, pres, cx);
  }
  ```
  Coalesce is optional at the replica layer when deltas already arrive batched; if per-token ScratchChanged fires, keep a `StreamCoalescer` on `FocusedTranscript` and only `upsert` when `push_delta` returns `Some`. Finalize still runs `StreamCoalescer::finalize(&final_text)` so the last streamed parse input equals the final parse input (D1 / grok #2).

- [ ] **Step 6: Run identity test — PASS.** With Task 1 key-preserve + Steps 3–5 wired, re-run:
  ```bash
  cargo test -p lens-ui --test markdown_identity
  ```
  **Expected:** PASS (`markdown_entity_id_stable_across_finalize ... ok`). If the window boilerplate from Step 2 does not compile against the live gpui test API, rewrite only the harness shell — keep the same `before == after` assertion on `markdown_state_entity_id(&key, ...)`.

- [ ] **Step 7: Extend `focused_finalize_probe.rs`.** After finalize, sample `markdown_state_entity_id` — assert unchanged; assert list anchor snapshot stable (reuse existing `AnchorSnapshot`).

- [ ] **Step 8: Failing test — selection survives height-growing update (P2).** Prefer the real-window probe (Step 9) as authoritative proof (`#[gpui::test]` NoopTextSystem can false-green paint). Add a unit guard that forces the carve-out path:
  ```rust
  // In md/text_view.rs #[cfg(test)] helpers:
  pub fn selection_is_some_for_test(state: &TextViewState) -> bool {
      state.selection_positions.0.is_some()
  }
  pub fn set_selection_for_test(state: &mut TextViewState, origin: gpui::Point<gpui::Pixels>) {
      state.selection_positions = (Some(origin), Some(origin));
      state.is_selecting = false;
  }

  #[test]
  fn update_bounds_preserves_selection_when_flagged() {
      let mut state = TextViewState::new_for_test(); // add thin test ctor
      set_selection_for_test(&mut state, gpui::point(gpui::px(1.), gpui::px(1.)));
      let grown = gpui::Bounds {
          origin: gpui::point(gpui::px(0.), gpui::px(0.)),
          size: gpui::size(gpui::px(400.), gpui::px(800.)),
      };
      state.update_bounds(grown, true);
      assert!(selection_is_some_for_test(&state));
      state.update_bounds(
          gpui::Bounds {
              origin: grown.origin,
              size: gpui::size(gpui::px(400.), gpui::px(900.)),
          },
          false,
      );
      assert!(!selection_is_some_for_test(&state));
  }
  ```
  Run: `cargo test -p lens-ui update_bounds_preserves_selection` → **Expected:** FAIL until P2 carve-out (Task 0 Step 18) lands, then PASS.

- [ ] **Step 9: Run probe.**
  ```bash
  cargo run -p lens-ui --bin focused_finalize_probe
  ```
  **Expected:** exit 0, identity + selection lines printed PASS.

- [ ] **Step 10: Commit.**
  ```bash
  git add crates/lens-ui
  git commit -m "feat(focused): assistant MarkdownView wiring + streaming coalesce + D11 identity"
  ```

- [ ] **Step 11: Cross-family review** (grok-4.5): D11 forbid acc→item key retarget, same-pipeline finalize, D10 scrollable(false). Re-gate.

---

### Task 3 — T3-2 Boundary + autolink

`security.rs` + paint-time P5/P6 + file-path autolink detect/paint/emit.

**Files:**
- Create: `crates/lens-ui/src/security.rs`
- Create: `crates/lens-ui/src/focused/content_events.rs`
- Create: `crates/lens-ui/src/focused/autolink.rs`
- Modify: `crates/lens-ui/src/md/node.rs:609` (P5), `inline.rs:359` + link-mark paint (P6)
- Modify: `crates/lens-ui/src/lib.rs`
- Create: `crates/lens-ui/tests/security_adversarial.rs`
- Test: `crates/lens-ui/src/security.rs` unit tests

**Interfaces:**
- Produces:
  ```rust
  pub enum LinkVerdict {
      AllowOpenUrl,
      NavigateToFile { path: String, line: Option<u32> },
      Strip,
  }
  pub enum ImageVerdict {
      AllowArtifactImg { url: String },
      RenderAsLink { url: String },
      Strip,
  }
  pub fn validate_link_url(url: &str) -> LinkVerdict;
  pub fn validate_image_ref(url: &str) -> ImageVerdict;
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct NavigateToFile { pub path: String, pub line: Option<u32> }
  pub enum ContentUiEvent { NavigateToFile(NavigateToFile) }
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct AutolinkHit { pub range: std::ops::Range<usize>, pub target: AutolinkTarget }
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum AutolinkTarget {
      Url(String),
      FilePath { path: String, line: Option<u32> },
  }
  pub fn scan_prose_autolinks(prose: &str) -> Vec<AutolinkHit>;
  ```

### 3A — `security.rs`

- [ ] **Step 1: Failing tests — link + image verdicts.** Create `crates/lens-ui/src/security.rs`:
  ```rust
  const MAX_URL_LEN: usize = 8 * 1024;

  pub enum LinkVerdict {
      AllowOpenUrl,
      NavigateToFile { path: String, line: Option<u32> },
      Strip,
  }

  pub enum ImageVerdict {
      AllowArtifactImg { url: String },
      RenderAsLink { url: String },
      Strip,
  }

  pub fn validate_link_url(url: &str) -> LinkVerdict {
      if url.starts_with("stitch:incomplete-link") {
          return LinkVerdict::Strip;
      }
      if url.len() > MAX_URL_LEN {
          return LinkVerdict::Strip;
      }
      let lower = url.to_ascii_lowercase();
      if lower.starts_with("javascript:")
          || lower.starts_with("data:")
          || lower.starts_with("file:")
          || lower.starts_with("vbscript:")
      {
          return LinkVerdict::Strip;
      }
      if let Some(rest) = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://")) {
          if rest.is_empty() || rest.contains(' ') {
              return LinkVerdict::Strip;
          }
          return LinkVerdict::AllowOpenUrl;
      }
      if looks_like_file_path(url) {
          let (path, line) = split_path_line(url);
          return LinkVerdict::NavigateToFile { path, line };
      }
      LinkVerdict::Strip
  }

  pub fn validate_image_ref(url: &str) -> ImageVerdict {
      if url.contains("..") || url.starts_with('/') {
          return ImageVerdict::Strip;
      }
      let lower = url.to_ascii_lowercase();
      if lower.starts_with("data:") || lower.starts_with("http://") || lower.starts_with("https://") {
          return ImageVerdict::RenderAsLink { url: url.to_string() };
      }
      if lower.starts_with("lens-artifact://") && !lower.contains("..") {
          return ImageVerdict::AllowArtifactImg { url: url.to_string() };
      }
      ImageVerdict::Strip
  }

  fn looks_like_file_path(url: &str) -> bool {
      url.starts_with("./")
          || url.starts_with("../")
          || url.contains('/')
          || url.ends_with(".rs")
          || url.ends_with(".md")
  }

  fn split_path_line(url: &str) -> (String, Option<u32>) {
      if let Some((path, line)) = url.rsplit_once(':') {
          if let Ok(n) = line.parse::<u32>() {
              return (path.to_string(), Some(n));
          }
      }
      (url.to_string(), None)
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn strips_javascript() {
          assert!(matches!(validate_link_url("javascript:alert(1)"), LinkVerdict::Strip));
      }

      #[test]
      fn strips_stitch_incomplete() {
          assert!(matches!(
              validate_link_url("stitch:incomplete-link"),
              LinkVerdict::Strip
          ));
      }

      #[test]
      fn file_path_navigates() {
          match validate_link_url("src/parser.rs:42") {
              LinkVerdict::NavigateToFile { path, line } => {
                  assert_eq!(path, "src/parser.rs");
                  assert_eq!(line, Some(42));
              }
              other => panic!("{other:?}"),
          }
      }

      #[test]
      fn remote_image_renders_as_link() {
          assert!(matches!(
              validate_image_ref("https://tracker.example/x.png"),
              ImageVerdict::RenderAsLink { .. }
          ));
      }
  }
  ```
  Run: `cargo test -p lens-ui security::tests` → PASS.

- [ ] **Step 2: `pub mod security;` in `lib.rs`.** Commit:
  ```bash
  git commit -am "feat(security): validate_link_url + validate_image_ref boundary"
  ```

### 3B — P5 image gate (`node.rs:~609`)

- [ ] **Step 3: Patch `img()` site.** **Before (vendor):**
  ```rust
  child_nodes.push(
      img(image.url.clone())
          .id(ix)
          // ...
  );
  ```
  **After:**
  ```rust
  use crate::security::{validate_image_ref, ImageVerdict};

  match validate_image_ref(&image.url) {
      ImageVerdict::AllowArtifactImg { url } => {
          // §8 deferral: artifact fetch API absent — never call `img()` for a
          // live network fetch. Artifact refs paint as a link-placeholder.
          // When the authenticated artifact API lands, replace this arm with
          // `img(resolved_local_or_authenticated_url)` only.
          child_nodes.push(
              div()
                  .child(format!("[artifact: {url}]"))
                  .into_any_element(),
          );
      }
      ImageVerdict::RenderAsLink { url } => {
          child_nodes.push(
              div()
                  .child(format!("[image: {url}]"))
                  .into_any_element(),
          );
      }
      ImageVerdict::Strip => {}
  }
  // CRITICAL: the unpatched vendor path calls `img(image.url.clone())` here.
  // After this patch there must be ZERO `img(` calls on unvalidated URLs in
  // this function — grep the patched `node.rs` for `img(` to confirm.
  ```

### 3C — P6 link strip (`inline.rs:359`, `node.rs:620`)

- [ ] **Step 4: Route `open_url` clicks through validator.** In `inline.rs` ~359:
  ```rust
  if let Some(link) = Self::link_for_position(&text_layout, &links, event.position) {
      cx.stop_propagation();
      match crate::security::validate_link_url(&link.url) {
          crate::security::LinkVerdict::AllowOpenUrl => cx.open_url(&link.url),
          crate::security::LinkVerdict::NavigateToFile { path, line } => {
              crate::focused::content_events::emit_navigate_to_file(path, line, cx);
          }
          crate::security::LinkVerdict::Strip => {}
      }
  }
  ```
  At link-mark **paint** time (where `TextMark::link` is applied), filter ranges: if `validate_link_url` → `Strip`, omit the link mark (inert text).

- [ ] **Step 5: `content_events.rs` test sink.**
  ```rust
  use std::cell::RefCell;
  thread_local! {
      static SINK: RefCell<Vec<ContentUiEvent>> = RefCell::new(Vec::new());
  }

  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct NavigateToFile {
      pub path: String,
      pub line: Option<u32>,
  }

  pub enum ContentUiEvent {
      NavigateToFile(NavigateToFile),
  }

  pub fn emit_navigate_to_file(path: String, line: Option<u32>, _cx: &mut gpui::App) {
      SINK.with(|s| s.borrow_mut().push(ContentUiEvent::NavigateToFile(NavigateToFile { path, line })));
  }

  #[cfg(test)]
  pub fn take_events() -> Vec<ContentUiEvent> {
      SINK.with(|s| std::mem::take(&mut *s.borrow_mut()))
  }
  ```

### 3D — File-path autolink scanner (§3.6)

- [ ] **Step 6: Failing test — `scan_prose_autolinks`.** In `autolink.rs`:
  ```rust
  pub fn scan_prose_autolinks(prose: &str) -> Vec<AutolinkHit> {
      let mut hits = Vec::new();
      for (idx, token) in prose.split_whitespace().enumerate() {
          let _ = idx;
          if token.starts_with("http://") || token.starts_with("https://") {
              hits.push(AutolinkHit {
                  range: find_token_range(prose, token),
                  target: AutolinkTarget::Url(token.trim_end_matches(&['.', ',', ';'][..]).to_string()),
              });
          } else if looks_like_path_token(token) {
              let clean = token.trim_end_matches(&['.', ',', ';'][..]);
              let (path, line) = split_path_line(clean);
              hits.push(AutolinkHit {
                  range: find_token_range(prose, token),
                  target: AutolinkTarget::FilePath { path, line },
              });
          }
      }
      hits
  }

  fn looks_like_path_token(token: &str) -> bool {
      // path-shaped: has a separator or a known code extension, not a bare word.
      (token.contains('/') || token.ends_with(".rs") || token.ends_with(".md"))
          && !token.contains("://")
  }

  fn split_path_line(token: &str) -> (String, Option<u32>) {
      if let Some((path, line)) = token.rsplit_once(':') {
          if let Ok(n) = line.parse::<u32>() {
              return (path.to_string(), Some(n));
          }
      }
      (token.to_string(), None)
  }

  // Byte range of `token` within `prose`. `split_whitespace` yields subslices of
  // `prose`, so recover the offset by pointer arithmetic (no re-search needed).
  fn find_token_range(prose: &str, token: &str) -> std::ops::Range<usize> {
      let start = token.as_ptr() as usize - prose.as_ptr() as usize;
      start..start + token.len()
  }
  ```
  `AutolinkHit` / `AutolinkTarget` are the structs from this task's Interfaces block — declare them at the top of `autolink.rs`. (`split_path_line` intentionally duplicates the private one in `security.rs` — different module, both tiny.)
  Test: `src/parser.rs:10` in prose → `FilePath`; autolink inside `` `code` `` suppressed at user layer (Task 4).

- [ ] **Step 7: Adversarial fixture test.** `crates/lens-ui/tests/security_adversarial.rs` covers matrix §4: `javascript:`, `data:`, `![](http…)`, embedded HTML → escaped, `stitch:incomplete-link` inert, path autolink emits `NavigateToFile`.

- [ ] **Step 8: Run tests.**
  ```bash
  cargo test -p lens-ui security_adversarial -- --nocapture
  ```
  **Expected:** all PASS.

- [ ] **Step 9: Commit.**
  ```bash
  git add crates/lens-ui/src/security.rs crates/lens-ui/src/focused crates/lens-ui/src/md crates/lens-ui/tests/security_adversarial.rs
  git commit -m "feat(security): paint-time P5/P6 gates + file-path autolink emit"
  ```

- [ ] **Step 10: Cross-family review** (grok-4.5): threat matrix §4 row coverage, paint-time strip vs click-only, `stitch:` sentinel. Re-gate.

---

### Task 4 — T3-3 User messages

§3.5 verbatim + backtick-gated segment pipeline; prose-only autolink through `security.rs`.

**Files:**
- Create: `crates/lens-ui/src/focused/user_content.rs`
- Modify: `crates/lens-ui/src/focused/rowsource.rs` — `UserVerbatim` projection
- Modify: `crates/lens-ui/src/focused/view.rs` — user renderer dispatch
- Test: `crates/lens-ui/src/focused/user_content.rs`

**Interfaces:**
- Consumes: `validate_link_url`, `scan_prose_autolinks`, `MarkdownView` (for ```md fences only).
- Produces:
  ```rust
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum UserSegment {
      Prose(String),
      InlineCode(String),
      Fenced { lang: Option<String>, body: String },
  }
  pub fn split_user_segments(text: &str) -> Vec<UserSegment>;
  pub fn render_user_content(
      content: &RowContent,
      window: &mut Window,
      cx: &mut App,
  ) -> gpui::AnyElement;
  ```

- [ ] **Step 1: Failing test — fence split.** In `user_content.rs`:
  ```rust
  pub fn split_user_segments(text: &str) -> Vec<UserSegment> {
      let mut out = Vec::new();
      let mut i = 0;
      while i < text.len() {
          if let Some(rest) = text[i..].strip_prefix("```") {
              let after_ticks = i + 3;
              let lang_end = text[after_ticks..]
                  .find('\n')
                  .map(|p| after_ticks + p)
                  .unwrap_or(text.len());
              let lang = text.get(after_ticks..lang_end).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
              let body_start = if lang_end < text.len() { lang_end + 1 } else { lang_end };
              let close = text[body_start..].find("\n```").map(|p| body_start + p).unwrap_or(text.len());
              let body = text[body_start..close].to_string();
              out.push(UserSegment::Fenced { lang, body });
              i = if close < text.len() { close + 4 } else { text.len() };
              continue;
          }
          if let Some(rest) = text[i..].strip_prefix('`') {
              let end = text[i + 1..].find('`').map(|p| i + 1 + p).unwrap_or(text.len());
              let code = text[i + 1..end].to_string();
              out.push(UserSegment::InlineCode(code));
              i = if end < text.len() { end + 1 } else { text.len() };
              continue;
          }
          let next = text[i..]
              .find('`')
              .map(|p| i + p)
              .unwrap_or(text.len());
          out.push(UserSegment::Prose(text[i..next].to_string()));
          i = next;
      }
      out
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn splits_inline_code() {
          let segs = split_user_segments("hello `x` world");
          assert_eq!(
              segs,
              vec![
                  UserSegment::Prose("hello ".into()),
                  UserSegment::InlineCode("x".into()),
                  UserSegment::Prose(" world".into()),
              ]
          );
      }

      #[test]
      fn splits_fenced_rust() {
          let segs = split_user_segments("```rust\nfn main() {}\n```");
          assert_eq!(
              segs,
              vec![UserSegment::Fenced {
                  lang: Some("rust".into()),
                  body: "fn main() {}\n".into(),
              }]
          );
      }

      #[test]
      fn splits_fenced_markdown() {
          let segs = split_user_segments("```md\n# H\n```");
          assert_eq!(
              segs,
              vec![UserSegment::Fenced {
                  lang: Some("md".into()),
                  body: "# H\n".into(),
              }]
          );
      }

      #[test]
      fn splits_untagged_fence() {
          let segs = split_user_segments("```\nplain\n```");
          assert_eq!(
              segs,
              vec![UserSegment::Fenced {
                  lang: None,
                  body: "plain\n".into(),
              }]
          );
      }
  }
  ```
  Run: `cargo test -p lens-ui split_user_segments` → PASS.

- [ ] **Step 2: Failing test — autolink in prose vs suppressed in code.**
  ```rust
  #[test]
  fn autolink_prose_not_in_inline_code() {
      use crate::focused::autolink::scan_prose_autolinks;
      let prose = "see src/a.rs:1";
      assert_eq!(scan_prose_autolinks(prose).len(), 1);
      let segs = split_user_segments("`src/a.rs:1`");
      assert!(matches!(&segs[0], UserSegment::InlineCode(_)));
      if let UserSegment::InlineCode(s) = &segs[0] {
          assert!(scan_prose_autolinks(s).is_empty());
      }
  }
  ```
  Run → PASS.

- [ ] **Step 3: Implement `render_user_content`.** In `user_content.rs`:
  ```rust
  use gpui::{div, prelude::*, App, IntoElement, ParentElement, SharedString, Styled, Window, px};
  use crate::focused::autolink::{scan_prose_autolinks, AutolinkTarget};
  use crate::focused::content_events::emit_navigate_to_file;
  use crate::focused::{ContentKey, RowContent};
  use crate::md::MarkdownView;
  use crate::security::{validate_link_url, LinkVerdict};

  pub fn render_user_content(
      content: &RowContent,
      window: &mut Window,
      cx: &mut App,
  ) -> gpui::AnyElement {
      let RowContent::UserVerbatim { text } = content else {
          return div().into_any_element();
      };
      let mut root = div().flex().flex_col().gap_1();
      for (seg_ix, seg) in split_user_segments(text).into_iter().enumerate() {
          match seg {
              UserSegment::Prose(prose) => {
                  root = root.child(render_prose_with_autolinks(&prose, seg_ix, cx));
              }
              UserSegment::InlineCode(code) => {
                  root = root.child(
                      div()
                          .font_family("monospace")
                          .child(code),
                  );
              }
              UserSegment::Fenced { lang, body } => {
                  let lang_l = lang.as_deref().map(|s| s.to_ascii_lowercase());
                  match lang_l.as_deref() {
                      Some("md") | Some("markdown") => {
                          let key = ContentKey::from_label(format!("user-md-{seg_ix}"));
                          root = root.child(
                              MarkdownView::new(key.as_element_id(), body, window, cx)
                                  .scrollable(false)
                                  .selectable(true)
                                  .into_inner(),
                          );
                      }
                      Some(other) => {
                          root = root.child(
                              div()
                                  .font_family("monospace")
                                  .child(format!("[{other}]\n{body}")),
                          );
                      }
                      None => {
                          root = root.child(div().font_family("monospace").child(body));
                      }
                  }
              }
          }
      }
      root.into_any_element()
  }

  fn render_prose_with_autolinks(prose: &str, seg_ix: usize, _cx: &mut App) -> gpui::AnyElement {
      let hits = scan_prose_autolinks(prose);
      if hits.is_empty() {
          return div().whitespace_normal().child(prose.to_string()).into_any_element();
      }
      let mut row = div().flex().flex_row().flex_wrap();
      let mut cursor = 0usize;
      for (hit_ix, hit) in hits.into_iter().enumerate() {
          if hit.range.start > cursor {
              row = row.child(prose[cursor..hit.range.start].to_string());
          }
          let label = prose[hit.range.clone()].to_string();
          let target = hit.target.clone();
          row = row.child(
              div()
                  .id(("ual", seg_ix as u64, hit_ix as u64))
                  .cursor_pointer()
                  .text_decoration_1()
                  .child(label)
                  .on_click(move |_, _, cx| match &target {
                      AutolinkTarget::Url(url) => {
                          if let LinkVerdict::AllowOpenUrl = validate_link_url(url) {
                              cx.open_url(url);
                          }
                      }
                      AutolinkTarget::FilePath { path, line } => {
                          emit_navigate_to_file(path.clone(), *line, cx);
                      }
                  }),
          );
          cursor = hit.range.end;
      }
      if cursor < prose.len() {
          row = row.child(prose[cursor..].to_string());
      }
      row.into_any_element()
  }
  ```
  Run: `cargo test -p lens-ui focused::user_content` still PASS; compile `cargo check -p lens-ui`.

- [ ] **Step 4: Wire `RowKind::UserMessage` in `view.rs`.** In the list render closure, replace stub dispatch:
  ```rust
  match &pres.content {
      RowContent::AssistantMarkdown { .. } => {
          render_assistant_markdown(&pres.content, window, app)
      }
      RowContent::UserVerbatim { .. } => {
          crate::focused::user_content::render_user_content(&pres.content, window, app)
      }
      RowContent::Reasoning { .. } => {
          // Task 5 wires this; keep stub_text until then
          FocusedTranscriptView::render_stub_row(&pres, ix)
      }
      RowContent::Stub { .. } => FocusedTranscriptView::render_stub_row(&pres, ix),
  }
  ```
  Note: the list closure currently only receives `app` — thread `window` from the `list` callback `(ix, window, app)` (gpui `list` already passes `window`).

- [ ] **Step 5: Update `presentation_for_item` for user messages.**
  ```rust
  pub(crate) fn presentation_for_item(item: &Item) -> RowPresentation {
      match &item.kind {
          ItemKind::Message {
              role: Role::User,
              content,
              ..
          } => RowPresentation {
              kind: RowKind::UserMessage,
              content: RowContent::UserVerbatim {
                  text: content
                      .iter()
                      .filter_map(|b| b.text.as_deref())
                      .collect::<Vec<_>>()
                      .join(""),
              },
              collapsed: false,
              height_hint: None,
          },
          ItemKind::Message { content, .. } => {
              let source = content
                  .iter()
                  .filter_map(|b| b.text.as_deref())
                  .collect::<Vec<_>>()
                  .join("");
              RowPresentation {
                  kind: RowKind::Message,
                  content: RowContent::AssistantMarkdown {
                      source,
                      content_key: ContentKey::from_label(item.id.as_str()),
                  },
                  collapsed: false,
                  height_hint: None,
              }
          }
          _ => RowPresentation {
              kind: sibling_row_kind(item),
              content: RowContent::Stub {
                  text: item_text_stub(item),
              },
              collapsed: false,
              height_hint: None,
          },
      }
  }
  ```
  Streaming finalize still overrides `content_key` via Task 1 preserve logic.

- [ ] **Step 6: Run focused suite.**
  ```bash
  cargo test -p lens-ui focused::user_content
  cargo test -p lens-ui
  ```
  **Expected:** green.

- [ ] **Step 7: Commit.**
  ```bash
  git commit -am "feat(focused): user verbatim segment pipeline + prose autolink"
  ```

- [ ] **Step 8: Cross-family review** (grok-4.5): per-fence-form coverage, markdown fence security boundary reuse. Re-gate.

---

### Task 5 — T3-4 Reasoning

§7 lifecycle: live capped (`scrollable(true)`, P3 here) → collapse → summary/full → encrypted; duration from Task 1 plumbing.

**Files:**
- Create: `crates/lens-ui/src/focused/reasoning.rs`
- Modify: `crates/lens-ui/src/md/node.rs:1123` (P3 conditional)
- Modify: `crates/lens-ui/src/focused/view.rs`
- Modify: `crates/lens-ui/src/focused/rowsource.rs` — finalized reasoning projection
- Test: `crates/lens-ui/src/focused/reasoning.rs`

**Interfaces:**
- Consumes: `RowContent::Reasoning`, `MarkdownView`, `ContentKey`, P3 scroll-preserving root render.
- Produces:
  ```rust
  pub enum ReasoningUiState {
      LiveExpanded,
      Collapsed { duration_secs: Option<u32> },
      SummaryExpanded,
      Encrypted { duration_secs: Option<u32> },
  }
  pub fn render_reasoning(
      content: &RowContent,
      ui_state: ReasoningUiState,
      window: &mut Window,
      cx: &mut App,
  ) -> gpui::AnyElement;
  ```

### 5A — P3 scroll-preserving splice (reasoning-only)

- [ ] **Step 1: Failing test — `list_state.reset` skipped when no scroll.** Unit test in `md/node.rs` `#[cfg(test)]`: render root with `list_state: None` → no panic (message rows path).

- [ ] **Step 2: Patch `render_root` (~1123).** **Before:**
  ```rust
  if list_state.item_count() != children.len() {
      list_state.reset(children.len());
  }
  ```
  **After:**
  ```rust
  if list_state.item_count() != children.len() {
      let prev_offset = list_state.offset();
      list_state.reset(children.len());
      list_state.scroll_to(prev_offset);
  }
  ```
  Only used when `MarkdownView::scrollable(true)` — message rows pass `None` list_state (D10).

### 5B — Reasoning renderer

- [ ] **Step 3: Failing tests — four §7 states.** In `reasoning.rs`:
  ```rust
  #[test]
  fn encrypted_label_includes_duration() {
      let label = reasoning_collapsed_label(true, Some(3));
      assert_eq!(label, "🔒 thought for 3s · reasoning hidden");
  }

  #[test]
  fn collapsed_label_without_duration() {
      let label = reasoning_collapsed_label(false, None);
      assert_eq!(label, "💭 thought");
  }

  fn reasoning_collapsed_label(encrypted: bool, duration_secs: Option<u32>) -> String {
      if encrypted {
          format!(
              "🔒 thought for {}s · reasoning hidden",
              duration_secs.map(|s| s.to_string()).unwrap_or_else(|| "?".into())
          )
      } else if let Some(s) = duration_secs {
          format!("💭 thought for {s}s")
      } else {
          "💭 thought".into()
      }
  }
  ```
  Run → FAIL until implemented.

- [ ] **Step 4: Implement `render_reasoning`.** In `reasoning.rs`:
  ```rust
  use gpui::{
      div, prelude::*, px, App, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
      Styled, Window,
  };
  use crate::focused::{ContentKey, RowContent};
  use crate::md::MarkdownView;

  pub enum ReasoningUiState {
      LiveExpanded,
      Collapsed { duration_secs: Option<u32> },
      SummaryExpanded,
      Encrypted { duration_secs: Option<u32> },
  }

  pub fn reasoning_collapsed_label(encrypted: bool, duration_secs: Option<u32>) -> String {
      if encrypted {
          match duration_secs {
              Some(s) => format!("🔒 thought for {s}s · reasoning hidden"),
              None => "🔒 thought · reasoning hidden".into(),
          }
      } else if let Some(s) = duration_secs {
          format!("💭 thought for {s}s")
      } else {
          "💭 thought".into()
      }
  }

  pub fn render_reasoning(
      content: &RowContent,
      ui_state: ReasoningUiState,
      window: &mut Window,
      cx: &mut App,
  ) -> gpui::AnyElement {
      let RowContent::Reasoning {
          summary,
          full,
          encrypted,
          duration_secs,
          content_key,
          live,
      } = content
      else {
          return div().into_any_element();
      };

      if *encrypted {
          return div()
              .child(reasoning_collapsed_label(true, *duration_secs))
              .into_any_element();
      }

      if *live {
          return div()
              .flex()
              .flex_col()
              .gap_1()
              .child(div().child("💭 thinking…"))
              .child(
                  div()
                      .id(("reason-live", content_key.as_element_id()))
                      .max_h(px(120.))
                      .overflow_hidden()
                      .child(
                          MarkdownView::new(content_key.as_element_id(), full.clone(), window, cx)
                              .scrollable(true)
                              .selectable(true)
                              .into_inner(),
                      ),
              )
              .into_any_element();
      }

      match ui_state {
          ReasoningUiState::Collapsed { duration_secs } => div()
              .child(reasoning_collapsed_label(false, duration_secs))
              .into_any_element(),
          ReasoningUiState::SummaryExpanded | ReasoningUiState::LiveExpanded => {
              let body = if summary.is_empty() { full } else { summary };
              div()
                  .flex()
                  .flex_col()
                  .gap_1()
                  .child(div().child(reasoning_collapsed_label(false, *duration_secs)))
                  .child(div().child("show full reasoning ↗"))
                  .child(
                      MarkdownView::new(content_key.as_element_id(), body.clone(), window, cx)
                          .scrollable(false)
                          .selectable(true)
                          .into_inner(),
                  )
                  .into_any_element()
          }
          ReasoningUiState::Encrypted { duration_secs } => div()
              .child(reasoning_collapsed_label(true, duration_secs))
              .into_any_element(),
      }
  }
  ```
  Run: `cargo test -p lens-ui reasoning_collapsed_label` → PASS.

- [ ] **Step 5: Wire reasoning rows in `view.rs`.** Keep a `HashMap<ContentKey, bool>` expand flag on `FocusedTranscriptView` (default collapsed when `!live`). Dispatch:
  ```rust
  RowContent::Reasoning {
      live,
      encrypted,
      duration_secs,
      content_key,
      ..
  } => {
      let ui = if *encrypted {
          ReasoningUiState::Encrypted {
              duration_secs: *duration_secs,
          }
      } else if *live {
          ReasoningUiState::LiveExpanded
      } else if self.reasoning_expanded.get(content_key).copied().unwrap_or(false) {
          ReasoningUiState::SummaryExpanded
      } else {
          ReasoningUiState::Collapsed {
              duration_secs: *duration_secs,
          }
      };
      crate::focused::reasoning::render_reasoning(&pres.content, ui, window, app)
  }
  ```
  Wire the `show full reasoning ↗` click (in a follow-up refinement of `render_reasoning` that takes a toggle callback, or via an `id`-keyed element on the view) to flip `reasoning_expanded` for that `content_key` and `cx.notify()`.

- [ ] **Step 6: Failing test — duration from RowContent.** Build `RowContent::Reasoning` with `duration_secs: Some(4)` → collapsed label contains `4s`.

- [ ] **Step 7: Real-window capped-region probe (optional).** Extend `focused_scroll_probe` or add `focused_reasoning_probe` asserting inner scroll stays pinned to bottom during live stream when `scrollable(true)`.

- [ ] **Step 8: Run suite.**
  ```bash
  cargo test -p lens-ui reasoning::
  cargo test -p lens-ui
  ```
  **Expected:** green.

- [ ] **Step 9: Commit.**
  ```bash
  git commit -am "feat(focused): reasoning lifecycle UI + P3 scroll preserve for capped region"
  ```

- [ ] **Step 10: Cross-family review** (grok-4.5): four states, encrypted path, P3 only on reasoning. Re-gate.

---

## End of workstream

- [ ] **Whole-branch cross-family review** — codex (`codex exec -s read-only`, gpt-5.6) on full `t3-message-reasoning-content` branch. Capture stdout.
- [ ] **Run `cargo run -p xtask -- gate`** — incl. clippy `-D warnings`, real-window probes (`focused_finalize_probe`, `focused_scroll_probe`). Demo screenshots are manual (not in gate).
- [ ] **Update `docs/STATUS.md`** + handoff per end-of-session convention. Record throttle interval chosen, any anchor drift from open item #1.

---

## Self-review notes (spec coverage)

### §5 (streaming) → task steps
| Requirement | Step(s) |
|---|---|
| §5 progressive safe-prefix (D1 mdstitch) | T0 Step 9 `safe_prefix`, T2 Step 1 `StreamCoalescer` + `finalize` same pipeline |
| §5 stable identity / finalize no-op (D11) | T1 Step 11 `content_key` preserved; T2 Step 6 identity test; T2 Step 7 probe |
| §5 coalesce to frame tick | T2 Step 1 `StreamCoalescer` 16 ms budget |
| §5 selection survives streamed update (P2) | T0 Steps 17–18; T2 Step 8 |
| §5 no scroll-jump (D10) | T2 Step 3 `scrollable(false)`; T5 P3 only for reasoning `scrollable(true)` |

### §6 (content) → task steps
| Requirement | Step(s) |
|---|---|
| §6.1 GFM + highlight | T0 vendor; T2 `MarkdownView` wiring |
| §6.1 file-path autolink → `navigate_to_file` | T3 Steps 6–7 `scan_prose_autolinks` + `ContentUiEvent` |
| §6.1 artifact-only inline images (P5) | T3 Steps 3, 7 adversarial |
| §6.1 no raw HTML (P4/D6) | T0 Steps 19–21 |
| §6.2 user verbatim + backtick-gating | T4 Steps 1–5 |
| §6.3 uniform boundary | T3 `security.rs` + P5/P6 + user autolink reuse T4 Step 3 |

### §7 (reasoning) → task steps
| Requirement | Step(s) |
|---|---|
| §7 live capped / auto-scroll | T5 Step 4 `scrollable(true)` capped region |
| §7 collapse / summary→full / encrypted | T5 Steps 3–5 four states |
| §7 duration Ns | T1 Steps 1–4b: `started_at_ms` on acc → `duration_ms` persisted on durable `ItemKind::Reasoning` at finalize; T1 Step 14 projection reads the item field; T5 Step 6 label |

### §6 task table (spec §6) → tasks
| Spec task | Plan task |
|---|---|
| T3-0 Infra + vendor | Task 0 |
| T3-M row plumbing | Task 1 |
| T3-1 Assistant markdown | Task 2 |
| T3-2 Boundary + autolink | Task 3 |
| T3-3 User messages | Task 4 |
| T3-4 Reasoning | Task 5 |

### §7 traceability table (design §7) — all rows mapped above.

### §9 verification → steps
| Verification | Step(s) |
|---|---|
| `xtask gate` + clippy | Every task end; End of workstream |
| T3-0 pre-plan de-risk | Task 0 Step 1 |
| Streaming identity headless + probe | T2 Steps 6–9 |
| Selection height-growing | T2 Step 8 |
| Security adversarial matrix | T3 Step 7 |
| Backtick-gating tests | T4 Steps 1–2 |
| Reasoning four states | T5 Steps 3–6 |

### §10 open items — disposition
| # | Item | Disposition |
|---|---|---|
| 1 | Reconfirm P1–P6 anchors | Task 0 header + Steps 14–20 "reconfirm at edit time" |
| 2 | Throttle interval N | Task 0 Step 16 starts 100 ms; End of workstream tuning note |
| 3 | Autolink scanner reuse vs standalone | Task 3 Step 6 standalone scanner (documented); no parser reuse in v1 |
| 4 | Reasoning duration source | Task 1 **PLUMB** — acc `started_at_ms`; `finalize_reasoning` (Step 4b) writes `duration_ms` onto the durable `ItemKind::Reasoning` (acc is dropped at finalize) |
| 5 | `security.rs` crate location | `lens-ui/src/security.rs` Task 3 |

### Placeholder scan
- Re-scanned after author fixes: no `TBD`, `TODO`, `todo!`, "similar to Task N", `assert!(true)`, or `clock_or_zero_placeholder` remain in step bodies.
- Gate command is `cargo run -p xtask -- gate` (no `cargo xtask` alias).
- `ContentKey::from_label` covers non-stream keys (user ```md fences, snapshot assistant rows).

### Type consistency
- All interfaces (`ContentKey`, `RowContent`, `LinkVerdict`, `ImageVerdict`, `UserSegment`, `AutolinkHit`, `AutolinkTarget`, `ContentUiEvent`, `NavigateToFile`, `StreamCoalescer`, `ReasoningUiState`) defined in a task's Interfaces before use.
- Task 0 `MarkdownView::new` accepts `impl Into<SharedString>` id (no `md`→`focused` cycle). Callers always pass `content_key.as_element_id()`; `markdown_state_entity_id` takes `&str`.
- `ReasoningAcc.started_at_ms` in lens-core consumed by Task 1 materializers and Task 5 labels.
- `accumulate_reasoning(..., clock: &dyn Clock)` signature updated in Task 1 Step 4 and used by `reduce/mod.rs` call sites.

### Gaps closed
- P2 covers **both** `:610` reparse-clear **and** `update_bounds` 253–256 carve-out (Task 0 Steps 17–18).
- P3 explicitly dead for message rows (D10); implemented only in Task 5 for reasoning `scrollable(true)`.
- D9 `validate_image_ref` before every `img()` (Task 3 Step 3) — v1 artifact still placeholder text (§8 deferral), never live fetch.
- Handler for `navigate_to_file` remains out of scope (§1); emit-only via `ContentUiEvent` (Task 3 Step 5).

**Unmapped requirements:** none.
