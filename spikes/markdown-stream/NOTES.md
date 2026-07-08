# markdown-stream spike — running notes

Throwaway harness for the framework §4.1 go/no-go. This file is the durable
record of API discovery + observations; the code is disposable.

## Task 1 — dependency feasibility + gpui-component markdown API (2026-07-07)

### Feasibility: PASS
- `gpui-component = "0.5.1"` resolves and **builds cleanly on this box**: 436
  crates, **~1m04s** cold, ~10–12s incremental. Zero errors.
- It pulls **gpui `0.2.2`** — the *exact* version the framework §3 pin already
  names. The "gpui-component pins its own gpui" caveat (framework §4.1) resolves
  in our favour: no version reconciliation needed for the real build either.
- gpui is **not** re-exported by gpui-component → add `gpui = "0.2.2"` as a
  direct dep (unifies with gpui-component's, one lockfile entry).
- Static markdown render **works**: window opens, renders heading/bold/list/
  fenced-code/table/link. Confirmed on screen.

### The markdown API (from source: `gpui-component-0.5.1/src/text/text_view.rs`)

Public component is **`TextView`** (`gpui_component::text::TextView`), a
`RenderOnce` builder element:

```rust
TextView::markdown(
    id: impl Into<ElementId>,
    markdown: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> TextView
```

Builder methods (all `mut self -> Self`):
- `.selectable(bool)` — text selection (default false)
- `.scrollable(bool)` — internal scroll (default false)
- `.style(TextViewStyle)`
- `.code_block_actions(f)` — per-code-block action hook
- also `TextView::html(...)` (same shape)

Minimal app bootstrap that works:
```rust
Application::new().run(|cx| {
    gpui_component::init(cx);                 // REQUIRED (theme + global state)
    cx.open_window(WindowOptions::default(), |window, cx| {
        let view = cx.new(|_| MdView);
        let any: gpui::AnyView = view.into(); // annotate; .into() is ambiguous inline
        cx.new(|cx| Root::new(any, window, cx)) // gpui_component::Root wrapper REQUIRED
    }).unwrap();
    cx.activate(true);
});
// In MdView::render: div().child(TextView::markdown("md", SAMPLE, window, cx))
```

### Stable-identity mechanism (the core §4.1 question) — strong PASS signal at source level

`TextView::markdown` is architected for streaming with stable identity:

- State is retained via `window.use_keyed_state("{id}/state", …)` → the parsed
  content + selection + scroll live in an `Entity<TextViewState>` **keyed by the
  `ElementId`**. Same `id` across frames ⇒ same retained state ⇒ **no remount**.
- Re-parse runs through an async `UpdateFuture` on a channel, **debounced**, and
  **only fires when the text actually changed** (`current_text != text`,
  `text_view.rs:168`). `ParsedContent` is `PartialEq`. So it does **not**
  re-parse per frame — it re-parses at most once per debounce window after the
  last delta, off the render path.
- **Debounce delay default = 200ms** (`text_view.rs:628`,
  `Duration::from_millis(200)`). Implication: streaming updates will land in
  ~200ms steps, not per-token. Good for perf; may feel chunky. **Open question
  for Task 5/6:** is the delay configurable (want ~1 frame for snappy stream)?
  If not exposed, note as a tuning gap / possible vendoring reason.
- Selection state (`selection_positions`, `is_selecting`) lives in the retained
  `TextViewState` → a selection made above should survive appends below (offsets
  are pixel `Point`s). To be confirmed at runtime (Task 6).

### Consequences for the plan (feed into the regroup)

- **Update path = re-emit `TextView::markdown("md", new_text, …)` every frame
  with a STABLE id.** There is no separate mutate call; the constructor pushes
  `Update::Text` into the retained state's debounced parser. This is the "one
  retained entity keyed by a fixed id" the plan's Task 5 assumed — confirmed.
- `.selectable(true)` + `.scrollable(true)` give the adversarial scenario
  (Task 6) real selection + scroll to stress — no need to build our own.
- The 200ms debounce means our own frame-tick coalescing (replay Task 2) is
  somewhat redundant with the component's built-in debounce; keep replay simple.
- Build-count instrumentation (Task 5 `probe`): the meaningful signal is
  **parse count** (how often `parse_content` runs), not gpui element builds.
  We can't easily instrument inside the dep without vendoring; alternative =
  measure frame time + observe that re-parse is debounced (indirect). Revisit
  probe design at Task 5 given this.
