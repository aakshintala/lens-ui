# Task 5 (T3-4 Reasoning) — Report

## Status: DONE

## Commit

| SHA | Message |
| --- | --- |
| `ba8e786` | `feat(focused): reasoning lifecycle UI + P3 scroll preserve for capped region` |

Base: `6c9f4c7` (Tasks 0–4).

## Test summary

`cargo test -p lens-ui` — **244 passed** (4 new: 3 `reasoning_collapsed_label` + 1 `render_root_none_list_state_does_not_panic`). `cargo check -p lens-ui -p lens-app` clean. `cargo clippy --workspace --all-targets` — **0 warnings**.

## What shipped

### `focused/reasoning.rs` (new)

- `ReasoningUiState` — four §7 lifecycle states.
- `reasoning_collapsed_label(encrypted, duration_secs)` — durable duration label helper.
- `render_reasoning` — dispatches all four states; `MarkdownView` keyed on `content_key.as_element_id()` (D11).

### `focused/view.rs`

- `reasoning_expanded: HashMap<ContentKey, bool>` on `FocusedTranscriptView` (default collapsed when finalized).
- `render_row` dispatches `RowContent::Reasoning` → `render_reasoning` with UI state derived from `live` / `encrypted` / expand flag.
- Collapsed label + “show full reasoning ↗” wire `on_expand` → flip `reasoning_expanded[content_key] = true` + `cx.notify()`.

### `md/node.rs` — P3 scroll-preserving splice

- In `render_root`, when `list_state` is `Some` and child count changes: capture `logical_scroll_top()`, `reset`, then `scroll_to(prev_offset)`.
- `list_state: None` early-return path unchanged (message rows, `scrollable(false)` — D10).

### `focused/rowsource.rs`

- No code changes — Task 1 already projects `duration_secs: duration_ms.map(|ms| ms/1000)` for finalized rows and `duration_secs: None, live: true` for streaming.

## Four reasoning states — how they render

| State | Trigger | UI |
| --- | --- | --- |
| **LiveExpanded** | `live: true` | Header `💭 thinking…` + capped (`max_h 120px`, `overflow_hidden`) `MarkdownView` with `scrollable(true)` on `full` text |
| **Collapsed** | finalized, `!encrypted`, expand flag false | Single line `💭 thought for {N}s` (or `💭 thought` if no duration); clickable to expand |
| **SummaryExpanded** | finalized, expand flag true | Duration label + `show full reasoning ↗` + `MarkdownView` on `summary` (or `full` if summary empty), `scrollable(false)` |
| **Encrypted** | `encrypted: true` | `🔒 thought for {N}s · reasoning hidden` (no markdown body) |

## Duration label source

- **Live rows:** `duration_secs: None` → label is `💭 thinking…` (never reads acc).
- **Finalized rows:** `duration_secs` projected from durable `ItemKind::Reasoning.duration_ms / 1000` in `rowsource::row_content_for_item` (Task 1 plumbing). Label reads `RowContent::Reasoning.duration_secs` only.
- Acc `started_at_ms` is consumed at finalize and dropped; UI never touches acc.

## P3 scroll-preserve gating

- `MarkdownView::scrollable(true)` → `TextView` passes `Some(list_state)` into `render_root` → P3 splice runs on child-count change.
- `scrollable(false)` (assistant/user/summary reasoning) → `list_state: None` → early `render_block` path; **no** `reset` / scroll preserve (D10).
- Unit test `render_root_none_list_state_does_not_panic` guards the message-row path.

## Concerns

1. **Optional real-window probe skipped** — Step 7 (`focused_reasoning_probe` for capped-region bottom-pin during live stream) not implemented; existing `focused_scroll_probe` unchanged. Manual/controller verification deferred to end-of-workstream gate.
2. **“Show full reasoning ↗”** currently sets expand flag (same as collapsed click); no separate summary→full toggle yet — `SummaryExpanded` already shows summary with fallback to full when empty.
3. **Cross-family review** (grok-4.5 / codex) not run in this session — per brief Step 10 / end-of-workstream.

## Files touched

- `crates/lens-ui/src/focused/reasoning.rs` (new)
- `crates/lens-ui/src/focused/mod.rs`
- `crates/lens-ui/src/focused/view.rs`
- `crates/lens-ui/src/md/node.rs`

## Fix A (I1)

- Tri-state `ReasoningExpand` (`Collapsed` → `Summary` → `Full`) replaces `HashMap<ContentKey, bool>`.
- `ReasoningUiState::FullExpanded` renders `full` via `expanded_body`; `SummaryExpanded` renders summary (fallback to full when empty).
- `on_set_expand(ReasoningExpand, &mut App)` setter; collapsed → Summary, "show full reasoning ↗" → Full, "show summary ↖" → Summary.
- D11: SummaryExpanded and FullExpanded key `MarkdownView` on the same `content_key.as_element_id()` (body swap = reparse, no remount).
- Dropped dead `LiveExpanded` `ReasoningUiState` variant (M1); live path still early-returns in `render_reasoning`.
- Tests: `expand_advances_collapsed_summary_full`, `full_expanded_renders_full_not_summary` (+ existing label tests).

## Fix B (I2)

- P3 splice in `md/node.rs render_root`: when child count changes on the scrollable path, pin to the last item's bottom if the viewer was at/near the bottom before `reset`; otherwise preserve `prev_offset` (existing P3).
- `at_bottom` heuristic: `old_count == 0 || prev_offset.item_ix + 1 >= old_count`; pins via `scroll_to(ListOffset { item_ix: new_count - 1, offset_in_item: px(f32::MAX) })`.
- `focused_reasoning_probe` (`probe` feature, `required-features`): real-window harness drives live `RowContent::Reasoning` through `render_reasoning` → vendored `render_root` list path; streams growing text past 120px cap with `wait_frames(>=12)` between growths; asserts STICK-TO-BOTTOM + PRESERVE-ON-SCROLL-UP (non-sticky, `process::exit`).
- ListState probe accessors on `TextViewState` + `md::markdown_probe_*` helpers (`logical_scroll_top`, `list_item_count`, `scroll_list_to`) behind `#[cfg(any(test, feature="probe"))]`.
- Teeth-check toggle: in `md/node.rs` P3 splice, force `let at_bottom = false;` (disables stick-to-bottom; probe exits 1).
- Non-probe gate: `cargo test -p lens-ui` 246 passed; `cargo check -p lens-ui -p lens-app` clean; clippy 0 warnings (no-probe + `--features probe`); probe builds with `--features probe`, excluded without.
