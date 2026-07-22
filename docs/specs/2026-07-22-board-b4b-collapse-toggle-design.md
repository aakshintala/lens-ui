# Board B-4b — collapse toggle + §7 collapsed-tile — design

**Written:** 2026-07-22 · **Status:** LOCKED — brainstormed + settled (write semantics,
click affordance, `✓N` rule) · **Depends on:** B-4a (`BoardReplica` + serialized `run_op`
write seam, `c189d4c`), B-3 (group chrome `absolute_group`, `board/rollup.rs`),
B-2 (packer + culling container), B-1 (`BoardItemKind::Group { collapsed }`,
`SqliteBoardStore::set_collapsed`, schema v3) · **Feeds:** B-4c (drag/move — inherits the
commit-gated-vs-optimistic decision with B-4b's measured latency in hand), B-6 (archive-side
`✓N` counts light up the suppressed footer).

> **Governing specs.** §7 "Collapsed tile geometry" of
> `docs/specs/2026-07-20-board-packing-and-group-rendering-design.md` (the mockup-validated
> geometry, recorded for B-4) and §8 "Seams & deferred decisions" of
> `docs/specs/2026-07-21-board-b4a-store-replica-write-path-design.md` (the `write(cmd)` seam,
> the idempotent-retry contract, the optimistic-drag-is-B-4c decision). This spec does not
> re-derive them; it builds on them.

---

## 1. Scope

B-4b is the **collapse interaction** — the *first real user write* through B-4a's `run_op`
seam — plus the **§7 collapsed-tile rendering**. Two coupled pieces:

1. A caret click on a group toggles its `collapsed` flag, persisted through the write seam.
2. A collapsed group renders as a 1×1 status-rollup tile (§7) instead of its member cards.

**Non-goals (deferred):** drag/move → B-4c; context-menu grouping → B-4d; archive-side
completed counts → B-6; animated collapse/expand reflow (snap only, §6); the optimistic-apply
write variant → B-4c owns whether collapse adopts it.

---

## 2. Write semantics — commit-gated (decided)

`SetCollapsed` is **commit-gated**: the in-memory `collapsed` flag flips only when the
off-thread persist reply lands, exactly like B-4a's `PlaceSessions`. It reuses B-4a's
`run_op` **mechanism** verbatim — same serialized single-in-flight commit-gated path; it adds
one `Op` enum variant (§2.1) but **no new write machinery**.

- **Why not optimistic here.** Optimistic-apply + rollback-snapshot is machinery the B-4a
  design explicitly earmarked for **B-4c** (drag), where a persistent `Err` leaves the
  in-memory layout diverged from disk and a card must snap back. Collapse has no such
  divergence stakes, and pulling that machinery forward would bloat this slice. `SetCollapsed`
  is idempotent, so it rides B-4a's existing transient-retry (re-enqueue-on-`BUSY`) safely —
  **none of the B-4d non-idempotent commit-phase work is needed.**
- **Sequencing.** B-4c builds the optimistic variant regardless. B-4b's commit-gated toggle
  is measured on-device (§7); that latency reading is handed to B-4c, which then decides —
  *with a number, not speculation* — whether to retrofit collapse to optimistic or leave it
  commit-gated. A single-row idempotent write against off-thread SQLite → reply → notify →
  repaint should land within a frame or two, so commit-gated is a legitimate **shippable**
  state, not a placeholder.

### 2.1 The op

A new variant on `board/replica.rs`'s `Op`:

```rust
Op::SetCollapsed { group_id: BoardItemId, collapsed: bool }
```

- `run_op_inner` handles it by calling the existing `SqliteBoardStore::set_collapsed`
  (already present, B-1) inside the off-thread closure, then returning the committed layout
  (same commit-gated shape as `PlaceSessions` — op returns the reloaded/updated layout, main
  thread swaps it in + notifies).
- `write(Op::SetCollapsed{..})` gates on `is_writable()`. A non-writable replica
  (`Degraded`/`LoadFailed`/`Stale`) **refuses** the write and surfaces the state via the
  banner (B-4a §5) — the user's toggle is *counted as a dropped write, never silently
  dropped*. Recovery `Load` remains always-allowed.
- Idempotent: `set_collapsed` writes an absolute value, not a toggle, so a re-enqueued retry
  after a transient `BUSY` re-writes the same value. No double-effect.

### 2.2 Wiring the caret

Caret-only hit target (decided — keeps the header lane uncommitted for B-4c/B-4d). The
`⌄`/`▸` glyph gets an `on_click` listener:

```rust
.on_click(cx.listener(move |board, _ev, _win, cx| {
    board.replica.update(cx, |r, cx| {
        r.write(Op::SetCollapsed { group_id: gid.clone(), collapsed: !was_collapsed }, cx);
    });
}))
```

`cx.stop_propagation()` on the caret click so it doesn't bubble to any future
header/card gesture ([[gpui-nested-click-stop-propagation]]).

---

## 3. Threading `collapsed` into the render path

The domain already carries the flag; the **UI never reads it today** — `pack_and_render`
unconditionally builds `Item::group(n)` (full footprint) and `board_tree` emits members
regardless of collapse. B-4b makes the UI collapse-aware **without changing the tree walk**:

- **`board_tree` keeps emitting members when collapsed.** The status rollup needs member
  data (status projection), so members must stay in the node. The *rendering*, not the tree,
  branches. `BoardNode::Group` already exposes `item` (carries `collapsed`); no domain change.
- **`GroupMeta`** (in `board/mod.rs`) gains a `collapsed: bool`, read off the group item
  during the `nodes → items` build.
- **`pack_and_render` branches on `collapsed`:**
  - **collapsed →** push a **1×1 `Item`** (new `pack::Item::group_collapsed()`, or a
    `collapsed` flag on `Item::group` that overrides `foot`→`(1,1)`), render the collapsed
    tile (§4), and **exclude the group's member session-ids from `visible`** (§3.1).
  - **expanded →** the unchanged B-3 path (`Item::group(n)` + `absolute_group`).

### 3.1 The visibility fork (load-bearing)

Today every non-culled tile's sessions are pushed into `visible`, which drives
`apply_visibility_gate` → instantiates a `SessionCardView` per id. For a **collapsed** group
the members feed the *rollup* (a data read of their status projection) but must **not** become
card entities.

**Fork:** for a collapsed group, the rollup reads the member projection, but the member ids
are **omitted from `visible`**. Getting this wrong spawns a hidden card view for every
collapsed member — a silent entity leak that defeats the purpose of collapsing. This is
mechanical but must be sabotage-verified (§7): assert a collapsed group's members are absent
from `visible` **and** hold no card view.

---

## 4. Collapsed-tile chrome (§7)

A collapsed group is a **1×1 tile** reusing the group ring / accent / 7%-tint (so it reads as
the same group, distinguished from an expanded one only by showing a rollup instead of member
cards).

- **Header-lane:** `● name · [spend · age] · ▸` — dot in the group accent, name, the B-3
  spend·age rollup (`group_rollup` + `format_group_spend`/`format_age`, reused), and the `▸`
  caret (flipped to "expand"). **No active-count badge** (§7 — redundant with the body rollup).
- **Body (the single body-cell):** the **status rollup** — one row per *non-empty* wave,
  `● N <label>`, dot colored per wave. **Ordering (supersedes §7's stale list):** §7 named only
  `Working · Needs-input · Failed · Ready · Slept`, which predates the `AwaitingReview` /
  `Scheduled` waves ([[wave-states-scheduled-awaitingreview]]) — `Wave` now has 8 variants.
  Order by the **existing wave priority ladder** (`derive_wave`'s resolution order / shell §5.1
  glow priority): NeedsInput · Failed · Working · AwaitingReview · Scheduled · Ready · Slept.
  `Neutral` (no meaningful status) is **excluded** from the rollup. This is self-maintaining —
  new waves inherit their ladder position with no separate list to keep in sync.
- **Footer:** `✓ N done →` archive peek — **rendered iff N > 0** (§5). Pre-B-6 the count is
  structurally 0, so the footer is absent until B-6 wires real archive counts.

Rendered by a collapsed arm of `absolute_group` (or a sibling `absolute_collapsed_group`);
either way, **the caret and header text render from one source** (§6).

### 4.1 Status rollup fold

A new **pure fn** in `board/rollup.rs`, alongside `group_rollup`:

```rust
pub struct StatusRollup { pub rows: Vec<(Wave, u32)> }  // non-empty only, ladder order

pub fn status_rollup(member_waves: &[Wave]) -> StatusRollup
```

- Input is a narrow **`Wave` projection** off each member `SessionCard` (mirrors B-4a's
  `MemberCost` narrow projection — no full-card clone). The UI computes each member's `Wave`
  via the existing `derive_wave(card, now_ms, is_focused=false)` and passes the slice; the fold
  is pure and unit-testable with no gpui/FleetStore dependency.
  - **Note the `now_ms` dependency:** `derive_wave` is time-sensitive (Ready decay, Scheduled
    windows), so the rollup is recomputed each render off the clock — consistent with how
    expanded member cards already derive their wave. No new invalidation beyond §6.2.
- Counts by wave, drops zero-count waves, orders by the priority ladder (NeedsInput · Failed ·
  Working · AwaitingReview · Scheduled · Ready · Slept; `Neutral` excluded). Label + dot color
  per wave resolved at render (reuse the `Wave` color/label SSOT in `card/`).

---

## 5. The unified `✓N` rule

**Render `✓N done` iff N > 0** — applied identically to the expanded-group header
(`absolute_group`) and the collapsed-tile footer. This:

- cleans up B-3's shipped `✓0` on expanded groups (previously rendered unconditionally), and
- makes both chrome forms light up together when B-6 delivers real archive counts.

One rule, both sites — no divergence between collapsed and expanded chrome.

---

## 6. Folded B-3 carried Minor + defaults

**Folds B-3's carried Minor (Opus review).** B-3 left a render-dead `group_header_text` /
inline-header duplication and an integration test that proved *data-wiring* only (correct
under `NoopTextSystem`, [[gpui-test-noop-text-system]]). Since B-4b already touches the header
(caret `on_click`, `✓N` suppression), fold the fix here: **render header/caret from one
source** (retire the dead duplicate) and add a **live rendered-chrome assertion** under the
real `Application` harness.

**Defaults (baked in):**

1. **Snap on collapse/expand — no reflow animation.** Going 1×1 frees grid cells and the
   packer backfills; other tiles jump. The board already reflows without animating (resize,
   culling), so snapping is consistent and avoids expensive absolute-position tweening in gpui
   0.2.2. Animated reflow would be a separate spike.
2. **Status-read invalidation carryforward.** The collapsed tile reads member status, so the
   board depends on every collapsed member → a member's status notify invalidates the whole
   board (the same B-4a I2 O(N)-at-scale note). Correct for rollup freshness, fine at
   collapsed-group scale (few, small); recorded, not solved here.

---

## 7. Testing

Real-window render assertions use the real `Application::new().run()` harness, never
`TestAppContext` ([[gpui-test-noop-text-system]]). Ops are off-thread (`background_spawn`) —
tests drive `run_until_parked` to settle replies.

**Write path**
- `SetCollapsed` round-trips: `write(SetCollapsed{true})` → `run_until_parked` → layout's
  group flag is `true`; **persists across reopen** (new replica on the same store loads it
  collapsed).
- **State gating:** a `Degraded`/`LoadFailed`/`Stale` replica **refuses** the collapse write
  (counted in `dropped_writes`, banner surfaces); a recovery `Load` is still accepted.
- **Idempotent retry:** a transient `BUSY` re-enqueue re-writes the same absolute value (no
  double-effect) — deterministic under the B-4a fault-injection seam.

**Render fork**
- A collapsed group **packs as 1×1** (`foot` override verified via the packing).
- **Visibility fork (sabotage-verified):** a collapsed group's members are **absent from
  `visible`** and hold **no `SessionCardView`**; expanding restores them. Sabotage: force the
  members into `visible` and assert the test fails.
- **Real-window caret toggle:** click the caret → `run_until_parked` → the tile switches
  between member-cards and the 1×1 rollup and **repaints** (live rendered-chrome assertion,
  not just a data flag).

**Rollup fold (pure unit tests, `board/rollup.rs`)**
- `status_rollup` counts by wave, **drops zero-count waves**, orders by the priority ladder
  (NeedsInput · Failed · Working · AwaitingReview · Scheduled · Ready · Slept); `Neutral`
  excluded.
- Empty group → empty rows. All-one-wave → single row. A `Neutral` member contributes no row.

**`✓N` rule (live rendered assertion, both forms)**
- N==0 → footer/badge **absent** on both the collapsed tile and the expanded header.
- N>0 → present on both (drive via a `completed_count` injection seam, since real counts are
  B-6).

**Perf / latency**
- **On-device commit-gated toggle latency** — measure caret-click → repainted-collapsed on
  the real app (feeds B-4c's optimistic decision). Reuse the [[wave-perf-fps-attribution]]
  `measure.sh` rig / the B-4a demo seed.

Full `xtask gate` green (clippy `-D warnings`, fmt `--check`, all crate test suites + benches
building).

---

## 8. Seams & deferred (recorded)

- **B-4c** ← the commit-gated-vs-optimistic decision for collapse, with B-4b's measured
  latency; the optimistic-apply + rollback-snapshot `run_op` variant (drag needs it, collapse
  may adopt it).
- **B-6** ← real archive `completed_count`; lights up the `✓N done →` footer/badge the
  unified rule currently suppresses at 0.
- **Status-read invalidation** (§6.2) — a selectively-updated per-group rollup entity would
  localize the O(N)-at-scale member-notify invalidation; deferred, gated on the on-device FPS
  signal (shared with the B-4a I2 follow-up).

---

## 9. Review

Per project rules: ≥1 cross-family review from a family other than the author's
(**codex gpt-5.6**, `codex exec -s read-only`, per [[codex-as-reviewer]] / [[review-spend-policy]])
of the collapse write + fork logic, plus an **Opus whole-branch** review synthesis before merge.
Subagent-driven execution (composer-2.5 implementers, [[composer-delegation-profile]]).
