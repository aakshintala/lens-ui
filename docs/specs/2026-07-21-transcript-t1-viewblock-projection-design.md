# T-1 — ViewBlock projection pipeline (design)

**Date:** 2026-07-21
**Status:** Design — ready for cross-family review, then implementation plan (new session).
**Owner:** Lens design effort
**Type:** Implementation slice (build), first of the transcript workstream T-1..T-7.

Implements the render pipeline foundation of `docs/design/conversation-transcript.md`
§3 (pure view-projection) + the structural half of §4 (turn lifecycle). This
document is an **implementation decomposition** of an already-complete product
design — it does not re-open product questions; it resolves the gpui/lens-core
specifics and records the deviations from §3's provisional type shapes.

Sibling slices (STATUS "transcript fan-out"): T-2 scaffold+virtualized surface ·
T-3 content/markdown · T-4 tool spans + resource markers · T-5 sub-agent spans
(child-session) · T-6 turn lifecycle · T-7 composer & live turn.

---

## 1. Scope & boundaries

**T-1 owns** one thing: the **pure projection** of a single session's canonical
`&[Item]` (+ RAM-only `StreamScratch`) into `Vec<ViewBlock>` — deterministic,
framework-neutral, no gpui, exhaustively unit-tested. It is the spine every
render slice (T-2..T-7) and the History view read from.

**T-1 does NOT own** (each → its slice):

| Concern | Why not T-1 | Slice |
|---|---|---|
| Any rendering / gpui | T-1 is pure data | T-2+ |
| `OptimisticUser` in the stream | Pending is composer-owned, not a projection input (§6.4) | T-7 |
| `ReconnectBreak` emission | No backing item/scratch datum; needs reconnect timing | T-2 |
| `SubAgentSpan` / `ChildRef` / real `flatten_sub_agents` | Child-**session** model, not `ctx.depth` (§6.3) | T-5 |
| Model / token / cost in `WorkSectionMeta` | Session-level cumulative only; no per-turn data exists (§5.4) | T-6 |
| `WorkSection` expand/collapse state | Pure UI state (§5.3) | T-6 |

---

## 2. Home & module layout

Lives in **lens-core**, a new module beside the existing render transforms
(`crates/lens-core/src/reduce/transforms.rs`). Rationale: `ViewBlock` borrows
`&Item` (a lens-core type), the projection is pure/framework-neutral, and the
design wants it "reused across surfaces by composing transform sets" — that is
lens-core's job, not lens-ui's. lens-ui depends on lens-core and consumes
`Vec<ViewBlock>`.

Proposed: `crates/lens-core/src/reduce/view.rs` (the `ViewBlock` enum +
composite transforms + the staged assembler), keeping the existing
`transforms.rs` filters where they are. Exact filename is a plan detail.

---

## 3. The `ViewBlock` enum

```rust
pub enum ViewBlock<'a> {
    Item(&'a Item),                                          // passthrough
    ToolSpan { call: &'a Item, output: Option<&'a Item> },  // paired by call_id
    WorkSection { blocks: Vec<ViewBlock<'a>>, meta: WorkSectionMeta },
    StreamingMessage(&'a str),                              // scratch.open_message.text
    StreamingReasoning(&'a str),                            // scratch.open_reasoning
}

pub struct WorkSectionMeta {
    pub duration_ms: i64,                       // last.created_at − first.created_at in the turn
    pub agent_transitions: Vec<(AgentId, AgentId)>,  // from AgentChanged items in the turn
    pub tool_count: usize,
}
```

The projection **borrows**; nothing is cloned. `'a` spans both the `&[Item]`
slice and the `&StreamScratch` (both live for the render frame). This is why
`merge_text_for_display` (the one existing transform that returns **owned**
`Vec<Item>`) is not part of the T-1 pipeline: if adjacent-message coalescing is
wanted later, the caller binds its owned result to a `let` that outlives the
`ViewBlock`s and projects from that — the projection itself stays borrow-only.

### 3.1 Deviations from `conversation-transcript.md` §3 (recorded)

§3 listed a richer enum; the following are deliberate corrections, each applying
the doc's own stated principle that `ViewBlock` "stays thin — mostly
`Item(&Item)` passthroughs plus the handful of composites." I will annotate §3
when this lands.

1. **`WorkSection { open: bool, .. }` → `{ blocks, meta }`** — `open` is pure UI
   state; a pure projection carrying it would either recompute the default every
   frame (clobbering user toggles) or take toggle-state as input (no longer pure
   over items). Dropped; render (T-6) owns expansion.
2. **`CompactionMarker` / `AgentChangedMarker` dropped as variants** — both are
   1:1 item-backed, so they are `Item(&Item)` passthroughs; render extracts
   display fields (`summary`/`tokens`, `from`/`to`) by matching `ItemKind`. Same
   structural-vs-render principle as (1). (`AgentChanged` *transitions* still
   feed `WorkSectionMeta.agent_transitions` for the chip.)
3. **`OptimisticUser` removed entirely** — see §6.4.
4. **`SubAgentSpan { child: ChildRef }` removed** — see §6.3.
5. **`ReconnectBreak` deferred to T-2** — zero-field marker with no backing
   item; the slice that wires reconnect adds the variant when it has the timing.

Exhaustiveness is unaffected: the **projection matches `ItemKind` exhaustively**
(no wildcard arm), so a server-added `ItemKind` is a compile error — even though
the `ViewBlock` enum stays small. Adding a variant later (T-2/T-5) is a local,
compiler-checked change.

---

## 4. The staged pipeline (approach A)

The transforms have three genuinely different signatures, so the pipeline is
**staged**, not a uniform `pipe`. Each surface (Chat column, History view,
future) composes which filters/groupers it runs; the stage boundaries are fixed.

```
Stage 1 — filters          [Item]            → [&Item]
    hide_reasoning, with_agent_changed_markers, only_agent, …   (existing, transforms.rs)

Stage 2 — project          [&Item] + &StreamScratch → [ViewBlock]
    pair_tool_spans  +  streaming-tail splice                   (new)

Stage 3 — groupers         [ViewBlock]       → [ViewBlock]
    group_work_section                                          (new)
```

Signatures:

```rust
// Stage 2 core — consumes the (optionally filtered) item view:
pub fn project<'a>(items: &[&'a Item], scratch: &'a StreamScratch) -> Vec<ViewBlock<'a>>

// Convenience assembler for the common no-filter path:
pub fn project_all<'a>(items: &'a [Item], scratch: &'a StreamScratch) -> Vec<ViewBlock<'a>>
//   = group_work_section(project(&items.iter().collect::<Vec<_>>(), scratch))
```

Stage 1 filters yield `Vec<&Item>`; Stage 2 (`project`) consumes that `&[&Item]`
view (so a filtering surface — e.g. History view running `hide_reasoning` — and
the no-filter Chat column share one projector). `project_all` is the sugar for
the unfiltered case. The contract that matters either way: **no `pending` input;
pure and borrow-only over `(items, scratch)`.** Exact assembler ergonomics
(filter-set param vs. pre-applied) are a plan detail; the signatures above are
the fixed contract.

---

## 5. Transforms

### 5.1 `pair_tool_spans` (Stage 2)

Pairs `ItemKind::FunctionCall` with its `FunctionCallOutput` by `call_id`,
emitting `ToolSpan { call, output }`.

- Output present → `ToolSpan { call, output: Some(out) }`.
- In-progress / not-yet-returned call (no matching output) →
  `ToolSpan { call, output: None }`. (`is_terminal()` already encodes "everything
  terminal except an in-progress `FunctionCall`".)
- **Orphan `FunctionCallOutput`** (no matching call in the window — possible at a
  disk-window boundary) → `Item(&Item)` passthrough; never dropped.
- Pairing is by `call_id` over the item slice directly. `scratch.unpaired_calls`
  is reduce-local bookkeeping and is **not** consulted here — the projection
  pairs from the durable items it is given.

### 5.2 Streaming-tail splice (Stage 2)

After the finalized-item blocks, append the live turn's in-flight blocks from
`scratch`, in turn order:

- `scratch.open_reasoning` present → `StreamingReasoning(&str)` (streams the
  accumulating reasoning text).
- `scratch.open_message` present → `StreamingMessage(&scratch.open_message.text)`.

Reasoning precedes message (a turn reasons, then emits text). When `scratch` has
neither open accumulator, there is no streaming tail (steady/settled state, incl.
disk-only paint).

### 5.3 `group_work_section` (Stage 3) + liveness

Wraps each **settled** turn's *work* blocks into one `WorkSection`; the **live**
turn stays flat.

**Liveness (verified against the reducer):** every item is stamped
`ctx.turn = scratch.turn` at reduce time (`reduce/items.rs:17`), and
`ResponseEvent::Completed` does `scratch.turn += 1` (`reduce/mod.rs:132–136`,
tests `completed_bumps_turn_*`). Therefore:

- A turn `T` is **settled** iff `T < scratch.turn`.
- The turn `T == scratch.turn` is **live** → its blocks stay flat (no section).
- **Disk-paint guard:** a settled conversation loaded from disk carries the
  actor's *live* scratch only when a turn is actually in flight (the D23 shape:
  live tail from actor scratch, finalized from disk). Disk-only paint therefore
  has no live scratch → nothing reads as live → all turns settle. Within a live
  actor session the rule is correct even for turn 0. No special-casing needed.

**What goes inside a settled turn's `WorkSection`:** its `Reasoning` blocks,
`ToolSpan`s, `NativeTool` blocks, and inline `AgentChanged` passthroughs. **What
stays a sibling (outside the section):** the turn's user/assistant `Message`
blocks (§4 "the final assistant text stays visible"), and turn-boundary markers
(`Compaction`, `Error`, `SlashCommand`, `TerminalCommand`). A turn with no work
blocks yields no `WorkSection` (just its messages); a pure-tool turn with no
final message yields a `WorkSection` with no trailing message (§4 "the chip alone
represents the turn"). Grouping keys off each block's `ctx.turn` (read through
`Item(&Item)`/`ToolSpan.call`).

### 5.4 `WorkSectionMeta` — the split

Computed in the projection, from **item-derivable data only**:

- `duration_ms` = last − first `created_at` within the turn (epoch millis;
  single-item turn → ~0).
- `agent_transitions` = ordered `(from, to)` from the turn's `AgentChanged`
  items.
- `tool_count` = number of `ToolSpan`s in the turn.

**Not in T-1 (data gap, verified):** the chip's model / token / cost fields.
Usage is **session-level cumulative** only — `session.usage` →
`SessionState.cumulative_cost` / `last_total_tokens`, `session.model` →
`SessionState.llm_model` (current, no per-turn history); items carry no usage.
Per-turn model/token/cost simply do not exist in the data today. Resolving the
chip's full line is a **T-6** concern and likely needs a state-model
prerequisite (snapshot cumulative deltas at each `response.completed`), or those
fields drop from the chip. Out of T-1.

### 5.5 Existing transforms reused as-is (Stage 1)

`hide_reasoning`, `with_agent_changed_markers`, `only_agent`, `by_depth` stay in
`transforms.rs` untouched. `merge_text_for_display` is **not** wired into the
T-1 pipeline (owned-return; see §3). `flatten_sub_agents` stays the current
identity stub — its real body is T-5 (§6.3).

---

## 6. Key resolutions (why the scope is what it is)

### 6.1 Pipeline shape = staged (A), not uniform

The three signatures (`[Item]→[&Item]`, `[&Item]→[ViewBlock]`,
`[ViewBlock]→[ViewBlock]`) cannot uniformly `pipe`. Staging is honest about the
select→project→group phases, keeps the shipped `[Item]→[&Item]` transforms
unchanged, and still supports per-surface reuse by varying which filters/groupers
run.

### 6.2 Liveness from `scratch.turn`, not sniffing accumulators

Keys off the real `response.completed` bump (§5.3) rather than "is an accumulator
open" — more robust, still pure over `(items, scratch)`.

### 6.3 Sub-agents are child *sessions*, not `ctx.depth`

The wire model is a `session.child_session.*` family:
`SessionEvent::Created { child_session_id, agent_id, parent_session_id }` (spawn)
and `ChildSessionUpdated { child_session_id, child: ChildSession }` (live status:
title, tool, busy, `current_task_status`) — `crates/lens-client/src/stream/event.rs`.
A sub-agent is a **separate session** with its own item stream, linked by
`parent_session_id` — **not** rows with `ctx.depth > 0`. So §3's
`flatten_sub_agents (depth-1)` and `SubAgentSpan { child: ChildRef }` are modeled
on data that does not exist, and D-P1-14 ("depth deferred") is largely moot —
depth-on-items may never be the model.

Consequence: the sub-agent in-transcript span (§8.6) is its own slice **T-5** —
reducer folding of `child_session.created/updated` into a parent↔child registry +
live status, projection of a span at the spawn point, §8.6 render, and
navigate-into-child (which shares the shell's session-focus machinery — the one
cross-surface seam). It is **in this workstream**, not deferred out; it is simply
not foundation work and too large to bury in T-4. T-1 omits its types.

### 6.4 Pending is composer-owned, out of the projection

An optimistic user message inline at the tail conflicts with §16 auto-follow: the
stick-to-bottom anchor would hold the *pending bubble* while the agent streams,
instead of the response, and there is no tail position that satisfies both
turn-order and follow-the-response. A composer-adjacent stack dissolves the
conflict (the transcript tail is always the live response; queued/failed sends
live next to the composer, entering the stream when they land as canonical
Items). **Only naive in-stream optimism requires an `OptimisticUser` `ViewBlock`,
and that is the broken option** — so T-1 drops it and takes no `pending` input.
The exact pending UX is a T-7 decision. Escape hatch if in-stream idle optimism
is ever wanted: the **state model** inserts a provisional `Message` Item on ack
(dedup by `store_item_id` via the existing `reconcile.rs` path) — it flows
through the normal projection as an ordinary Item, never reaching back into T-1.

---

## 7. Testing strategy

T-1 is the cheapest slice to test hard and the one everything trusts, so it is
tested exhaustively. Matches the `reduce/` idiom (inline item construction,
hand-asserted; no new snapshot dep).

- **Per-transform unit tests, table-driven:**
  - `pair_tool_spans` — paired / in-progress (output None) / orphan output /
    parallel batch (multiple interleaved call_ids) / out-of-order output.
  - `group_work_section` — settled vs live boundary via `scratch.turn`; no-work
    turn (no section); pure-tool turn (section, no trailing message); AgentChanged
    inside a section; disk-only paint (default scratch → all settled).
  - `WorkSectionMeta` — duration, ordered agent_transitions, tool_count;
    single-item turn (duration ~0); no-transition turn.
  - streaming-tail splice — reasoning-only / message-only / both / neither.
- **Full-pipeline integration fixtures:** a handful of hand-authored realistic
  item lists run through all three stages, asserting the whole `Vec<ViewBlock>`
  tree (cross-transform interactions: agent-changed mid-turn inside a section,
  live turn at tail, disk-paint-only).
- **Invariant test:** every input `Item` appears in exactly one output
  `ViewBlock` (passthrough / paired / grouped) unless an explicit Stage-1 filter
  removed it — a guard against silent projection loss.

Fixture source is hand-authored item lists (targeted, reducer-independent); a
captured `.sse` fixture is not worth the reducer round-trip for unit-testing the
projection. `insta` is reconsidered only if the nested `WorkSection` trees become
unreadable to assert by hand.

---

## 8. Dependencies

- **T-1 blocks:** T-2..T-7 (all render off `Vec<ViewBlock>`).
- **T-1 depends on:** nothing new — the domain `Item`/`ItemKind`/`StreamScratch`
  and the existing `transforms.rs` are shipped.
- **Recorded for downstream (not T-1 work):** T-5 carries a reducer prerequisite
  (fold `child_session.*`); T-6 carries a possible state-model prerequisite
  (per-turn usage deltas) for the full chip line. Both surfaced in §6.3 / §5.4.

---

## 9. Success criteria

- `ViewBlock` + the three composite transforms + the staged `project()` land in
  lens-core; the projection matches `ItemKind` exhaustively (no wildcard).
- All §7 tests pass; `xtask gate` green (fmt/clippy/test, zero warnings).
- The pipeline is pure and borrow-only over `(items, scratch)` — no clone, no
  `pending`, no gpui, no session-level input.
