# T-1 — ViewBlock projection pipeline (design)

**Date:** 2026-07-21
**Status:** Design — cross-family reviewed (Grok 4.5 + GPT-5.6) and revised. Ready
for implementation plan (new session). **Blocked on T-0** (authoritative turn identity).
**Owner:** Lens design effort
**Type:** Implementation slice (build), transcript workstream T-1 of T-0..T-7.

Implements the render-pipeline foundation of `docs/design/conversation-transcript.md`
§3 (pure view-projection) + the structural half of §4 (turn lifecycle). This is an
**implementation decomposition** of an already-complete product design — it does
not re-open product questions; it resolves the lens-core specifics and records the
deviations from §3's provisional type shapes.

Sibling slices (STATUS "transcript fan-out"): **T-0** authoritative turn identity ·
T-2 scaffold+virtualized surface · T-3 content/markdown · T-4 tool spans + resource
markers · T-5 sub-agent spans (child-session) · T-6 turn lifecycle + `WorkSectionMeta` ·
T-7 composer & live turn.

---

## 0. What the cross-family review changed (2026-07-21)

The first draft keyed turns on `ctx.turn` and inferred liveness from `scratch.turn`.
Both reviewers independently broke that: `wire_to_domain_item` (`actor/runloop.rs:221-233`)
stamps every catch-up `/items` row `turn: 0` + fetch-time `created_at`, and `scratch.turn`
is RAM-only, defaults to 0, is never restored on wake, and never bumps on
failed/incomplete/cancelled responses. So `ctx.turn` is **not** a usable turn signal for
disk-sourced history — the transcript's steady state.

Verification of the fix: `/items`' `ConversationItem` carries a **required** `response_id`
and `created_at` (`vendor/omnigent-0.5.1/openapi.json:877-965`); the live stream carries the
active `response_id` on `SessionEvent::Status` (`event.rs:51-54`). Both are **available and
currently discarded** — a lens-core gap, not a contract limit. **T-0** makes `response_id`
the single authoritative turn signal; T-1 keys on it, with **no heuristic**.

---

## 1. Scope & boundaries

**T-1 owns** one thing: the **pure projection** of a session's canonical `&[Item]`
(+ RAM-only `StreamScratch`, + the active-response liveness signal) into
`Vec<ViewBlock>` — deterministic, framework-neutral, no gpui, exhaustively
unit-tested. It is the spine every render slice (T-2..T-7) and the History view
read from.

**T-1 does NOT own** (each → its slice):

| Concern | Why not T-1 | Slice |
|---|---|---|
| Making `response_id` authoritative (map/stamp it, `BlockContext`) | State-model plumbing, a prerequisite | **T-0** |
| Any rendering / gpui | T-1 is pure data | T-2+ |
| `OptimisticUser` in the stream | Pending is composer-owned, not a projection input (§6.4) | T-7 |
| `ReconnectBreak` emission | No backing item/scratch datum; needs reconnect timing | T-2 |
| `SubAgentSpan` / `ChildRef` / real `flatten_sub_agents` | Child-**session** model, not `ctx.depth` (§6.3) | T-5 |
| `WorkSectionMeta` (duration/model/tokens/cost/transitions) | Needs per-turn data T-1 can't supply (§5.4) | T-6 |
| `WorkSection` expand/collapse state | Pure UI state (§5.3) | T-6 |

---

## 2. Home & module layout

Lives in **lens-core**, a new module beside the existing render transforms
(`crates/lens-core/src/reduce/transforms.rs`). Rationale: `ViewBlock` borrows
`&Item` (a lens-core type), the projection is pure/framework-neutral, and the
design wants it "reused across surfaces by composing transform sets" — lens-core's
job, not lens-ui's. lens-ui consumes `Vec<ViewBlock>`.

Proposed: `crates/lens-core/src/reduce/view.rs`. Exact filename is a plan detail.

---

## 3. The `ViewBlock` enum

```rust
pub enum ViewBlock<'a> {
    Item(&'a Item),                                          // passthrough
    ToolSpan { call: &'a Item, output: Option<&'a Item> },  // paired by call_id
    WorkSection { response_id: &'a ResponseId, blocks: Vec<ViewBlock<'a>> },
    StreamingReasoning(&'a ReasoningAcc),                   // scratch.open_reasoning
    StreamingMessage(&'a MessageAcc),                       // scratch.open_message
}
```

- **`WorkSection` carries `response_id` as its stable key** (for T-6 to attach
  expansion state + meta) and **no `open`, no `meta`** (§3.1).
- **Streaming variants borrow the whole accumulator**, not `&str`:
  `StreamingMessage` needs `MessageAcc.message_id` for stable streaming→finalized
  identity (transcript §5); `StreamingReasoning` needs `full_text`/`summary_text`/
  `encrypted` to render summary-vs-full and the encrypted case (§7).

The projection **borrows**; nothing in the block tree is cloned. `'a` spans the
`&[Item]` slice, the `&StreamScratch`, and the borrowed `&ResponseId` (all live for
the render frame). `merge_text_for_display` (the one existing transform that returns
**owned** `Vec<Item>`) is therefore not in the T-1 pipeline: if adjacent-message
coalescing is wanted later, the caller binds its owned result to a `let` that
outlives the `ViewBlock`s and projects from that.

### 3.1 Deviations from `conversation-transcript.md` §3 (recorded)

Each applies the doc's own principle that `ViewBlock` "stays thin — mostly
`Item(&Item)` passthroughs plus the handful of composites." I annotate §3 when this
lands.

1. **`WorkSection { open, .. }` → `{ response_id, blocks }`** — `open` is pure UI
   state; render (T-6) owns expansion. `response_id` replaces it as the stable key.
2. **`WorkSection` carries no `meta`** — every field the §4 chip shows
   (duration/model/tokens/cost/transitions) needs per-turn data T-1 can't supply
   (§5.4). Deferred whole to T-6.
3. **`CompactionMarker` / `AgentChangedMarker` dropped as variants** — 1:1
   item-backed, so `Item(&Item)` passthroughs; render extracts fields by matching
   `ItemKind`.
4. **`OptimisticUser` removed entirely** — §6.4.
5. **`SubAgentSpan { child: ChildRef }` removed** — §6.3.
6. **`ReconnectBreak` deferred to T-2** — zero-field marker, no backing item.

Exhaustiveness is unaffected: the **projection matches `ItemKind` exhaustively** (no
wildcard arm) — a server-added `ItemKind` is a compile error — even though the enum
stays small. Adding a variant later (T-2/T-5) is a local, compiler-checked change.

---

## 4. The staged pipeline (approach A)

Three genuinely different signatures, so the pipeline is **staged**, not a uniform
`pipe`. Each surface composes which filters/groupers it runs; stage boundaries are
fixed.

```
Stage 1 — filters       [Item]                              → [&Item]
    hide_reasoning, with_agent_changed_markers, only_agent          (existing, transforms.rs)

Stage 2 — project       [&Item] + &StreamScratch + active_response  → [ViewBlock]
    pair_tool_spans  +  streaming-tail splice                       (new)

Stage 3 — groupers      [ViewBlock] + active_response               → [ViewBlock]
    group_work_section                                              (new)
```

Signatures:

```rust
// Stage 2 core — consumes the (optionally filtered) item view:
pub fn project<'a>(
    items: &[&'a Item],
    scratch: &'a StreamScratch,
    active_response: Option<&'a ResponseId>,   // the session's live response, or None when idle
) -> Vec<ViewBlock<'a>>

// Convenience assembler for the common no-filter path:
pub fn project_all<'a>(items: &'a [Item], scratch: &'a StreamScratch,
                       active_response: Option<&'a ResponseId>) -> Vec<ViewBlock<'a>>
```

Stage 1 filters yield `Vec<&Item>`; Stage 2 consumes that `&[&Item]` view (so a
filtering surface — e.g. History view running `hide_reasoning` — and the no-filter
Chat column share one projector). The contract either way: **pure and borrow-only
over `(items, scratch, active_response)`; no `pending` input.** Note `by_depth`
(`transforms.rs:27`) is **not** a Stage-1 filter — it returns a `BTreeMap`, not a
`[&Item]`, and is not pipeline-composable.

`active_response` is the authoritative liveness signal from T-0 (the session's
current `response_id` when working; `None` when idle or on disk-only paint). It is
**not** derived from `scratch.turn`.

---

## 5. Transforms

### 5.1 `pair_tool_spans` (Stage 2)

Pairs `ItemKind::FunctionCall` with its `FunctionCallOutput` by `call_id`, emitting
`ToolSpan { call, output }`. Pairs from the item slice directly;
`scratch.unpaired_calls` is reduce-local bookkeeping and is **not** consulted.

- Output present → `ToolSpan { call, output: Some(out) }`.
- Non-`completed` call with no matching output → `ToolSpan { call, output: None }`.
- **Orphan `FunctionCallOutput`** (no call in the window — possible at a disk-window
  boundary) → `Item(&Item)` passthrough; never dropped.
- **Ordering:** the `ToolSpan` takes the **call's** position; a consumed output is
  removed from the flat stream (preserving the exactly-once invariant). Output that
  precedes its call in stream order is still paired to the call's position.
- **Duplicates:** first output per `call_id` wins; a second output for the same
  `call_id`, or a re-used `call_id` across a supersession window, is a passthrough
  `Item` (not silently merged). Contracted + tested.

### 5.2 Streaming-tail splice (Stage 2)

After the finalized-item blocks, append the live response's in-flight blocks from
`scratch`, in order — `StreamingReasoning(&open_reasoning)` then
`StreamingMessage(&open_message)`. When `scratch` has neither open accumulator, there
is no streaming tail (settled / disk-only paint).

**Filter consistency:** the splice respects the Stage-1 filter set — e.g. when
`hide_reasoning` is applied, `StreamingReasoning` is **not** spliced. (The first
draft appended streaming post-filter, so live reasoning leaked past `hide_reasoning`.)

**Streaming never enters a `WorkSection`:** streaming variants are the live tail
below all settled sections; Stage 3 does not fold them (§5.3). Contracted.

### 5.3 `group_work_section` (Stage 3) + liveness

Groups each **response's** work blocks into one `WorkSection`, keyed by the shared
`response_id`.

- **Membership = shared `response_id`** on the agent items (authoritative, from T-0):
  a response's `Reasoning`, `ToolSpan`s, `NativeTool`, and inline `AgentChanged`
  passthroughs. This is the load-bearing grouping — no heuristic.
- **Siblings (outside any section):** items with no agent `response_id` — user/
  assistant `Message`s (§4 "final assistant text stays visible"), `ResourceEvent`,
  `Compaction`, `Error`, `SlashCommand`, `TerminalCommand`. User messages sit before
  their response's section by **stream (ordinal) order** — a positional relation, not
  a grouped one. The classification is **exhaustive over `ItemKind`**; the default
  bucket is explicit sibling passthrough, so a future kind can't silently vanish.
- **Live vs settled:** the section whose `response_id == active_response` is the live
  turn — Stage 3 leaves its work **flat** (§4 "work streams as flat rows while
  running"); every other response folds into a `WorkSection`. When `active_response`
  is `None` (idle / disk-only paint), **all** responses fold. Keyed off the
  authoritative live signal, not `scratch.turn` — so it survives wake, disk paint,
  and failed/incomplete/cancelled turns (a new `response_id` is simply a new section).

> Note on §4 collapse *timing*: T-1 folds a completed response into a `WorkSection`
> immediately; keeping the **latest** settled section rendered *expanded* until the
> next user message is render default-open state (T-6), not structure. T-1 emits no
> `open`.

### 5.4 `WorkSectionMeta` — deferred whole to T-6

The §4 chip (`worked for 8.1s · Sonnet→Opus · 12.4k tok · $0.04`) is **not**
projectable in T-1:

- **duration** needs real per-item timestamps. Live items have them; disk history
  gets real `created_at` **only once T-0 maps the wire value** (today it's fetch
  time). Even then, "duration" is a T-6 chip concern.
- **model / tokens / cost** are per-turn and live on `response.completed.response.usage`
  (`openapi.json:2573+`), but `ResponseEvent::Completed` is a payload-less unit
  variant today — Lens **discards** it. Retaining it is a T-6 prerequisite.
- **agent transitions** are derivable from the section's `AgentChanged` items, but
  computing them belongs with the chip (T-6), and doing so from the *unfiltered* turn
  items (not after `with_agent_changed_markers` drops the markers).

So T-1's `WorkSection` is structural only: `{ response_id, blocks }`. All meta is T-6.

### 5.5 Existing transforms reused as-is (Stage 1)

`hide_reasoning`, `with_agent_changed_markers`, `only_agent` stay in `transforms.rs`
untouched. `by_depth` is not pipeline-composable (§4). `merge_text_for_display` is not
wired in (owned-return; §3). `flatten_sub_agents` stays the current identity stub —
its real body is T-5 (§6.3).

---

## 6. Key resolutions (why the scope is what it is)

### 6.1 Pipeline shape = staged (A), not uniform
The three signatures can't uniformly `pipe`; staging is honest about select→project→
group and keeps the shipped `[Item]→[&Item]` transforms unchanged.

### 6.2 Turn identity + liveness = authoritative `response_id` (T-0), not `ctx.turn`/`scratch.turn`
The review proved `ctx.turn`/`scratch.turn` unusable for disk/wake/error paths (§0).
`response_id` is on every agent item (cold) and on `SessionEvent::Status` (live),
currently discarded. T-0 makes it authoritative; T-1 groups and gates on it with no
heuristic. **User-input items carry a distinct id** (a `turn_`/task namespace in the
0.3.0 capture, vs agent `resp_`) — irrelevant to grouping, since user messages are
siblings positioned by order, never inside a section.

### 6.3 Sub-agents are child *sessions*, not `ctx.depth`
Wire model is `session.child_session.*` (`event.rs:94-97,126-130`): a sub-agent is a
separate session linked by `parent_session_id`, not a `ctx.depth > 0` row. So §3's
`flatten_sub_agents (depth-1)` / `SubAgentSpan { child: ChildRef }` are modeled on
data that doesn't exist. The in-transcript span (§8.6) is its own slice **T-5**
(child-session fold + span projection + §8.6 render + navigate-into-child) — in this
workstream, not foundation, not buried in T-4. T-1 omits its types.

### 6.4 Pending is composer-owned, out of the projection
An optimistic user message inline at the tail conflicts with §16 auto-follow (the
stick-to-bottom anchor would hold the pending bubble, not the streaming response); a
composer-adjacent stack dissolves it. Only naive in-stream optimism needs an
`OptimisticUser` `ViewBlock`, and that's the broken option — so T-1 drops it and takes
no `pending` input. Escape hatch if in-stream idle optimism is ever wanted: the state
model inserts a provisional `Message` Item on ack (dedup by `store_item_id` via
`reconcile.rs`) — flows through as an ordinary Item, never reaching back into T-1.

---

## 7. Testing strategy

T-1 is the cheapest slice to test hard and the one everything trusts. Matches the
`reduce/` idiom (inline item construction, hand-asserted; no new snapshot dep).

- **Per-transform unit tests, table-driven:**
  - `pair_tool_spans` — paired / non-`completed` (output None) / orphan output /
    output-before-call / parallel interleaved `call_id`s / duplicate output.
  - `group_work_section` — membership by shared `response_id`; user message +
    `ResourceEvent` as siblings; live section (== `active_response`) stays flat;
    `active_response = None` folds all; multi-response sequence; a future/unknown kind
    lands in the explicit sibling bucket.
  - streaming-tail splice — reasoning-only / message-only / both / neither; filter
    consistency (`hide_reasoning` suppresses `StreamingReasoning`); streaming never
    grouped.
- **Full-pipeline integration fixtures:** hand-authored realistic item lists through
  all three stages, asserting the whole `Vec<ViewBlock>` tree (agent-changed inside a
  section, live response at tail, disk-only paint with `active_response = None`).
- **Invariant test:** every input `Item` appears in exactly one output `ViewBlock`
  (passthrough / paired / grouped) unless an explicit Stage-1 filter removed it.

Fixtures are hand-authored item lists (targeted, reducer-independent). `insta` only if
`WorkSection` trees become unreadable to hand-assert.

---

## 8. Dependencies

- **T-1 depends on T-0** — `response_id` authoritative on items (`BlockContext`),
  active-response liveness signal, real `created_at`. T-1 cannot key turns correctly
  without it.
- **T-1 blocks:** T-2..T-7 (all render off `Vec<ViewBlock>`).
- **Flagged downstream (not T-1):**
  - Live **in-progress tool calls** may not reach the projector through the shipped
    detailed feed (they sit in `state.items`; the feed exposes scratch + committed-disk
    watermark — `actor/runloop.rs:~1177`). A **T-2** actor-feed sourcing dependency:
    T-1's `ToolSpan { output: None }` contract is correct as a pure function, but
    whether the caller can *supply* live in-progress items is T-2's to solve.
  - Modeling `response.completed.response.usage` — a **T-6** prerequisite.

---

## 9. Success criteria

- `ViewBlock` + the composite transforms + the staged `project()` land in lens-core;
  the projection matches `ItemKind` exhaustively (no wildcard).
- Turns group and gate on the authoritative `response_id` (from T-0); no `ctx.turn`/
  `scratch.turn` heuristic; correct under wake, disk-only paint, and
  failed/incomplete/cancelled turns.
- All §7 tests pass; `xtask gate` green (fmt/clippy/test, zero warnings).
- The pipeline is pure and borrow-only over `(items, scratch, active_response)` — no
  clone in the block tree, no `pending`, no gpui, no other session-level input.
