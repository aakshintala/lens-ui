# Board packing & group rendering — design (B-2 + B-3)

**Written:** 2026-07-20 · **Status:** design locked, container/culling **spike pending** ·
**Supersedes:** the mockup's rigid `auto-fill` grid (was never faithful to §4.3) ·
**Depends on:** B-1 (`BoardLayout` + `SqliteBoardStore`, shipped `8100cc8`) ·
**Pixel SSOT:** `docs/design/renders/board-home.html`

This merges the **B-2** (adaptive packing / scroll / culling — the layout engine) and
**B-3** (group rendering & aggregation) brainstorms into one design, because the
container-primitive and packing-model decisions straddle both slices. Implementation
stays **two plans** (B-2 engine, B-3 group chrome), written **after** the spike.

Behavior anchors: `docs/design/application-shell-and-layout.md` §4.1–4.6.

---

## 1. Scope & non-goals

**In scope (this design):** the count-aware packing algorithm; the gpui board
container + scroll + off-screen culling + the anim-gate-on-scroll fix; group visual
chrome (ring, header-lane, tint); aggregation rollups (spend / age / ✓N-completed);
the focused-mode rail layout.

**Explicitly deferred (with reasons):**
- **Collapse — render *and* toggle → B-4.** The data model (`set_collapsed`) shipped in
  B-1, but the toggle is a board **write** path (UI → `BoardLayout` Entity →
  `BoardStore.save` → notify), which is B-4's foundation. Rendering-only would be dormant
  at runtime until then. Only the **rollup aggregation** happens here (B-3 needs it for the
  badge). The collapsed-tile treatment was mocked and validated (status rollup + done-peek);
  geometry is captured in §7 for B-4.
- **Movement / drag-to-reorder / create-group / context menus → B-4** (§4.5). B-2 owes B-4
  a **slot hit-testing / tile-geometry seam** (the packer already produces `(gx,gy,w,h)` per
  tile — expose it).
- **Multiple boards, rail switcher → B-5.** **Archive-as-board → B-6.**
- **Nested-group rendering beyond depth-1** — the layout pass is written recursively
  (a tile's footprint is a recursive function over `BoardItem`), so depth-N works
  structurally, but only **depth-1 is committed/tested**; deeper is PROVISIONAL until B-5
  makes nested groups reachable at runtime (per [[premature-layer-boundary-binding]]).

---

## 2. The packing model — B (groups-as-tiles) + grid-snap

Three candidate models were compared (`scratchpad/group-layout-options.html`):
A bands, B groups-as-2D-tiles, C border-overlay-on-one-grid. **Chosen: B** — the only
one that faithfully renders the recursive Board→(Card|Group) tree and avoids the
full-width-band whitespace of A.

Within B, packing is **grid-snapped** (not free/pixel-masonry). A direct comparison
(`scratchpad/grid-vs-free-packing.html`) showed free packing buys **≈0 density** here
(tile heights are already card-row-quantized) while costing reflow-jump + ordinal
scramble. Grid-snap dominates for this content shape.

### 2.1 The cell grid

Every board position is a uniform **cell** with three vertical bands:

```
CELL_W = CARD_W + GAP
CELL_H = CARD_H + HEADER + GAP        // [header-lane HEADER][card-body CARD_H][gap GAP]
```

- `CARD_W = 280`, `CARD_H = 160` — the **shipped** compact card (`CARD_WIDTH_PX`/
  `CARD_HEIGHT_PX`, what `mount_cached_card` pins). *Not* the older rich card.
- `HEADER ≈ 24`, `GAP = 16`, `INSET ≈ 5` — new tunables (mockup values; tune on device).

**The header-lane is the load-bearing idea.** Every card — loose or group member — sits in
its cell's **body-zone** (offset `HEADER` from the cell top). This makes loose cards and
group members align on one shared grid. A group fills only its **top** cell's lane with its
header; members occupy full cells below, so the surplus header-allowance of a group's lower
rows becomes the inter-member gap — which **equals** the loose-card row gap. Result: **zero
dead space, no ring bleed into neighbors, one shared vertical rhythm.**

> This was reached after rejecting two wrong turns: (a) centering loose cards in taller cells
> → misalignment; (b) a gap-borne header that bleeds up into the shared row-gap → groups
> overlap neighbors and the top-row group clips. The body-zone + top-lane rule fixes both.

### 2.2 Footprint function

A group of `n` members occupies `foot(n)` cells (cols × rows):

```
foot(n) = (n, 1)                       if n <= 3      // single row, no hole (§4.3 "3 → row")
        = (ceil(sqrt(n)), ceil(n/c))   if n >= 4      // near-square (4→2×2, 6→3×2, 9→3×3)
```

Reproduces every §4.3 anchor (1→centered, 3→row, 4→2×2, 6→3×2). A loose card is `(1,1)`.
`foot` is **recursive** over `BoardItem`: a nested group's footprint is `foot` of its own
children (depth-1 committed; see §1).

### 2.3 Packing algorithm (pure, deterministic)

- `cols = max(1, floor((avail_width + GAP) / CELL_W))`.
- **Grid-snap first-fit** in **ordinal order**: for each item, scan rows top→bottom, cols
  left→right, place at the first free `fc × fr` cell block; mark occupied. This **backfills
  holes** (a later 1×1 drops beside an earlier 2×2) → near-free-packing density while keeping
  reading order.
- `content_height = maxRow * CELL_H - GAP`.
- **Residual gaps** to the right of wide tiles that don't tessellate into `cols` are
  **accepted** — the only alternative is reordering out of ordinal sequence, which we reject.
  They reflow as width changes.

### 2.4 Tile placement (render geometry)

For a tile at grid `(gx, gy)`, `X = gx*CELL_W`, `Y = gy*CELL_H`:

- **Loose card:** `left = X`, `top = Y + HEADER`, size `CARD_W × CARD_H` (body-zone; the
  header-lane above is left empty — the consistent row rhythm).
- **Group:** ring box at `left = X-INSET`, `top = Y-INSET`,
  `w = fc*CELL_W - GAP + 2*INSET`, `h = fr*CELL_H - GAP + 2*INSET` (ring lives in the
  inter-tile gap so members stay full-size). Header in the top lane. Member `(cc, rr)` at
  `left = INSET + cc*CELL_W`, `top = INSET + HEADER + rr*CELL_H`, size `CARD_W × CARD_H`.

Reference implementation of all of the above: `docs/design/renders/board-home.html`
(`pack()` + `render()`), which is the pixel SSOT.

---

## 3. Group chrome (B-3)

- **Ring + tint** — 1.5px border in the group's `color_token`, faint color-matched body
  wash, soft outer glow + inner vignette (see `.grp` in the SSOT). Lives in the gap; does
  not shrink members.
- **Header-lane** (`HEADER` tall, top cell): `● dot · name · [spend · age] · ✓N · ⌄`.
  - **`✓N` badge = completed→Archive count** (the §4.6 "Completed (N)" peek), **not** active
    card count. Rationale: active count is redundant (members are visible when expanded,
    summed by the rollup when collapsed); the completed count is otherwise homeless. Deep-links
    to Archive filtered to the group.
  - `⌄` caret is the collapse affordance; **B-4** wires the toggle.
- **Members** render the identical compact card chrome as loose cards — same 280×160,
  fully readable (this was a hard requirement; it drove the ring-in-gap decision).

### 3.1 Aggregation (rollups)

Pure fold over the group's member `SessionCard`s (from the existing FleetStore / §9
`SummaryUpdate` feed via a `group_of(&SessionCard)` seam):

- **spend** = Σ `cumulative_cost` over members.
- **age** = derived from oldest member `created_at` (or group `created_at`).
- **✓N completed** = count of the group's archived/completed sessions (Archive-side).
- **status rollup** (count by wave/status) — computed here; **consumed by the collapsed tile
  in B-4** (see §7). Not rendered on expanded groups.

All derived at render; nothing new persisted (collapse flag already persisted by B-1).

---

## 4. Container, scroll & culling — **SPIKE RESOLVED: GO** (2026-07-20)

`list()` / `uniform_list` are 1-D (uniform- or variable-height rows); neither does 2-D
masonry. So the container is a **custom scrollable surface with absolutely-positioned
tiles**, content-height from the packer. This was B-2's real implementation risk; the spike
(`spikes/board-container/`, real-window gpui program, harness=false per
[[gpui-test-noop-text-system]]) resolved all four unknowns **GO**. Verdict below; full
detail in `spikes/board-container/NOTES.md` and memory [[board-container-spike]].

1. **Scroll surface — GO.** Absolute-positioned tiles inside a **stateful**
   `div().id(..).overflow_scroll().track_scroll(&handle)`, wrapping **one in-flow child of
   explicit `content_height`** (from the packer). That explicit-height child establishes the
   scroll extent even though every tile is `absolute` (out of flow). `ScrollHandle::offset()`
   reads scroll each frame; **`offset.y ≤ 0` scrolled down → `scroll_top = -offset.y`**.
   (Gotchas: div must be stateful or `overflow_scroll`/`track_scroll` aren't in scope;
   `Pixels.0` is private → `f32::from(px)`.)
2. **Render culling — GO.** Build only tiles whose `y`-range intersects
   `[scroll_top - overdraw, scroll_top + viewport_h + overdraw]`. Culled tiles are simply
   **absent from the child vec → gpui never builds them** (proven: at top, 9/56 tiles built,
   off-screen cards' `render_count == 0`). gpui does not force-build a div's children.
3. **Timer gating on scroll — GO; retires the old gate, fixes the freeze at the root.** The
   container is the **sole visibility authority**. Cards init **hidden** (no timer). Each
   frame the container computes the visible set from packer geometry (pure — no entity reads)
   and applies `card.set_visible(bool)` **via `App::defer`**, OFF its own render path, so it
   never touches sibling card entities inside `render`'s accessed-entity window (the
   `.cached()` dirty-tracking landmine, [[viewport-reentry-freeze]]). `set_visible(true)`
   spawns the anim timer; `(false)` drops it. Probe proof: card scrolled off → timer stops,
   ticks freeze; scrolled back → **timer respawns, ticks resume** — exactly the scroll case
   the old edge-triggered `recover_viewport_gates_on_reentry` (focus↔board only) could not
   handle. This **retires** both the paint-time `last_bounds` gate (`card/view.rs`) and the
   edge-trigger (`board/mod.rs`). *Init subtlety:* cards MUST start hidden — if they start
   visible, the first `set_visible(true)` is a no-op and the timer never spawns.
4. **Off-screen CPU — GO, culling ~halves idle CPU.** Release, 56-tile fixture, 3 cols,
   ~9 visible, idle: **cull-ON ≈ 6.8%** vs **all-timers ≈ 15.3%** CPU → **~55% saved**; the
   delta scales with off-screen count. Rig: `spikes/board-container/measure.sh`.

**Fallback (unused):** a hand-rolled `uniform_list` over *rows* — rejected need; absolute
masonry works. The pure packer (`spikes/board-container/src/packer.rs`, unit-tested against
the §2.2 anchors) is ready to lift into `lens-core` for the B-2 plan.

---

## 5. Focused-mode rail

The rail (`.boards`, **286px**, existing `focused-session.html`) is the **same layout logic at
1 column**: a group clamped to 1 column becomes a **1×N vertical stack** of its members inside
its ring/header; loose cards are 1-wide; all in ordinal order. At 1 column grid-snap degenerates
to a simple vertical flex stack — no 2-D packing needed.

- Rail cards use a **compact card variant** (~244px wide, auto-height) — a full 280×160 does not
  fit the 286px strip, and compact is right for a peripheral nav strip.
- The current session is outlined (`.fc.cur`).
- The collapsed-rollup tile is **not** rail-forced (earlier premise retracted); it is a board
  feature only.

`focused-session.html` already reflects this model; refresh it to the latest compact chrome only
if it drifts.

---

## 6. Seams (referenced, not folded in)

- **B-4** ← slot hit-testing / tile-geometry (`(gx,gy,w,h)` per tile); board **write** path
  (`BoardLayout` Entity + `BoardStore.save` + notify); collapse toggle; drag/reorder/group.
- **B-5** ← nested-group runtime creation (unblocks depth-N rendering); multiple boards.
- **B-6** ← Archive-as-board (consumes the ✓N deep-link; renders archived groups via the same UI).
- **B-1 open read-API** — `board_tree(board_id)` ordered walk is **not** exposed yet (B-1 has only
  `children(board_id, parent)`); B-2 adds it (the packer needs the walk).
- **`group_of(&SessionCard)`** — the coarse `SummaryUpdate` feed already exists; the group
  membership lookup is Lens-owned (not `card.workspace`).

---

## 7. Collapsed tile geometry (captured for B-4)

Validated in the mockup; recorded so B-4 rebuilds it without re-deriving:

- A collapsed group is a **1×1 tile** (`foot` overridden to `(1,1)` when `collapsed`).
- Header-lane: `● name · [spend · age] · ▸` (caret flips to "expand"). **No active-count badge**
  (redundant with the rollup below).
- Body (the single body-cell): **status rollup** — one row per non-empty status
  `● N <label>` in order `Working · Needs-input · Failed · Ready · Slept`, colored dot per
  status — plus a footer `✓ N done →` archive peek (border-top separated).
- Reuses the group ring/tint; distinguishes from an expanded group (which shows member cards).

---

## 8. Open questions for implementation (not blocking design)

- Exact `HEADER` / `INSET` / `GAP` px — tune on device (mockup uses 24 / 5 / 16).
- Overdraw margin for the visible-range cull — **resolved: `1 × CELL_H` (200px)**. Covers the
  one-frame offset lag (cull uses last frame's painted offset) with zero pop-in in the spike's
  fast programmatic scroll jumps; one row suffices.
- Whether the board write-path (for B-4) is stubbed minimally in B-2 or left entirely to B-4 —
  B-2 is read-mostly (renders the tree); recommend leaving writes to B-4.
