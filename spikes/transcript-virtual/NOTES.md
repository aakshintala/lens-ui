# transcript-virtual spike — Phase 0 notes

Throwaway harness for framework §4.1c/d go/no-go (variable-height virtualization).
This file is the durable record; the code is disposable.

## Phase 0 — scaffold + §5 introspection (2026-07-07)

### Build

- Crate: `transcript-virtual` (`spikes/transcript-virtual/`)
- Deps: `gpui = "0.2.2"`, `gpui-component = "0.5.1"` (same pins as `markdown-stream`)
- `cargo build -p transcript-virtual` — clean

### §5 — logical scroll anchor introspection

Probe **1b** needs programmatic read of `(top-item-index, sub-offset)`. Raw API
facts from the cargo registry sources (`gpui-0.2.2`, `gpui-component-0.5.1`).

---

#### Candidate A — gpui native `list()` + `ListState`

**Verdict: logical anchor IS readable.**

`list()` signature:

```rust
pub fn list(
    state: ListState,
    render_item: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static,
) -> List
```

Render closure is `FnMut(usize, &mut Window, &mut App) -> AnyElement` — index
is a plain `usize`, not `AnyElement` directly but equivalent to spec's question.

`ListAlignment::Bottom` **exists**:

```rust
pub enum ListAlignment {
    Top,
    Bottom,  // "like a chat log"
}
```

Constructed via `ListState::new(item_count, alignment, overdraw)`.

**Stick-to-bottom:** when `alignment == ListAlignment::Bottom` and scroll position
reaches the maximum, internal `logical_scroll_top` is set to `None` (meaning
"pinned to bottom"). When read via the public getter, `None` maps to:

```rust
ListAlignment::Bottom => ListOffset {
    item_ix: self.items.summary().count,
    offset_in_item: px(0.),
}
```

**Logical anchor getter** — public, first-class:

```rust
/// Get the current scroll offset, in terms of the list's items.
pub fn logical_scroll_top(&self) -> ListOffset

#[derive(Debug, Clone, Copy, Default)]
pub struct ListOffset {
    pub item_ix: usize,
    pub offset_in_item: Pixels,
}
```

Also available: `scroll_to(ListOffset)`, `scroll_by(Pixels)`,
`scroll_to_reveal_item(ix)`, `bounds_for_item(ix)`, `viewport_bounds()`.

**Pixel offset** (secondary, for scrollbar): `scroll_px_offset_for_scrollbar() -> Point<Pixels>`.

**Scroll handler** exposes visible range but NOT sub-offset:

```rust
pub struct ListScrollEvent {
    pub visible_range: Range<usize>,
    pub count: usize,
    pub is_scrolled: bool,
}
```

For 1b, use `logical_scroll_top()` directly — no eyeball degradation on Backend A.

---

#### Candidate B — gpui-component virtualized `List` + `VirtualListScrollHandle`

**Verdict: no public logical-anchor getter — pixel offset only (1b degrades to
eyeball or manual derivation).**

gpui-component's high-level `List` (`list/list.rs`) wraps `v_virtual_list` with a
`VirtualListScrollHandle`. Public scroll surface on `ListState`:

```rust
pub fn scroll_handle(&self) -> &VirtualListScrollHandle
pub fn scroll_to_item(&mut self, ix: IndexPath, strategy: ScrollStrategy, ...)
```

`VirtualListScrollHandle` (`virtual_list.rs`):

```rust
pub fn base_handle(&self) -> &ScrollHandle
pub fn scroll_to_item(&self, ix: usize, strategy: ScrollStrategy)
pub fn scroll_to_bottom(&self)
// Deref -> ScrollHandle
```

`ScrollHandle` (gpui `div.rs`) exposes **pixel** state:

```rust
pub fn offset(&self) -> Point<Pixels>
pub fn set_offset(&self, position: Point<Pixels>)
pub fn max_offset(&self) -> Size<Pixels>
pub fn top_item(&self) -> usize      // child index in painted children, NOT logical item ix
pub fn bottom_item(&self) -> usize
pub fn bounds_for_item(&self, ix: usize) -> Option<Bounds<Pixels>>
```

`VirtualList` computes `first_visible_element_ix` internally during `prepaint`
(by walking cached `item_sizes`), but this is **not exposed** on any public handle.

`v_virtual_list` render closure:

```rust
Fn(&mut V, Range<usize>, &mut Window, &mut Context<V>) -> Vec<R>
```

Visible range is passed to the renderer; the scroll handle does not echo it back.

**Stick-to-bottom:** no `ListAlignment::Bottom` equivalent. Closest API:
`scroll_handle.scroll_to_bottom()` which calls
`scroll_to_item(items_count - 1, ScrollStrategy::Top)` — scroll-to-last-item, not
a persistent bottom-alignment mode like gpui native `list()`.

**Possible 1b workaround (not built in Phase 0):** derive `(top_ix, sub_offset)`
from `scroll_handle.offset().y` + the harness-owned `Rc<Vec<Size<Pixels>>>` item
size cache (same data `v_virtual_list` already takes). That is probe logic, not
framework API — record as PARTIAL if we go that route.

---

### Surprises

1. **Backend A is better instrumented than expected** — `logical_scroll_top()` is
   exactly the `(top-item, sub-offset)` pair the spec names; not pixel-only.
2. **Backend B's `ScrollHandle::top_item()` is misleading for virtualization** —
   it indexes painted child bounds (visible slice), not logical row index.
3. **gpui-component `List` doc comment still says "all items has the same height"**
   even though it sits on variable-height `v_virtual_list` — stale comment, but
   the implementation does support mixed heights via `RowsCache` + per-row sizes.
4. **Stick-to-bottom semantics differ materially** — native `list()` has a
   first-class `ListAlignment::Bottom` with `logical_scroll_top == None` meaning
   "at bottom"; gpui-component only has imperative `scroll_to_bottom()`.

---

## Phase 1 — Backend A + probes (2026-07-07)

### Build / run

```bash
cargo build -p transcript-virtual
cargo run -p transcript-virtual          # default N=200
cargo run -p transcript-virtual -- --n 2000
```

### Keybindings (focus the window first)

| Key | Probe |
|-----|-------|
| `shift-2` | Reload N=200 |
| `shift-3` | Reload N=2000 |
| `1` | Windowing (press at each N; second press compares frame-cost ratio) |
| `2` | Variable heights |
| `3` | Anchor 1a — append to last row (must be pinned to bottom) |
| `4` | Anchor 1b setup — scroll to mid, record anchor |
| `5` | Anchor 1b mutate — bump off-screen-above row height |
| `6` | Identity — reveal markdown row, record, scroll off |
| `shift-6` | Identity — scroll back, assert Entity retained |
| `7` | UX — append while paused |
| `8` | UX — evaluate follow transitions |
| scroll wheel | UX — scroll up to pause, scroll to bottom to resume |

### Identity / selection note

`TextView` selection is mouse-driven only (no public `set_selection` API).
Identity probe asserts **Entity id** + **markdown_init_count** (TextView
keyed-state not re-created on re-scroll), not selection survival. Manual
selection eyeball: drag in a CodeBlock row after shift-6.

### API adaptations vs spec sketch

- `ListState::new(count, ListAlignment::Bottom, overdraw)` — no render fn in
  `new`; render closure is the second arg to `list()`.
- Height changes require `list_state.splice(ix..ix+1, 1)` to invalidate cached
  heights (gpui list contract).
- Probe counters sample **one frame after keypress** (list closure runs post-
  `render()` return).
