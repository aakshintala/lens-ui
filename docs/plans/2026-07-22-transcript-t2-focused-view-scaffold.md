# Transcript T-2 — Focused View Scaffold + Live Disk-Sourced Surface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mount a live focused transcript into the shell's `#chat-slot`, backed by a store-side `FocusedTranscript` replica that sources finalized rows from disk (baseline + forward-delta + reconcile re-read), splices the live tail from the actor's scratch, projects through T-1 into **owned** row presentations, and renders through gpui's native `list()` — the first real consumer of the T-1 `Vec<ViewBlock>` projection, with a structurally flash-free streaming→finalize handoff.

**Architecture:** The single-consumer actor feed stays drained by one poller, which fans each drained **batch** through a new `FleetStore::fold_session_feed` (`WeakEntity`) to the card (chrome, unchanged) **and**, when focused, to a store-owned `FocusedTranscript` replica installed **before** `Promote`. The replica reads finalized items on **one dedicated serialized reader worker** holding a read-only `TranscriptReader` (busy_timeout, no DDL), via a single transactional `(ordinal, Item) + watermark` primitive, focus-generation-gated. Rows are two-level retained gpui entities (a `WorkSection` per `response_id` owning work-child entities), keyed so finalize flips a **derived render flag** and swaps streaming children in place — nothing remounts. Three precise actor→replica disk signals (`TranscriptAdvanced` append / `TranscriptRewritten` in-place / reconcile-epoch coarse re-read) keep the disk mirror correct.

**Tech Stack:** Rust 2024; lens-core (`reduce/`, `persist/`, `actor/`); lens-ui (`fleet/`, new `focused/` module, `slot/`); gpui 0.2.2 native `list()` / `ListState` / `ListAlignment::Bottom`; rusqlite (WAL, read-only handle); `#[cfg(test)]` inline unit tests + a real-window `Application::new().run()` harness for identity/paint/scroll.

## Global Constraints

- **Spec:** `docs/specs/2026-07-21-transcript-t2-focused-view-scaffold-design.md` (rev 4, architecture-locked). This plan is its faithful decomposition; **do not re-open product questions** — it resolves the four deferred mechanism items (below) with TDD.
- **Four mechanism decisions resolved for this plan (were spec-deferred):**
  1. **Section identity = `(response_id, run_index)` — per contiguous run, NOT merged (D-3 refinement, user-approved 2026-07-22).** Sections are per contiguous agent-work run (as original T-1); a sibling message mid-turn leaves the run boundary intact, so an interleaved turn renders **multiple chips in chronological order** with the messages visible between them — not one merged chip with narration hoisted below. Confirmed against `docs/spikes/captures/2026-06-26-live-recapture/claude-native-todos.sse` (one `response_id` `resp_claude_3b905154…` with assistant messages interleaved between tool-call runs). A′'s flash-free core is unchanged: the section entity is keyed by `(response_id, run_index)` (finalize-stable — `run_index` doesn't shift when a run's streaming tail settles; only the rare coarse reconcile re-keys), and the **collapse flag is derived per `response_id`** so §4 timing folds a whole turn's runs together. Supersedes the round-3 "one section per `response_id`" framing (that optimized entity simplicity at the cost of chronology). Metadata-per-run vs per-turn is a **T-6** `WorkSectionMeta` question (T-2 stubs the chip).
  2. **`Retired { acc_id, disposition }`** emitted at three sites (Task 4).
  3. **Live re-projection index = `live_section_start: usize`** (Task 9) — the live turn's items are contiguous at the `items` tail, recomputed on `ActiveResponseChanged`.
  4. **Silent re-fire → precise `TranscriptRewritten { ordinal }` signal (Task 5)**, not a reconnect proxy. The actor→replica contract announces *every* below-watermark write it performs: append (`TranscriptAdvanced`), in-place re-fire (`TranscriptRewritten`), scattered reconcile (coarse epoch re-read). User-approved 2026-07-22.
- **Delegation (CLAUDE.md):** default subagent work → `cursor-delegate` on `composer-2.5`; **every lens-core change gets ≥1 cross-family review** (gpt-5.6 via `codex exec -s read-only … < /dev/null`, the `< /dev/null` avoids the stdin hang). Opus subagent only for the §6/§12 staged-finalize architecture + final synthesis.
- **Gate:** `cargo run -p xtask -- gate` must stay green (fmt + workspace clippy `-D warnings` + tests + drift). There is **no** `cargo xtask` alias. Delegated gates MUST include `cargo fmt --check`.
- **Real-window proof (MANDATORY where noted):** `#[gpui::test]`/`TestAppContext` fakes the text system and false-greens paint/identity/scroll ([[gpui-test-noop-text-system]], [[terminal-realwindow-harness-pitfalls]]). Identity/paint/scroll assertions run under `Application::new().run()` (`harness=false`), asserting on **every intervening paint** — the run is the only proof. Fold-logic units use an in-memory `TranscriptReader`.
- **Scope fences (do NOT implement here — each → its slice):** byte-budgeted **windowed baseline** (T-2 loads all resident on open), **scroll-back paging**, **bounded-tail** scoping of the reconcile re-read (T-2 re-reads the whole resident set — O(N), correct, rare) → **T-2b**. Rich message/reasoning/tool content → T-3/T-4 (T-2 renders **stubs** for those `ViewBlock` variants — stubs are *replaced*, not extended around). Live in-progress tool-tail feed extension → **T-4**. `WorkSectionMeta` chip content / composer / interrupt / elicitation → T-6/T-7. Polymorphic `ContentTab` protocol → terminal-UI-integration (leave `ContentTab` an inert marker).
- **Merge coordination:** `terminal-ws` concurrently touches `reduce/`. T-2's `reduce/update.rs` + `reduce/snapshot.rs` + `reduce/folds.rs` touches are small; second-to-merge reconciles.

---

## File Structure

**New module tree — `crates/lens-ui/src/focused/`:**
- `focused/mod.rs` — `FocusedTranscript` gpui `Entity` (state, batch fold rules, staged finalize, projection driver).
- `focused/reader.rs` — dedicated serialized reader worker + `TranscriptReader` client wrapper + focus-generation gating + bounded coalescing target queue + `Retryable`/`Fatal` states.
- `focused/rowsource.rs` — production `RowStore` (id-keyed retained `Entity<RowState>`, **owned** `RowPresentation`, `ListState::splice`/`reset` discipline), lifted from `spikes/transcript-virtual/src/rowsource.rs`.
- `focused/view.rs` — the gpui `Render` surface: `list()` wiring, four §16 scroll contracts, stub row renderers; `focused_transcript_tab(replica, cx) -> TabHandle`.

**Modified — lens-ui:**
- `fleet/store.rs` — retain a per-session **reader factory** (`data_dir` + `conn_id` + `session_id`) + current **reconcile epoch**; install the replica in `focus_session` **before** `Promote`; add `fold_session_feed`.
- `fleet/poller.rs` — fan the drained feed batch through a `WeakEntity<FleetStore>` (not `card` directly); route `ActorOutcome::TransportChanged.reconcile_in_flight` to the replica.
- `slot/mod.rs` — add `focused_transcript_tab`; **`ContentTab` untouched** (inert marker).
- `board/mod.rs` — mount the focused tab in `#chat-slot` (replaces the literal `"chat"`, `board/mod.rs:266`).
- `lib.rs` — `pub mod focused;`.

**Modified — lens-core (each cross-family reviewed):**
- `reduce/view.rs` — **T-1 amendment** (Task 1): uniform response-keyed grouping merging non-consecutive runs; `StreamingReasoning { response_id, acc }`.
- `reduce/update.rs` — `StreamUpdate::Reconnected { gap: Option<u64> }` (Task 2); `Retired { acc_id, disposition }` + `TranscriptRewritten { ordinal }` (Tasks 4/5).
- `reduce/snapshot.rs` — thread `gap` into `on_reconnected`; emit `Discarded` on gap≠Some(0) scratch clear (Tasks 2/4).
- `reduce/folds.rs` — emit `Discarded` + retire scratch on `Failed`/`Incomplete`/`Cancelled` (`folds.rs:221`) (Task 4).
- `reduce/mod.rs`, `reduce/items.rs`, `reduce/scratch.rs` — `Retired { Finalizing }` at `Completed`; `AccId` minting (Task 3/4).
- `domain/item.rs`, `domain/ids.rs` — `AccId` branded id; `acc_id` on `MessageAcc`/`ReasoningAcc` (Task 3).
- `domain/session.rs` — monotonic `next_acc_seq` mint counter (Task 3).
- `actor/runloop.rs` — emit `TranscriptRewritten { ordinal }` on the in-place re-fire path (`commit_terminal_prefix`) (Task 5).
- `persist/transcript.rs`, `persist/mod.rs`, `persist/db.rs`, `persist/map.rs` — new read-only `TranscriptReader` trait + opener + transactional ranged `read_range` primitive (Task 6).

### Reference: exact current shapes this plan builds on (verified 2026-07-22)

```rust
// domain/item.rs — accumulators (Task 3 adds acc_id to both)
pub struct StreamScratch { pub open_message: Option<MessageAcc>, pub open_reasoning: Option<ReasoningAcc>,
    pub unpaired_calls: HashMap<CallId,ItemId>, pub turn: u32, pub current_agent: Option<String> }
pub struct MessageAcc { pub message_id: Option<String>, pub text: String, pub block_index: usize }
pub struct ReasoningAcc { pub full_text: String, pub summary_text: String, pub encrypted: bool } // derives Default

// reduce/view.rs — the two functions Task 1 amends
pub fn project_filtered<'a>(items:&[&'a Item], scratch:&'a StreamScratch,
    _active_response: Option<&'a ResponseId>, splice_reasoning: bool) -> Vec<ViewBlock<'a>>; // stamps StreamingReasoning
pub fn group_work_section<'a>(blocks: Vec<ViewBlock<'a>>, active_response: Option<&'a ResponseId>) -> Vec<ViewBlock<'a>>; // per-run + run_index; fold live too

// reduce/update.rs — the enum Tasks 2/4/5 extend
pub enum StreamUpdate { TranscriptAdvanced{committed_ordinal:i64}, ScratchChanged(Arc<StreamScratch>),
    ActiveResponseChanged(Option<ResponseId>), Reconnected /*→ {gap}*/, Rebased(Box<SessionState>), /* … */ }

// actor/feed.rs / actor/outcome.rs
pub enum ActorFeed { Summary(Box<SummaryUpdate>), Detailed(StreamUpdate) }
pub enum ActorOutcome { TransportChanged{transport:ActorTransport, reconcile_in_flight:bool}, PersistError{..}, Parked{..}, /* … */ }

// persist/mod.rs — the read primitives Task 6 mirrors read-only
pub trait TranscriptStore { fn load_items(&self)->Result<Loaded<Item>>; fn store_frontier(&self)->Result<Option<(i64,ItemId)>>; /* write API … */ }
pub struct Loaded<T> { pub rows: Vec<T>, pub skipped: Vec<SkippedRow> }
pub enum StoreMode { ReadWrite, ReadOnlyDegraded }
// persist/map.rs: pub(crate) fn row_to_item(row:&rusqlite::Row)->Result<Item>; collect_skipping(rows, id_col, decode)

// fleet/store.rs (current)
pub struct FleetStore { pub cards: HashMap<SessionId, Entity<SessionCard>>, pub focused: Option<SessionId>,
    pub fake: Option<FakeFleet>, scheduler: Option<FleetScheduler>, clock: Arc<dyn UiClock>,
    store_notify_count: Cell<u64>, command_txs: HashMap<SessionId, Sender<SessionCommand>>,
    pollers: HashMap<SessionId, Task<()>>, stream_bridges: HashMap<SessionId, StreamBridge> }
// spawn_live_session takes data_dir:&Path (NOT retained); focus_session sends Demote(prev)/Promote(id).

// spikes/transcript-virtual/src/rowsource.rs (lift source)
pub struct RowStore { pub(crate) order: Vec<RowId>, entities: HashMap<RowId, Entity<RowState>> }
pub struct RowState { pub id: RowId, pub kind: RowKind, pub text: String, pub height_delta: Pixels,
    pub use_markdown: bool, /* … */ pub measured_height: Option<Pixels> }
// list(list_state, move |ix, window, app| entity.update(...))  — 'static closure captures entity, re-enters.
// ListState::new(n, ListAlignment::Bottom, OVERDRAW); .reset(n); .splice(range, count).
```

---

# Phase A — lens-core foundation (Tasks 1–6)

Each Phase-A task is a self-contained lens-core change with inline tests and a gate + cross-family review. They can land before any lens-ui work and unblock the replica (Phase B).

---

## Task 1: T-1 amendment — per-run sections (fold live too) + `run_index` keying + streaming-reasoning attribution

Revises the already-executed (unmerged) T-1 `view.rs`. This is a **small** change from original T-1: keep run-based grouping (per contiguous run), but (a) drop the flat-when-live special case so **every** run folds into a `WorkSection` (the live/expanded decision moves entirely to T-2's renderer), (b) stamp each section with a per-response `run_index` for finalize-stable entity keying, and (c) fold the live `StreamingReasoning` into its run and carry its `response_id`. Projection **keeps** `active_response` for streaming-tail attribution; `group_work_section` no longer uses it for flat-vs-grouped.

**Files:**
- Modify: `crates/lens-core/src/reduce/view.rs` (`ViewBlock::WorkSection` + `StreamingReasoning`, `project_filtered`, `group_work_section`, `grouping_key`)
- Test: inline `#[cfg(test)] mod tests` in `view.rs` (update ≥4 of the 21 existing; add per-run + run_index + attribution cases)

**Interfaces:**
- Consumes: `Item`, `ReasoningAcc`, `MessageAcc`, `StreamScratch`, `ResponseId` (unchanged shapes).
- Produces (amended):
  ```rust
  pub enum ViewBlock<'a> {
      Item(&'a Item),
      ToolSpan { call: &'a Item, output: Option<&'a Item> },
      WorkSection { response_id: &'a ResponseId, run_index: u32, blocks: Vec<ViewBlock<'a>> }, // per contiguous run
      StreamingReasoning { response_id: Option<&'a ResponseId>, acc: &'a ReasoningAcc }, // was: StreamingReasoning(&'a ReasoningAcc)
      StreamingMessage(&'a MessageAcc), // unchanged — stays a top-level sibling
  }
  // group_work_section(blocks, _active) → one WorkSection per contiguous run; run_index = count of prior
  // runs of the SAME response_id (0,1,2…), so the live turn's runs are (R,0),(R,1),… — finalize-stable keys.
  ```

- [ ] **Step 1: Write the failing per-run test** — a sibling message splitting one response into two runs yields **two** sections in chronological order (msg between them), with `run_index` 0 and 1.

```rust
#[test]
fn interleaved_message_keeps_two_runs_in_order_with_run_index() {
    // reasoning(resp_a), assistant msg(resp_a) [sibling], reasoning(resp_a) again.
    let items = [
        reasoning("r1", Some("resp_a")),
        msg("a1", Some("resp_a"), Role::Assistant, "narration"),
        reasoning("r2", Some("resp_a")),
    ];
    let refs: Vec<&Item> = items.iter().collect();
    let scratch = scratch_with(None, None);
    let out = group_work_section(project(&refs, &scratch, None), None);
    // section(resp_a,#0){r1} | a1 sibling IN PLACE | section(resp_a,#1){r2} — order preserved.
    assert_eq!(out.len(), 3);
    let (i0, ri0) = assert_section_ri(&out[0], "resp_a"); // (blocks, run_index)
    assert_eq!(ri0, 0);
    assert_item(&i0[0], "r1");
    assert_item(&out[1], "a1"); // message stays between the runs
    let (i1, ri1) = assert_section_ri(&out[2], "resp_a");
    assert_eq!(ri1, 1);
    assert_item(&i1[0], "r2");
}
```

(Add an `assert_section_ri(vb, resp) -> (&[ViewBlock], u32)` helper alongside `assert_section`, returning `blocks` + `run_index`; keep `assert_section` as a `run_index`-agnostic wrapper for the existing single-run tests.)

- [ ] **Step 2: Write the failing streaming-attribution test** — `StreamingReasoning` carries the active `response_id`.

```rust
#[test]
fn streaming_reasoning_carries_active_response_id() {
    let items: [Item; 0] = [];
    let refs: Vec<&Item> = items.iter().collect();
    let scratch = scratch_with(Some(r_acc()), None);
    let resp_a = ResponseId::new("resp_a");
    let out = project(&refs, &scratch, Some(&resp_a));
    match &out[0] {
        ViewBlock::StreamingReasoning { response_id, .. } =>
            assert_eq!(response_id.map(|r| r.as_str()), Some("resp_a")),
        other => panic!("expected StreamingReasoning, got {other:?}"),
    }
}
```

- [ ] **Step 3: Run both, verify they fail**

Run: `cargo test -p lens-core reduce::view 2>&1 | tail -20`
Expected: FAIL — `interleaved_message_keeps_two_runs…` fails to compile (`WorkSection` has no `run_index`, `assert_section_ri` undefined); `streaming_reasoning…` fails to compile (variant is a tuple, not a struct).

- [ ] **Step 4: Amend `ViewBlock::StreamingReasoning` + `project_filtered`** — carry `response_id`, sourced from the (now used) `active_response` param.

```rust
// in the enum:
StreamingReasoning { response_id: Option<&'a ResponseId>, acc: &'a ReasoningAcc },

// project_filtered: use the active_response param (drop the leading underscore) for the reasoning tail.
pub fn project_filtered<'a>(
    items: &[&'a Item],
    scratch: &'a StreamScratch,
    active_response: Option<&'a ResponseId>,
    splice_reasoning: bool,
) -> Vec<ViewBlock<'a>> {
    let mut blocks = pair_tool_spans(items);
    if splice_reasoning && let Some(r) = &scratch.open_reasoning {
        blocks.push(ViewBlock::StreamingReasoning { response_id: active_response, acc: r });
    }
    if let Some(m) = &scratch.open_message {
        blocks.push(ViewBlock::StreamingMessage(m));
    }
    blocks
}
```

Update `grouping_key` + `covered_item_ids` for the new `StreamingReasoning { .. }` shape (both currently match `StreamingReasoning(_)`). **Crucially** — per §11 the live reasoning tail splices *under* the live section, so `grouping_key(StreamingReasoning { response_id, .. })` now **returns `*response_id`** (groups into its section), while `StreamingMessage(_)` stays **`None`** (top-level sibling). This is the mechanism by which `group_work_section` folds the live reasoning into the live section:

```rust
match vb {
    ViewBlock::Item(i) => item_key(i),
    ViewBlock::ToolSpan { call, .. } => item_key(call),
    ViewBlock::StreamingReasoning { response_id, .. } => *response_id, // §11: under the live section
    ViewBlock::WorkSection { .. } | ViewBlock::StreamingMessage(_) => None,
}
```

- [ ] **Step 5: Rewrite `group_work_section` for per-run sections with `run_index`** — this is the original T-1 run-based flush minus the flat-when-live branch, plus a per-response run counter.

```rust
/// Stage 3: fold each CONTIGUOUS agent-work run into a `WorkSection`. A sibling (message,
/// ResourceEvent, …) closes the current run and renders in place, so an interleaved turn
/// stays chronological (multiple sections + the siblings between them). Each section carries
/// `run_index` = the number of prior runs of the SAME `response_id`, giving finalize-stable
/// `(response_id, run_index)` keys. `_active` is unused — the collapse decision is the renderer's
/// (derived per `response_id`, T-2 §6/§12).
pub fn group_work_section<'a>(
    blocks: Vec<ViewBlock<'a>>,
    _active: Option<&'a ResponseId>,
) -> Vec<ViewBlock<'a>> {
    use std::collections::HashMap;
    let mut out: Vec<ViewBlock<'a>> = Vec::with_capacity(blocks.len());
    let mut run: Vec<ViewBlock<'a>> = Vec::new();
    let mut run_key: Option<&'a ResponseId> = None;
    let mut run_counts: HashMap<&'a ResponseId, u32> = HashMap::new();

    fn flush<'a>(
        out: &mut Vec<ViewBlock<'a>>,
        run: &mut Vec<ViewBlock<'a>>,
        run_key: &mut Option<&'a ResponseId>,
        run_counts: &mut HashMap<&'a ResponseId, u32>,
    ) {
        if let Some(key) = run_key.take() {
            let idx = run_counts.entry(key).or_insert(0);
            out.push(ViewBlock::WorkSection {
                response_id: key,
                run_index: *idx,
                blocks: std::mem::take(run),
            });
            *idx += 1;
        }
        run.clear();
    }

    for vb in blocks {
        match grouping_key(&vb) {
            Some(key) if run_key == Some(key) => run.push(vb),
            Some(key) => {
                flush(&mut out, &mut run, &mut run_key, &mut run_counts);
                run_key = Some(key);
                run.push(vb);
            }
            None => {
                flush(&mut out, &mut run, &mut run_key, &mut run_counts);
                out.push(vb);
            }
        }
    }
    flush(&mut out, &mut run, &mut run_key, &mut run_counts);
    out
}
```

- [ ] **Step 6: Update the affected existing tests** — under A′ the live response now **folds**, and the live `StreamingReasoning` now folds **into** its section (no longer a trailing flat tail):
  - `live_response_stays_flat` → renamed/rewritten to `live_response_also_folds_into_section` (below).
  - `streaming_tail_never_grouped` → now the streaming reasoning tail **is** grouped under its `resp_a` section (flip the assertion: expect the section's last child to be the `StreamingReasoning`, not a top-level sibling after it).
  - `idle_folds_all_but_active_folds_all_others`, `full_pipeline_agent_changed_inside_section_live_tail` → the active response folds like the rest; the `StreamingReasoning` sits inside the live section, `StreamingMessage` stays a top-level sibling after it.

```rust
#[test]
fn live_response_also_folds_into_section() {
    let items = [
        reasoning("r1", Some("resp_a")),
        call("c1", Some("resp_a"), "call_1", "completed"),
        output("o1", Some("resp_a"), "call_1"),
    ];
    let refs: Vec<&Item> = items.iter().collect();
    let scratch = scratch_with(None, None);
    let resp_a = ResponseId::new("resp_a");
    // Even when resp_a is active, grouping folds it — expanded-vs-collapsed is the renderer's job now.
    let out = group_work_section(project(&refs, &scratch, Some(&resp_a)), Some(&resp_a));
    assert_eq!(out.len(), 1);
    let inner = assert_section(&out[0], "resp_a");
    assert_eq!(inner.len(), 2); // reasoning + tool-span
}
```

- [ ] **Step 7: Run the view tests to green**

Run: `cargo test -p lens-core reduce::view 2>&1 | tail -20`
Expected: PASS (all, including the two new + the rewritten legacy cases).

- [ ] **Step 8: Annotate the T-1 spec §5.3 superseded note is already present; verify no other caller breaks**

Run: `cargo build -p lens-core 2>&1 | tail -20`
Expected: clean (no other crate consumes `StreamingReasoning`'s tuple shape yet — T-2 is the first consumer).

- [ ] **Step 9: Gate + cross-family review, then commit**

Run: `cargo run -p xtask -- gate 2>&1 | tail -15`
Cross-family: `codex exec -s read-only "Review the diff on reduce/view.rs: does group_work_section emit one WorkSection per CONTIGUOUS run (siblings stay in place, chronological order preserved), assign run_index = count of prior runs of the same response_id (finalize-stable), fold the live run too, and place StreamingReasoning inside its run while StreamingMessage stays a top-level sibling? Every input item exactly once?" < /dev/null`

```bash
git add crates/lens-core/src/reduce/view.rs
git commit -m "feat(reduce): T-1 amendment — response-keyed WorkSection merging non-consecutive runs (T-2 A′)"
```

---

## Task 2: `StreamUpdate::Reconnected { gap: Option<u64> }`

The wire event `ServerStreamEvent::Reconnected { gap }` already carries the gap, but `on_reconnected` collapses it into a **unit** `StreamUpdate::Reconnected`, discarding the value the replica needs to inject a `ReconnectBreak` (Task 14). Widen the `StreamUpdate` variant and thread the gap through.

**Files:**
- Modify: `crates/lens-core/src/reduce/update.rs` (variant), `crates/lens-core/src/reduce/snapshot.rs` (`on_reconnected`)
- Test: inline in `snapshot.rs`

**Interfaces:**
- Produces: `StreamUpdate::Reconnected { gap: Option<u64> }` (was unit). Consumed by the replica (Task 9) → `ReconnectBreak` (Task 14).

- [ ] **Step 1: Update the failing test** — assert the emitted update carries the gap.

```rust
#[test]
fn reconnected_update_carries_gap() {
    let mut s = st();
    reduce(&mut s, &resp_text("partial", None, None), &clock());
    let u = reduce(&mut s, &ServerStreamEvent::Reconnected { gap: Some(3) }, &clock());
    assert!(u.iter().any(|x| matches!(x, StreamUpdate::Reconnected { gap: Some(3) })));
}
```

- [ ] **Step 2: Run, verify it fails to compile** (`Reconnected` is a unit variant).

Run: `cargo test -p lens-core reduce::snapshot 2>&1 | tail -15` → FAIL (compile).

- [ ] **Step 3: Widen the variant** in `update.rs`:

```rust
// was: Reconnected,
Reconnected { gap: Option<u64> },
```

- [ ] **Step 4: Thread the gap** in `snapshot.rs::on_reconnected`:

```rust
pub(crate) fn on_reconnected(state: &mut SessionState, gap: Option<u64>) -> Updates {
    let mut u: Updates = smallvec![StreamUpdate::Reconnected { gap }];
    if gap != Some(0) {
        // (Task 4 adds Discarded emission here for any open accumulator.)
        let had = state.stream.open_message.is_some()
            || state.stream.open_reasoning.is_some()
            || !state.stream.unpaired_calls.is_empty();
        state.stream.open_message = None;
        state.stream.open_reasoning = None;
        state.stream.unpaired_calls.clear();
        if had {
            u.push(StreamUpdate::ScratchChanged(Arc::new(state.stream.clone())));
        }
    }
    u
}
```

- [ ] **Step 5: Fix the existing assertions** — `reconnected_with_gap_clears_scratch_not_pending_user` uses `u.contains(&StreamUpdate::Reconnected)`; change to match `Reconnected { gap: None }`.

- [ ] **Step 6: Run to green + gate**

Run: `cargo test -p lens-core reduce::snapshot 2>&1 | tail -10 && cargo run -p xtask -- gate 2>&1 | tail -8`
Expected: PASS + gate green. (This is a small additive change — cross-family review folds into Task 4's, which touches the same file.)

- [ ] **Step 7: Commit**

```bash
git add crates/lens-core/src/reduce/update.rs crates/lens-core/src/reduce/snapshot.rs
git commit -m "feat(reduce): widen StreamUpdate::Reconnected to carry gap (T-2 ReconnectBreak)"
```

---

## Task 3: `AccId` on `MessageAcc` / `ReasoningAcc`, minted at open

The replica keys streaming children (the live reasoning/message tail rows) by a stable id so it can stage them across finalize (Task 4/12). Today `ReasoningAcc` carries **no** id and `MessageAcc.message_id` is `Option` (absent for unkeyed messages). Give **both** accumulators a stable `acc_id: AccId` minted the moment the accumulator opens, from a monotonic per-session counter (deterministic — no clock-collision hazard, cf. `local_id`).

**Files:**
- Modify: `crates/lens-core/src/domain/ids.rs` (add `AccId` to the `branded_id!` set), `domain/item.rs` (`acc_id` field on both accs), `domain/session.rs` (`next_acc_seq: u64` + `mint_acc_id`), `reduce/scratch.rs` (mint at accumulator creation), `reduce/mod.rs` (mint at `ReasoningStarted`)
- Test: inline in `scratch.rs`

**Interfaces:**
- Produces:
  ```rust
  // domain/ids.rs
  branded_id!(ItemId, CallId, ResponseId, AgentId, BoardId, BoardItemId, AccId);
  // domain/item.rs
  pub struct MessageAcc  { pub acc_id: AccId, pub message_id: Option<String>, pub text: String, pub block_index: usize }
  pub struct ReasoningAcc{ pub acc_id: AccId, pub full_text: String, pub summary_text: String, pub encrypted: bool }
  // domain/session.rs
  impl SessionState { pub fn mint_acc_id(&mut self) -> AccId; } // format!("acc_{}", n), n = next_acc_seq++
  ```
- Consumes: nothing new. `acc_id` read by Task 4 (Retired) + the replica (Task 12) via the `StreamingReasoning`/`StreamingMessage` ViewBlocks (which borrow the accs).

- [ ] **Step 1: Write the failing mint-stability test**

```rust
#[test]
fn open_message_keeps_stable_acc_id_across_deltas() {
    let mut s = st();
    reduce(&mut s, &resp_text("hel", None, None), &clock());
    let first = s.stream.open_message.as_ref().unwrap().acc_id.clone();
    reduce(&mut s, &resp_text("lo", None, None), &clock());
    let second = s.stream.open_message.as_ref().unwrap().acc_id.clone();
    assert_eq!(first, second, "acc_id must be stable across streaming deltas");
}

#[test]
fn reasoning_and_message_accs_get_distinct_acc_ids() {
    let mut s = st();
    reduce(&mut s, &ServerStreamEvent::Response(ResponseEvent::ReasoningStarted), &clock());
    reduce(&mut s, &resp_text("hi", None, None), &clock());
    let r = s.stream.open_reasoning.as_ref().unwrap().acc_id.clone();
    let m = s.stream.open_message.as_ref().unwrap().acc_id.clone();
    assert_ne!(r, m);
}
```

- [ ] **Step 2: Run, verify fail** (no `acc_id` field). `cargo test -p lens-core reduce::scratch 2>&1 | tail -15` → FAIL (compile).

- [ ] **Step 3: Add `AccId`, the fields, and the mint counter.** `branded_id!(… , AccId)` in `ids.rs`; add `acc_id: AccId` to both structs in `item.rs` (this breaks `ReasoningAcc: Default` — remove the `Default` derive on `ReasoningAcc` and mint explicitly, see Step 5); add to `SessionState`:

```rust
// domain/session.rs
pub next_acc_seq: u64, // init 0 in SessionState::new
pub fn mint_acc_id(&mut self) -> AccId {
    let id = AccId::new(format!("acc_{}", self.next_acc_seq));
    self.next_acc_seq += 1;
    id
}
```

- [ ] **Step 4: Mint at message-accumulator creation** in `scratch.rs::accumulate_text` — where `open_message` is `None` and a new `MessageAcc` is built, call `state.mint_acc_id()` for `acc_id`. (Accumulator creation currently lives in `scratch::accumulate_text`; the signature takes `&mut state.stream` — widen it to take `&mut SessionState` so it can mint, or pass a pre-minted `AccId` from the `reduce` call site. Prefer passing a pre-minted id to keep `scratch` free of `SessionState`.)

- [ ] **Step 5: Mint at reasoning-accumulator creation.** Replace `mod.rs:87-93` `ReasoningStarted`'s `get_or_insert_with(Default::default)` with an explicit mint:

```rust
ResponseEvent::ReasoningStarted => {
    if state.stream.open_reasoning.is_none() {
        let acc_id = state.mint_acc_id();
        state.stream.open_reasoning = Some(ReasoningAcc { acc_id, ..Default::default() });
        // ^ ReasoningAcc no longer derives Default on acc_id; use `Default::default()` for the text fields
        //   via a manual `Default`-less constructor or `#[derive(Default)]` kept with AccId: Default::default()=acc_0.
    }
    smallvec![StreamUpdate::ScratchChanged(Arc::new(state.stream.clone()))]
}
```
(Simplest: keep `#[derive(Default)]` on `ReasoningAcc` — `AccId` derives `Default` via `branded_id!` giving an empty string — but ALWAYS overwrite `acc_id` with a minted value at creation so an empty acc_id never reaches the replica. Assert this in Step 1's tests.)

- [ ] **Step 6: Fix all `MessageAcc`/`ReasoningAcc` constructors** — `reduce/view.rs` test helpers `r_acc()`/`m_acc()`, `item.rs` roundtrip tests, any `scratch.rs` fixtures. Add `acc_id: AccId::new("acc_test")` (or a mint) to each.

- [ ] **Step 7: Run scratch + view + item tests to green**

Run: `cargo test -p lens-core reduce:: 2>&1 | tail -15 && cargo test -p lens-core domain::item 2>&1 | tail -8`
Expected: PASS.

- [ ] **Step 8: Gate + cross-family review, commit**

Run: `cargo run -p xtask -- gate 2>&1 | tail -10`
Cross-family: `codex exec -s read-only "Review the acc_id mint: is every accumulator guaranteed a non-empty, session-unique acc_id at open, stable across deltas, with no clock-collision? Does removing/keeping ReasoningAcc Default leave any path that ships acc_0/empty?" < /dev/null`

```bash
git add crates/lens-core/src/domain crates/lens-core/src/reduce/scratch.rs crates/lens-core/src/reduce/mod.rs
git commit -m "feat(domain): stable AccId minted at accumulator open (T-2 staged finalize key)"
```

---

## Task 4: `Retired { acc_id, disposition }` signal + terminal/reconnect scratch retirement

The reducer emits an **explicit retirement disposition** keyed by `acc_id` so the replica never infers finalize-vs-abandon intent. `Finalizing { item_id }` (a disk row is coming — swap in place) vs `Discarded` (abandoned — drop, no ghost). Emitted on ordinary finalize (`Completed`) **and** on terminal `Failed`/`Incomplete`/`Cancelled` **and** reconnect discontinuity — the latter two currently clear `active_response` **without** retiring scratch (`folds.rs:221`, `snapshot.rs:98`).

**Files:**
- Modify: `reduce/update.rs` (variant + `RetireDisposition`), `reduce/items.rs` (`finalize_message`/`finalize_reasoning` return `Finalizing`), `reduce/mod.rs` (`Completed` collects them), `reduce/folds.rs` (`Failed`/`Incomplete`/`Cancelled` → `Discarded` + clear scratch), `reduce/snapshot.rs` (`on_reconnected` gap≠Some(0) → `Discarded`)
- Test: inline in `items.rs` (finalize), `folds.rs` (terminal), `snapshot.rs` (reconnect)

**Interfaces:**
- Produces:
  ```rust
  pub enum StreamUpdate { /* … */ Retired { acc_id: AccId, disposition: RetireDisposition } }
  #[derive(Clone, Debug, PartialEq)]
  pub enum RetireDisposition { Finalizing { item_id: ItemId }, Discarded }
  ```
- Consumed by the replica (Task 12): `Finalizing` → stage the child keyed by `acc_id` under its section, swap when the disk row for `item_id` arrives; `Discarded` → drop the streaming child.

- [ ] **Step 1: Write the failing finalize test** — `Completed` emits `Finalizing { item_id }` for both accumulators, keyed by their `acc_id`.

```rust
#[test]
fn completed_emits_finalizing_for_message_and_reasoning() {
    let mut s = st();
    let clk = clock();
    reduce(&mut s, &ServerStreamEvent::Response(ResponseEvent::ReasoningStarted), &clk);
    reduce(&mut s, &ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta { delta: "why".into() }), &clk);
    reduce(&mut s, &resp_text("hi", Some("msg_A"), None), &clk);
    let r_acc_id = s.stream.open_reasoning.as_ref().unwrap().acc_id.clone();
    let m_acc_id = s.stream.open_message.as_ref().unwrap().acc_id.clone();
    let u = reduce(&mut s, &ServerStreamEvent::Response(ResponseEvent::Completed), &clk);
    // message finalizes to item_id == "msg_A"; reasoning to a synthesized local id.
    assert!(u.iter().any(|x| matches!(x,
        StreamUpdate::Retired { acc_id, disposition: RetireDisposition::Finalizing { item_id } }
        if *acc_id == m_acc_id && item_id.as_str() == "msg_A")));
    assert!(u.iter().any(|x| matches!(x,
        StreamUpdate::Retired { acc_id, disposition: RetireDisposition::Finalizing { .. } }
        if *acc_id == r_acc_id)));
}
```

- [ ] **Step 2: Write the failing terminal-discard test** — `Failed`/`Incomplete`/`Cancelled` emit `Discarded` for any open accumulator **and** clear scratch.

```rust
#[test]
fn terminal_failure_discards_open_accumulators_and_clears_scratch() {
    for term in [ResponseEvent::Failed, ResponseEvent::Incomplete, ResponseEvent::Cancelled] {
        let mut s = st();
        reduce(&mut s, &resp_text("partial", None, None), &clock());
        let acc_id = s.stream.open_message.as_ref().unwrap().acc_id.clone();
        let u = reduce(&mut s, &ServerStreamEvent::Response(term.clone()), &clock());
        assert!(s.stream.open_message.is_none(), "{term:?} must clear scratch");
        assert!(u.iter().any(|x| matches!(x,
            StreamUpdate::Retired { acc_id: a, disposition: RetireDisposition::Discarded } if *a == acc_id)));
    }
}
```

- [ ] **Step 3: Write the failing reconnect-discard test** — gap≠Some(0) emits `Discarded` for any open accumulator (extends `reconnected_with_gap_clears_scratch_not_pending_user`).

```rust
#[test]
fn reconnect_gap_discards_open_accumulator() {
    let mut s = st();
    reduce(&mut s, &resp_text("partial", None, None), &clock());
    let acc_id = s.stream.open_message.as_ref().unwrap().acc_id.clone();
    let u = reduce(&mut s, &ServerStreamEvent::Reconnected { gap: Some(4) }, &clock());
    assert!(u.iter().any(|x| matches!(x,
        StreamUpdate::Retired { acc_id: a, disposition: RetireDisposition::Discarded } if *a == acc_id)));
}
```

- [ ] **Step 4: Run all three, verify fail** — `cargo test -p lens-core reduce:: 2>&1 | tail -20` → FAIL (no `Retired` variant).

- [ ] **Step 5: Add the variant + `RetireDisposition`** to `update.rs`.

- [ ] **Step 6: `finalize_message`/`finalize_reasoning` return `Finalizing`.** Capture `acc.acc_id` before consuming the acc; after `push_item` computes the durable `id`, push `Retired { acc_id, disposition: Finalizing { item_id: id.clone() } }`:

```rust
pub(crate) fn finalize_message(state: &mut SessionState, clock: &dyn Clock) -> Updates {
    let Some(acc) = state.stream.open_message.take() else { return smallvec![]; };
    let acc_id = acc.acc_id.clone();
    let id = acc.message_id.clone().map(ItemId::new).unwrap_or_else(|| local_id("msg", state));
    let kind = ItemKind::Message { role: Role::Assistant, content: vec![ContentBlock {
        kind: "output_text".into(), text: Some(acc.text), data: Value::Null }] };
    let response_id = state.active_response.clone();
    let mut u = push_item(state, id.clone(), kind, None, response_id, clock);
    u.push(StreamUpdate::Retired { acc_id, disposition: RetireDisposition::Finalizing { item_id: id } });
    u
}
// finalize_reasoning: identical shape — capture acc.acc_id, id = local_id("reasoning", state), push Finalizing { item_id: id }.
```

- [ ] **Step 7: `Failed`/`Incomplete`/`Cancelled` → clear scratch + `Discarded`** in `folds.rs:221`:

```rust
ResponseEvent::Failed | ResponseEvent::Incomplete | ResponseEvent::Cancelled => {
    state.active_response = None;
    let mut u: Updates = smallvec![StreamUpdate::ActiveResponseChanged(None)];
    for acc_id in take_open_acc_ids(&mut state.stream) { // drains open_message/open_reasoning, returns their acc_ids
        u.push(StreamUpdate::Retired { acc_id, disposition: RetireDisposition::Discarded });
    }
    if !state.stream.unpaired_calls.is_empty() { state.stream.unpaired_calls.clear(); }
    u.push(StreamUpdate::ScratchChanged(Arc::new(state.stream.clone())));
    u
}
```
Add a small `take_open_acc_ids(&mut StreamScratch) -> SmallVec<[AccId; 2]>` helper (in `scratch.rs`) that `.take()`s both accumulators and returns their acc_ids — reused by `on_reconnected`.

- [ ] **Step 8: `on_reconnected` gap≠Some(0) → `Discarded`** — replace the manual scratch clear (Task 2's block) with `take_open_acc_ids` + `Discarded` pushes (same shape as Step 7).

- [ ] **Step 9: Run to green** — `cargo test -p lens-core reduce:: 2>&1 | tail -20`. Fix `completed_clears_preview_emits_scratch_changed` / `terminal_response_clears_active_and_emits_none` if they assert exact update-vec equality (they use `.any(..)`, so should be tolerant).

- [ ] **Step 10: Gate + cross-family review, commit**

Run: `cargo run -p xtask -- gate 2>&1 | tail -10`
Cross-family: `codex exec -s read-only "Review Retired emission: is EVERY open accumulator retired on Completed (Finalizing), Failed/Incomplete/Cancelled (Discarded), and reconnect gap!=Some(0) (Discarded), with acc_id matching what open minted and item_id the durable finalized id? Any path that clears scratch WITHOUT emitting Retired (would orphan a staged row)?" < /dev/null`

```bash
git add crates/lens-core/src/reduce
git commit -m "feat(reduce): Retired{acc_id,disposition} — Finalizing on Completed, Discarded on terminal/reconnect (T-2 D-2)"
```

---

## Task 5: `TranscriptRewritten { ordinal }` — complete the commit-path signal

Close the actor→replica contract hole: `commit_terminal_prefix` performs an in-place re-fire write (an already-persisted terminal id re-fires, updating content at its preserved ordinal below the watermark) that emits **no** signal today (`runloop.rs` test `refire_of_pruned_item_does_not_gap_ordinals`). Emit a precise `TranscriptRewritten { ordinal }` so the replica re-reads exactly that ordinal — the disk mirror tracks actual writes, not a reconnect proxy.

**Files:**
- Modify: `reduce/update.rs` (variant), `actor/runloop.rs` (`commit_terminal_prefix` detects `stored_ord != requested` and reports it; the call sites feed it to the batch/feed)
- Test: inline in `runloop.rs` (mirror `refire_of_pruned_item_does_not_gap_ordinals`, now asserting exactly one `TranscriptRewritten { ordinal }`)

**Interfaces:**
- Produces: `StreamUpdate::TranscriptRewritten { ordinal: i64 }`. Consumed by the replica (Task 9) → single-ordinal re-read.
- `commit_terminal_prefix` today returns `Option<i64>` (the advanced watermark, or `None`). Widen it to also surface rewritten ordinals:
  ```rust
  struct CommitResult { advanced: Option<i64>, rewritten: SmallVec<[i64; 2]> }
  fn commit_terminal_prefix(..) -> CommitResult
  ```

- [ ] **Step 1: Write the failing test** — a re-fire of an already-persisted id emits exactly one `TranscriptRewritten { ordinal }` (and still no `TranscriptAdvanced`). Base it on `refire_of_pruned_item_does_not_gap_ordinals`; after the third `output_item.done` for `item_a`, assert a `Detailed(TranscriptRewritten { ordinal: 0 })` frame arrives and no `TranscriptAdvanced`.

- [ ] **Step 2: Run, verify fail** — `cargo test -p lens-core actor::runloop::tests::refire 2>&1 | tail -15` → FAIL (no variant / no frame).

- [ ] **Step 3: Add the variant** to `update.rs`: `TranscriptRewritten { ordinal: i64 }`.

- [ ] **Step 4: Detect the in-place write in `commit_terminal_prefix`** — the branch where `upsert_item` returns `stored_ord != requested` (currently `state.items.remove(0)` without setting `committed`). Record `stored_ord` in `rewritten`:

```rust
Ok(stored_ord) => {
    let requested = *next_ordinal;
    state.items.remove(0);
    if stored_ord == requested {
        *next_ordinal += 1;
        advanced = Some(*next_ordinal - 1);
    } else {
        rewritten.push(stored_ord); // in-place re-fire at an existing ordinal
    }
}
```

- [ ] **Step 5: Emit at the call sites** — each caller of `commit_terminal_prefix` (`apply_reduced_batch` ~663, `finish_deferred_transcript_commit` ~821) already pushes `TranscriptAdvanced` from `advanced`; also push one `TranscriptRewritten { ordinal }` per `rewritten` entry (into the same batch / via `feed.send_blocking`).

- [ ] **Step 6: Run to green** — `cargo test -p lens-core actor:: 2>&1 | tail -15`.

- [ ] **Step 7: Gate + cross-family review, commit**

Run: `cargo run -p xtask -- gate 2>&1 | tail -10`
Cross-family: `codex exec -s read-only "Review commit_terminal_prefix rewritten-ordinal detection: is stored_ord!=requested the exact and only in-place-rewrite condition? Can an append ever be misclassified as a rewrite or vice versa? Is TranscriptRewritten emitted for reconcile-path writes too (it must NOT — reconcile is the coarse epoch path)?" < /dev/null`

```bash
git add crates/lens-core/src/reduce/update.rs crates/lens-core/src/actor/runloop.rs
git commit -m "feat(actor): TranscriptRewritten{ordinal} on in-place re-fire — complete the disk-write signal (T-2 §3.4)"
```

---

## Task 6: read-only `TranscriptReader` + transactional ranged read

The replica does **all** disk reads on a read-only handle that runs **no** DDL/migration/meta writes (unlike `SqliteTranscriptStore::open`) and sets a bounded `busy_timeout` (WAL readers still see `SQLITE_BUSY`; the default handler is null). One **transactional** primitive returns `Vec<(ordinal, Item)>` **plus the snapshot watermark** in a single transaction — closing the two-snapshot race between `load_items` (no ordinals) and `store_frontier` (separate query).

**Files:**
- Create/Modify: `persist/transcript.rs` (new `SqliteTranscriptReader` + `open_read_only`), `persist/mod.rs` (new `TranscriptReader` trait + `ReadRange` + `RangeRead`), `persist/db.rs` (a read-only opener that skips DDL/meta), `persist/map.rs` (reuse `row_to_item` — the private decoder shared with the write store)
- Test: inline in `transcript.rs` (concurrent writer/reader; range shapes; watermark-in-txn; busy tolerance)

**Interfaces:**
- Produces:
  ```rust
  pub trait TranscriptReader {
      fn mode(&self) -> StoreMode;
      /// One transaction: the requested (ordinal, Item) rows + the snapshot's non-provisional watermark.
      fn read_range(&self, range: ReadRange) -> Result<RangeRead>;
  }
  pub enum ReadRange {
      /// Full resident set (T-2 baseline + reconcile re-read). T-2b adds a windowed variant.
      All,
      /// Live growth: ordinals in (after, through].
      Delta { after: i64, through: i64 },
      /// Single ordinal (TranscriptRewritten re-read).
      One { ordinal: i64 },
  }
  pub struct RangeRead { pub rows: Vec<(i64, Item)>, pub skipped: Vec<SkippedRow>, pub watermark: Option<i64> }
  // open: pub fn open_read_only(path: &Path, busy_timeout: Duration) -> Result<SqliteTranscriptReader>;
  ```
- Consumed by the reader worker (Task 10). The write `TranscriptStore` shares **only** the private `row_to_item` decoder — no public write-side addition.

- [ ] **Step 1: Write the failing read-only-opener test** — opening read-only on a WAL file does not migrate/stamp, refuses writes, and reads existing rows.

```rust
#[test]
fn read_only_opener_reads_without_ddl_or_meta_write() {
    let d = tempdir().unwrap();
    let s = store(d.path()); // write store creates + seeds
    s.upsert_item(0, &item("item_a", None, "a"), false).unwrap();
    let r = SqliteTranscriptReader::open_read_only(&d.path().join("conv_1.db"), Duration::from_millis(200)).unwrap();
    let read = r.read_range(ReadRange::All).unwrap();
    assert_eq!(read.rows.len(), 1);
    assert_eq!(read.rows[0].0, 0); // ordinal
    assert_eq!(read.rows[0].1.id.as_str(), "item_a");
}
```

- [ ] **Step 2: Write the failing transactional-watermark test** — a `Delta` read returns the ordinals in range AND the watermark from the same snapshot.

```rust
#[test]
fn read_range_delta_returns_ordinals_and_watermark_in_one_txn() {
    let d = tempdir().unwrap();
    let s = store(d.path());
    s.upsert_item(0, &item("a", None, "a"), false).unwrap();
    s.upsert_item(1, &item("b", None, "b"), false).unwrap();
    s.upsert_item(2, &function_call("fc_live", "c1"), true).unwrap(); // provisional — not the watermark
    let r = SqliteTranscriptReader::open_read_only(&d.path().join("conv_1.db"), Duration::from_millis(200)).unwrap();
    let read = r.read_range(ReadRange::Delta { after: 0, through: 2 }).unwrap();
    let ords: Vec<i64> = read.rows.iter().map(|(o, _)| *o).collect();
    assert_eq!(ords, vec![1, 2]); // (0, 2]
    assert_eq!(read.watermark, Some(1)); // newest non-provisional
}
```

- [ ] **Step 3: Write the failing concurrent-reader test** — the reader tolerates the writer under a busy timeout (WAL): a read during an open write transaction still succeeds.

```rust
#[test]
fn reader_tolerates_concurrent_writer_wal() {
    let d = tempdir().unwrap();
    let path = d.path().join("conv_1.db");
    let s = store(d.path());
    s.upsert_item(0, &item("a", None, "a"), false).unwrap();
    let r = SqliteTranscriptReader::open_read_only(&path, Duration::from_millis(500)).unwrap();
    // writer holds a txn; reader (WAL snapshot) still reads committed rows.
    let read = r.read_range(ReadRange::All).unwrap();
    assert_eq!(read.rows.len(), 1);
}
```

- [ ] **Step 4: Run all three, verify fail** — `cargo test -p lens-core persist::transcript 2>&1 | tail -15` → FAIL (types absent).

- [ ] **Step 5: Add the read-only opener** in `db.rs` — `open_db_read_only(path, busy_timeout) -> Result<Connection>` using `Connection::open_with_flags(path, SQLITE_OPEN_READ_ONLY)`, `conn.busy_timeout(busy_timeout)`, **no** `CREATE TABLE`, **no** DDL, **no** WAL flip (the writer already set WAL). Return `StoreMode::ReadWrite`-equivalent read capability (reads only) — model as `ReadOnlyDegraded`-style but usable for reads; or a dedicated `Reader` marker. Keep it simple: the reader has no `mode` mutation concern — return `StoreMode::ReadOnlyDegraded` semantically (writes impossible).

- [ ] **Step 6: Implement `read_range`** — one `unchecked_transaction()`; select `ordinal, item_id, live_seq, kind, payload, agent, depth, created_at, response_id FROM items` with the range's `WHERE` (`All` = none; `Delta` = `ordinal > ?after AND ordinal <= ?through`; `One` = `ordinal = ?`), `ORDER BY ordinal`, decode each via `row_to_item` inside `collect_skipping` (skip corrupt rows → `RangeRead.skipped`); in the **same** txn, query the watermark (`SELECT ordinal FROM items WHERE provisional=0 ORDER BY ordinal DESC LIMIT 1`); commit. Pair each decoded `Item` with its `ordinal` column (note: `row_to_item`'s current column order in `load_items` starts at `item_id`; the reader's SELECT prepends `ordinal`, so read `ordinal` from column 0 and pass columns `1..` to the decoder — adjust the decoder call or add an `ordinal`-aware wrapper).

- [ ] **Step 7: Run to green** — `cargo test -p lens-core persist:: 2>&1 | tail -15`.

- [ ] **Step 8: Gate + cross-family review, commit**

Run: `cargo run -p xtask -- gate 2>&1 | tail -10`
Cross-family: `codex exec -s read-only "Review SqliteTranscriptReader: does open_read_only truly avoid ALL writes (no CREATE TABLE meta, no DDL, no WAL flip, no schema stamp)? Is read_range's rows+watermark genuinely one snapshot (single transaction)? Is the busy_timeout set so a WAL SQLITE_BUSY is retried not null-handled? Does the ordinal/decoder column offset line up with load_items' row_to_item?" < /dev/null`

```bash
git add crates/lens-core/src/persist
git commit -m "feat(persist): read-only TranscriptReader + transactional ranged read (ordinal,Item)+watermark (T-2 §3.3)"
```

---

# Phase B — lens-ui consumer machinery (Tasks 7–15)

Phase B consumes Phase A. Tasks 7–8 are the plumbing (reader factory + fan-out); 9–14 build the replica, reader worker, rowsource, and the crux staged-finalize; 13 wires the surface; 15 closes edge states + perf. gpui glue leans on `spikes/transcript-virtual` as the reference — **the spike is the source, not re-derived here**; where a step lifts spike code, cite the spike file.

---

## Task 7: `FleetStore` retains the per-session reader factory + reconcile epoch

`FleetStore` discards `data_dir`/connection context after `spawn_live_session` (it's a bare `data_dir: &Path` param; `wake_session` is a no-op seam for exactly this reason). The replica needs a **reader factory** to open its read-only handle, and the **current reconcile epoch** so a replica installed mid-reconcile is seeded correctly (Imp-4) rather than missing the falling edge.

**Files:**
- Modify: `crates/lens-ui/src/fleet/store.rs` (add `readers`/`reconcile_epochs` maps + a `ReaderFactory`; retain them in `spawn_live_session`)
- Test: inline in `fleet/store.rs` (a spawned session retains a factory that opens a reader over the real `{id}.db`)

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone)]
  pub struct ReaderFactory { data_dir: PathBuf, conn_id: ConnectionId, session_id: SessionId }
  impl ReaderFactory {
      pub fn open(&self, busy_timeout: Duration) -> Result<SqliteTranscriptReader, PersistError>; // {data_dir}/{session_id}.db
  }
  // FleetStore gains:
  reader_factories: HashMap<SessionId, ReaderFactory>,
  reconcile_epochs: HashMap<SessionId, ReconcileEpoch>, // {epoch: u64, in_flight: bool}, bumped on TransportChanged
  ```
- Consumed by `focus_session` (Task 9) to install the replica; by the poller (Task 8) to seed the epoch.

- [ ] **Step 1: Write the failing retention test** — after `spawn_live_session`, `store.reader_factory(&id)` opens a reader that reads the committed baseline.

```rust
#[gpui::test]
async fn spawned_session_retains_reader_factory(cx: &mut TestAppContext) {
    // build a FleetStore in live mode over a tempdir data_dir; spawn a session; commit an item via the actor;
    let factory = store.read_with(cx, |s, _| s.reader_factory(&id).cloned()).unwrap();
    let reader = factory.open(Duration::from_millis(200)).unwrap();
    assert!(reader.read_range(ReadRange::All).is_ok());
}
```

- [ ] **Step 2: Run, verify fail** (`reader_factory` / `reader_factories` absent).

- [ ] **Step 3: Add the maps + `ReaderFactory`**; in `spawn_live_session`, after `live::open_stores(data_dir, &conn.id, &session_id)`, build and insert `ReaderFactory { data_dir: data_dir.to_path_buf(), conn_id: conn.id.clone(), session_id: session_id.clone() }` and `reconcile_epochs.insert(session_id, ReconcileEpoch::default())`.

- [ ] **Step 4: Add accessors** `reader_factory(&SessionId) -> Option<&ReaderFactory>` and `reconcile_epoch(&SessionId) -> ReconcileEpoch`.

- [ ] **Step 5: Run to green + gate** — `cargo test -p lens-ui fleet::store 2>&1 | tail -12 && cargo run -p xtask -- gate 2>&1 | tail -8`.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-ui/src/fleet/store.rs
git commit -m "feat(fleet): retain per-session reader factory + reconcile epoch (T-2 §3.2)"
```

---

## Task 8: Poller fan-out through `FleetStore::fold_session_feed` + route `reconcile_in_flight`

The poller today updates `card` directly (`card.fold_feed`) and discards `TransportChanged.reconcile_in_flight` with `..`. Reroute the drained **batch** through a `WeakEntity<FleetStore>` so `FleetStore::fold_session_feed(session_id, batch, cx)` fans `Summary`→card / `Detailed`→card-chrome **and** (when focused) the replica; and route `reconcile_in_flight` edges to the replica + the retained epoch. Batch routing lets the replica recognize *scratch-clear + watermark in one batch* as one finalize episode (§6).

**Files:**
- Modify: `fleet/poller.rs` (capture `WeakEntity<FleetStore>`; call `fold_session_feed` for the feed batch; forward `TransportChanged` to the store), `fleet/store.rs` (`fold_session_feed` + `apply_transport` methods)
- Test: inline in `fleet/store.rs` (one `Detailed` batch updates card chrome + focused replica; an unfocused session's batch never touches a replica; a `reconcile_in_flight` true→false bumps the epoch)

**Interfaces:**
- Produces:
  ```rust
  impl FleetStore {
      pub fn fold_session_feed(&mut self, id: &SessionId, batch: SmallVec<[ActorFeed; 8]>, cx: &mut Context<Self>);
      pub fn apply_transport(&mut self, id: &SessionId, transport: ActorTransport, reconcile_in_flight: bool, cx: &mut Context<Self>);
  }
  ```
- The poller is respawned with `(session_id, WeakEntity<FleetStore>, feed_rx, outcomes_rx, clock)` instead of `(card, …)`. `fold_session_feed` still calls `card.fold_feed` for chrome (unchanged behavior for the card path).

- [ ] **Step 1: Write the failing fan-out test** — a focused session's `Detailed` batch reaches both the card and the replica; an unfocused one reaches only the card.

- [ ] **Step 2: Write the failing epoch test** — `apply_transport(id, _, true)` then `apply_transport(id, _, false)` bumps `reconcile_epochs[id].epoch` and flips `in_flight` true→false (the edge the replica consumes for the reconcile re-read).

- [ ] **Step 3: Run, verify fail.**

- [ ] **Step 4: Add `fold_session_feed`** — drain the batch: for each `ActorFeed::Summary(u)` → `card.fold_summary`; `ActorFeed::Detailed(u)` → `card.fold_detailed(u.clone())` **and**, if `self.focused == Some(id)` and a replica exists, `replica.update(cx, |r, cx| r.fold_detailed(u, cx))`. Keep the whole batch so the replica sees co-arriving frames together (Task 9's fold is batch-aware for the finalize episode).

- [ ] **Step 5: Add `apply_transport`** — update the card overlay (existing behavior) **and** `reconcile_epochs.entry(id)`: on `in_flight` rising, bump `epoch`; on falling, set `in_flight=false` and, if a replica exists, `replica.update(cx, |r, cx| r.on_reconcile_epoch_settled(epoch, cx))` (Task 9).

- [ ] **Step 6: Rewire the poller** — `spawn_session_poller(session_id, store: WeakEntity<FleetStore>, feed_rx, outcomes_rx, clock, cx)`; in the feed arm, build the batch (existing coalescing loop) then `store.update(cx, |s, cx| s.fold_session_feed(&session_id, batch, cx))`; in the outcomes arm, forward `TransportChanged` via `s.apply_transport(...)` (keep `Parked` overlay handling). Update `spawn_live_session`'s poller call to pass `cx.entity().downgrade()` + `session_id`.

- [ ] **Step 7: Run to green + gate.** Re-run the existing poller/card tests — the Ready-decay timer + notify_count behavior must be preserved (the batch/coalescing loop is unchanged; only the routing target moves from `card` to `store`).

- [ ] **Step 8: Cross-family review + commit**

Cross-family: `codex exec -s read-only "Review the poller→FleetStore fan-out: does capturing WeakEntity<FleetStore> avoid the task↔entity cycle? Is the whole feed batch delivered atomically to the replica (finalize-episode recognition)? Does an unfocused session ever touch a replica? Is the Ready-decay timer path preserved?" < /dev/null`

```bash
git add crates/lens-ui/src/fleet
git commit -m "feat(fleet): batch fan-out via WeakEntity + route reconcile_in_flight to replica (T-2 §3.1/§9)"
```

---

## Task 9: `FocusedTranscript` replica skeleton + batch fold rules + live-section index

The store-owned replica: state + the documented per-frame fold rules + the `live_section_start` re-projection index (decision 3). No rendering yet (Task 13) and no staged finalize yet (Task 12) — this task establishes the state machine and drives the reader worker (Task 10) from folds, verified with an **in-memory** `TranscriptReader`.

**Files:**
- Create: `crates/lens-ui/src/focused/mod.rs` (`FocusedTranscript`), `crates/lens-ui/src/lib.rs` (`pub mod focused;`)
- Modify: `fleet/store.rs` (`focus_session` installs the replica **before** `Promote`, drops on `Demote`)
- Test: inline in `focused/mod.rs` (fold-rule units with a fake reader)

**Interfaces:**
- Produces:
  ```rust
  pub struct FocusedTranscript {
      items: Vec<Item>,                 // resident finalized transcript (ordinal-keyed order)
      scratch: Arc<StreamScratch>,
      active_response: Option<ResponseId>,
      last_rendered_ordinal: i64,       // forward-delta cursor
      live_section_start: usize,        // index in `items` of the live turn's first item (decision 3)
      rows: RowStore,                   // Task 11 (empty stub here)
      pending_finalize: HashMap<AccId, RowPresentation>, // Task 12 (empty here)
      markers: Vec<Marker>,             // Task 14 (empty here)
      focus_generation: u64,
      reader: ReaderWorkerHandle,       // Task 10
      session_id: SessionId,
      baseline_epoch: u64,              // epoch at replica creation (Imp-4 seed)
  }
  impl FocusedTranscript {
      pub fn new(factory: ReaderFactory, seed_epoch: ReconcileEpoch, focus_generation: u64, cx: &mut Context<Self>) -> Self; // enqueues baseline read
      pub fn fold_detailed(&mut self, u: StreamUpdate, cx: &mut Context<Self>);
      pub fn on_reconcile_epoch_settled(&mut self, epoch: u64, cx: &mut Context<Self>);
      pub fn apply_read(&mut self, gen: u64, read: RangeRead, cx: &mut Context<Self>); // called by the worker on the UI thread
  }
  ```
- Consumes: `RangeRead`/`ReadRange`/`SqliteTranscriptReader` (Task 6), `ReaderFactory`/`ReconcileEpoch` (Task 7), `Retired`/`RetireDisposition`/`TranscriptRewritten`/`Reconnected{gap}` (Tasks 2/4/5), the T-1 projection (Task 1).

- [ ] **Step 1: Write the failing fold-rule tests** (fake reader, one per row):
  - `Rebased(scalars)` → updates status/title/active-response scalars **only**; never clears `items` (baseline read was enqueued at `new`).
  - `TranscriptAdvanced{ord>last}` → enqueues exactly one `Delta{after:last_rendered, through:ord}` read; a stale `ord <= last_rendered` is a no-op on the forward path.
  - `TranscriptRewritten{ord}` → enqueues exactly one `One{ord}` read.
  - `ActiveResponseChanged(r)` → sets `active_response`, recomputes `live_section_start` = index of first `items[i].ctx.response_id == r` (or `items.len()` if none resident yet).
  - `ScratchChanged(s)` → stores scratch; marks the live section dirty (re-project `&items[live_section_start..]` + scratch — asserted via a projection-count probe, cheap).
  - `on_reconcile_epoch_settled(epoch)` where `epoch` overlapped `baseline_epoch` → enqueues a `ReadRange::All` reconcile re-read.

- [ ] **Step 2: Run, verify fail.**

- [ ] **Step 3: Implement the state + fold match** — exhaustive over the `StreamUpdate` variants the replica consumes; unrelated variants are no-ops. `new` stamps `baseline_epoch` from `seed_epoch` and enqueues `ReadRange::All` at generation `focus_generation`.

- [ ] **Step 4: Implement `live_section_start` recompute** — on `ActiveResponseChanged` and after each `apply_read` that changed `items`: scan for the first index whose `ctx.response_id == active_response`; the live turn's items are contiguous to the tail, so the re-projection slice is `&items[live_section_start..]`.

- [ ] **Step 5: `apply_read`** — drop if `gen != focus_generation` (stale-read gate); else id-keyed upsert of `read.rows` into `items` (by `item.id`, positioned by `ordinal`), advance `last_rendered_ordinal` to `read.watermark` where applicable, recompute `live_section_start`, trigger a projection (Task 11/12 own the row materialization; here assert `items` mutated correctly).

- [ ] **Step 6: Install/drop in `focus_session`** — before `send_command(&id, Promote)`, create the replica via `cx.new(|cx| FocusedTranscript::new(factory, epoch, focus_generation, cx))`, store it in a `focused_replica: Option<(SessionId, Entity<FocusedTranscript>)>` field, bump `focus_generation`. On `Demote`/blur, drop it.

- [ ] **Step 7: Run to green + gate.** Add the **stale-read gating** test (a read completing after a focus switch is dropped) and the **focus-mid-reconcile** test (Imp-4: a replica created while `in_flight` is already true still re-reads on epoch settle, seeded from `baseline_epoch`).

- [ ] **Step 8: Cross-family review + commit**

Cross-family: `codex exec -s read-only "Review the replica fold rules: is Rebased scalars-only (never clears items)? Is TranscriptAdvanced idempotent for ord<=last_rendered on the forward path while below-watermark changes still reach the reconcile/rewrite paths? Is live_section_start correct when the live turn's items aren't yet resident, and are they truly contiguous at the tail? Is the stale-read gate airtight across focus switches?" < /dev/null`

```bash
git add crates/lens-ui/src/focused/mod.rs crates/lens-ui/src/lib.rs crates/lens-ui/src/fleet/store.rs
git commit -m "feat(focused): FocusedTranscript replica skeleton + batch fold rules + live-section index (T-2 §5)"
```

---

## Task 10: dedicated reader worker — serialized, focus-gated, coalescing, typed errors

All disk reads run on **one** background reader worker (serialized — never independent spawns) holding a `SqliteTranscriptReader`. A **bounded latest-target queue**: forward-watermark targets **coalesce** (only the highest pending `through` survives); reconcile re-reads take **priority**; the baseline is the first target. Each result is `Retryable` (`SQLITE_BUSY` past the busy timeout → re-enqueue with backoff) or `Fatal` (surface an error state, not a silent blank). Results apply on the UI thread, dropped if `gen != focus_generation`.

**Files:**
- Create: `crates/lens-ui/src/focused/reader.rs`
- Test: inline in `reader.rs` (coalescing, priority, retry, gating) using a fake reader with injectable `SQLITE_BUSY`/`Fatal`.

**Interfaces:**
- Produces:
  ```rust
  pub struct ReaderWorkerHandle { tx: Sender<ReadTarget>, }
  pub struct ReadTarget { pub range: ReadRange, pub gen: u64, pub priority: Priority } // Baseline|Delta|Reconcile|Rewrite
  pub enum ReadOutcome { Ok(RangeRead), Retryable, Fatal(String) }
  // The worker calls back onto the replica: replica.update(cx, |r, cx| r.apply_read(gen, read, cx))
  // or r.on_read_error(gen, ReaderError, cx) for Fatal.
  impl ReaderWorkerHandle {
      pub fn enqueue(&self, target: ReadTarget);
      pub fn spawn(factory: ReaderFactory, replica: WeakEntity<FocusedTranscript>, cx: &mut Context<FocusedTranscript>) -> Self;
  }
  ```
- Consumed by the replica (Task 9). Bounded channel per `.agents/rust-ui.md:7`. A `Mutex` (if any) is never locked on the gpui thread; read transactions are short.

- [ ] **Step 1: Write failing tests** — (a) two `Delta` targets enqueued back-to-back coalesce to the higher `through`; (b) a `Reconcile`/`Baseline` target jumps ahead of a pending `Delta`; (c) a `Retryable` result re-enqueues the same target (bounded backoff); (d) a `Fatal` result calls `on_read_error`, not `apply_read`; (e) a result whose `gen` mismatches is dropped.

- [ ] **Step 2: Run, verify fail.**

- [ ] **Step 3: Implement the worker** — `cx.background_spawn` a serialized loop over a bounded `Receiver<ReadTarget>` with a single-slot latest-target-per-priority coalescer; open the reader once via `factory.open(busy_timeout)`; per target run `read_range`; map `PersistError::Sqlite(SQLITE_BUSY)` → `Retryable`, other errors → `Fatal`; hop to the UI thread via `replica.update(cx, …)` to apply/error. (Model on the Board B-4a off-thread `run_op` pattern, memory [[board-b4a-design]] — `Arc<Mutex>` + `background_spawn`, conn pinned to the worker.)

- [ ] **Step 4: Run to green + gate.**

- [ ] **Step 5: Cross-family review + commit**

Cross-family: `codex exec -s read-only "Review the reader worker: is it truly serialized (one connection, no concurrent reads)? Does coalescing ever drop a reconcile/rewrite in favor of a delta (it must not — those are correctness reads)? Is a Retryable backoff bounded (no hot spin)? Can a Fatal ever present as a silent blank instead of an error state?" < /dev/null`

```bash
git add crates/lens-ui/src/focused/reader.rs
git commit -m "feat(focused): serialized reader worker — coalescing, priority, Retryable/Fatal, focus-gated (T-2 §3.3)"
```

---

## Task 11: production `RowStore` — owned presentations, id-keyed retained entities

Lift `spikes/transcript-virtual/src/rowsource.rs` to production: an id-keyed `RowStore` of retained `Entity<RowState>` holding **owned** `RowPresentation` (kind + the text/flags a stub renderer needs — not the whole `Item`), with `ListState::splice`/`reset` discipline. `project_*` returns borrowing `Vec<ViewBlock<'a>>`; the `list()` closure is `'static` and cannot capture a borrow, so T-2 **materializes** each block into an owned presentation (§6 — "zero clone in the render tree" is dropped as unworkable).

**Files:**
- Create: `crates/lens-ui/src/focused/rowsource.rs` (lifted + adapted from the spike)
- Test: inline (materialize a `Vec<ViewBlock>` → owned rows; id-keyed upsert preserves `EntityId` on content change; splice-not-reset on live change)

**Interfaces:**
- Produces:
  ```rust
  pub enum RowId { Section(ResponseId, u32 /*run_index*/), Work(ItemId), Sibling(ItemId), StreamTail(AccId), Marker(u64) }
  #[derive(Clone)]
  pub struct RowPresentation { pub kind: RowKind, pub text: String, /* stub flags: collapsed, height hints */ }
  pub enum RowKind { SectionChip, SectionRail, WorkChild, Message, UserMessage, ResourceEvent, StreamingReasoning, StreamingMessage, ReconnectBreak }
  pub struct RowStore { order: Vec<RowId>, entities: HashMap<RowId, Entity<RowState>> }
  impl RowStore {
      pub fn upsert(&mut self, id: RowId, pres: RowPresentation, cx: &mut App) -> UpsertEffect; // Inserted|UpdatedInPlace{entity_id_stable}
      pub fn splice_into(&self, list: &ListState, effect_range: Range<usize>, count: usize);
      pub fn materialize(blocks: &[ViewBlock], into: &mut RowStore, cx: &mut App); // owned copy per block
  }
  ```
- Consumed by Task 12 (staged finalize) + Task 13 (render). `RowId::Work`/`Sibling` use item ids; `Section` uses `(response_id, run_index)` (finalize-stable); `StreamTail` uses `acc_id`; `Marker` uses the synthetic seq (Task 14). The **collapse flag is looked up by `response_id`** (Task 12) so all runs of a turn fold together.

- [ ] **Step 1: Write the failing materialize test** — a `Vec<ViewBlock>` (a per-run section + a sibling message + a streaming tail) materializes to the expected `order` and `RowKind`s, with `Section` → one `SectionChip`/`SectionRail` + one `WorkChild` per child, and the `RowId::Section` carrying its `run_index`.

- [ ] **Step 2: Write the failing id-stability test** — upserting the same `RowId::Work` with changed `text` returns `UpdatedInPlace { entity_id_stable: true }` (same `EntityId`), never a new entity.

- [ ] **Step 3: Run, verify fail.**

- [ ] **Step 4: Lift the spike `RowStore`/`RowState`** into `focused/rowsource.rs`; add `RowId`/`RowKind`/`RowPresentation` as above; implement `materialize` (owned copy of each block's minimal presentation — `ViewBlock::Item` → text stub by `ItemKind`; `ToolSpan`/`WorkSection`/streaming → their stubs) and `upsert` (id-keyed: present → mutate the entity's `RowState.presentation` in place; absent → `cx.new`).

- [ ] **Step 5: Implement `splice`/`reset` discipline** — `splice` for live order/count/height change; `reset` reserved for initial mount / new-session (Task 13). Every content-mutated row whose height may change is height-invalidated (`list.splice(index..index+1, 1)`, per spike `invalidate_row_height`).

- [ ] **Step 6: Run to green + gate.**

- [ ] **Step 7: Cross-family review + commit**

Cross-family: `codex exec -s read-only "Review RowStore: does id-keyed upsert preserve EntityId on content change (no remount)? Is every ViewBlock variant materialized to an owned presentation (no borrow escapes into the 'static closure)? Is reset used ONLY for mount/new-session (splice everywhere else)?" < /dev/null`

```bash
git add crates/lens-ui/src/focused/rowsource.rs
git commit -m "feat(focused): production RowStore — owned presentations, id-keyed retained entities (T-2 §6)"
```

---

## Task 12: two-level retained-entity model + staged finalize (the crux)

Every turn's work is a `WorkSection` **from birth** — a Level-1 entity keyed by `response_id` owning Level-2 work-child entities (reasoning id / tool `call_id`). Expanded-vs-collapsed is a **derived render flag** (live **or** latest-settled-until-next-user per §4), not a structural difference — so finalize flips the flag and swaps streaming children in place; **no entity is created or destroyed**. `Retired` (Task 4) drives the flash-free handoff: `Finalizing { item_id }` stages the child's last presentation keyed by `acc_id` until the disk row for `item_id` arrives, then swaps in place; `Discarded` drops with no ghost.

**Files:**
- Modify: `focused/mod.rs` (wire `Retired` into `pending_finalize`; the collapse-flag derivation; the two-path projection), `focused/rowsource.rs` (section/child entity nesting; expand/collapse splice)
- Test: **MANDATORY real-window harness** (`Application::new().run()`, `harness=false`) — the streaming→finalize identity/paint test — plus in-memory units for discard/collapse-timing/write-failure.

**Interfaces:**
- Consumes: `Retired`/`RetireDisposition` (Task 4), `acc_id` (Task 3), `RowStore` (Task 11), the reader worker (Task 10).
- Produces: a derived **per-`response_id`** collapse flag `expanded(response_id) = (response_id == active_response) || is_latest_settled_before_next_user(response_id)`, applied to **all** `(response_id, run_index)` sections of that turn so they fold/unfold together; `pending_finalize: HashMap<AccId, RowPresentation>` staging.

- [ ] **Step 1: Write the MANDATORY real-window finalize test** — a streaming→finalize sequence asserts, **on every intervening paint**: the message row's `EntityId` is unchanged, the row is **present** (row count never dips), content is correct, and `ListOffset` (bottom-pin) holds. Endpoint-only `EntityId` equality is insufficient ([[terminal-realwindow-harness-pitfalls]]). Structure: drive `ScratchChanged` (streaming tail) → `Retired{Finalizing{item_id}}` → the forward-delta read delivering the committed row → assert across paints.

- [ ] **Step 2: Write the discard test** — a `Discarded` (reconnect gap **and** each terminal Failed/Incomplete/Cancelled) drops the streaming child, leaves **no** permanent `pending_finalize` entry, no ghost row.

- [ ] **Step 3: Write the collapse-timing test** (New-Imp-4) — the **latest settled** turn stays **expanded** until the **next user message**, then collapses; older settled turns are collapsed; the live turn is expanded. Assert against §4, not "collapsed iff not active".

- [ ] **Step 4: Write the write-failure-recovery test** — a persistence failure after `Finalizing` (reader `Fatal` path) does not orphan the staged row — it is retried/surfaced, never a permanent ghost.

- [ ] **Step 5: Run all, verify fail.**

- [ ] **Step 6: Implement the two-level nesting in `RowStore`** — a `Section` row owns an ordered child list; `list()` flattening: collapsed section = **one** chip row; expanded = a rail row + one row per child; each sibling = one row. Expand/collapse = **`splice`** the child rows in/out (never `reset`).

- [ ] **Step 7: Implement the derived per-`response_id` collapse flag** — track the latest-settled/next-user boundary in the replica (the highest settled `response_id` with no user message after it stays expanded); recompute on `ActiveResponseChanged` + on a new user message landing. The flag is keyed by `response_id` and applied to **every** `(response_id, run_index)` section of that turn, so a turn's interleaved runs collapse in lockstep (a per-run collapse-timing test asserts two runs of one response fold together on the next user message).

- [ ] **Step 8: Wire `Retired`** — `Finalizing { item_id }` → move the streaming child's presentation into `pending_finalize[acc_id]`, keep rendering it under its section; on the forward-delta read delivering `item_id`'s disk row, swap the section child in place (same `Entity`) and remove `pending_finalize[acc_id]`. `Discarded` → drop the streaming child immediately.

- [ ] **Step 9: Two-path projection (D-1)** — baseline/reconcile: full staged pipeline over the resident set, cache each settled section's owned presentation; `ScratchChanged`: re-project **only** `&items[live_section_start..]` + scratch (per-response). After any reconcile re-read, **invalidate all settled caches** (coarse; §3.4).

- [ ] **Step 10: Run to green** — real-window test first (it's the proof); then units. `cargo test -p lens-ui focused 2>&1 | tail -20`.

- [ ] **Step 11: Opus + cross-family review of the crux** — this is the load-bearing architecture. Opus subagent reviews the staged-finalize + retained-entity invariants; codex reviews the diff.

Opus: `Agent(subagent_type: claude)` — "Review the staged-finalize implementation in crates/lens-ui/src/focused: prove no entity is created/destroyed at finalize (flag flip only); prove pending_finalize can never permanently orphan a row (every Finalizing is matched by a disk-row swap OR a recovery path); prove Discarded leaves no ghost; prove the derived collapse flag matches §4 collapse timing exactly."
Cross-family: `codex exec -s read-only "Review focused staged finalize: is the streaming→finalize handoff structurally flash-free (EntityId stable, row count never dips)? Is the acc_id→item_id swap correct for the unkeyed-message case (local_id != acc_id)? Does the coarse cache invalidation blow ALL settled caches after reconcile?" < /dev/null`

- [ ] **Step 12: Commit**

```bash
git add crates/lens-ui/src/focused
git commit -m "feat(focused): two-level retained-entity model + staged finalize — structurally flash-free (T-2 §6/D-3)"
```

---

## Task 13: `focused/view.rs` — `list()` surface, four §16 scroll contracts, stub renderers, mount

The gpui `Render` surface: native `list()` / `ListState` / `ListAlignment::Bottom`, the four §16 scroll contracts, a stub renderer per `RowKind`, and the `focused_transcript_tab(replica, cx) -> TabHandle` mounted into `#chat-slot`.

**Files:**
- Create: `crates/lens-ui/src/focused/view.rs`
- Modify: `slot/mod.rs` (`focused_transcript_tab`), `board/mod.rs:266` (mount in `#chat-slot`, replacing `"chat"`)
- Test: **real-window** scroll-contract tests + a "every RowKind renders / none panics" smoke test.

**Interfaces:**
- Produces: `pub fn focused_transcript_tab(replica: Entity<FocusedTranscript>, cx: &mut App) -> TabHandle;` (`ContentTab` untouched).
- Consumes: `RowStore`/`RowId`/`RowKind` (Task 11), the replica (Task 9/12).

- [ ] **Step 1: Write the failing scroll tests** (real-window): (1) stick-to-bottom while pinned, scroll-up pauses auto-follow; (2) `↓ N new · jump to latest` pill shows only when scrolled up, `N` = rows appended since pause, click → bottom + resume; (3) scroll anchoring on finalize / above-viewport height change (no jump — id-keyed upsert + staged finalize keep the anchor); (4) new-session open lands at bottom (the one `reset` site). Plus **paused-scroll-not-yanked** (New-Crit-3): a live change while scrolled up uses `splice`, does **not** jump to bottom.

- [ ] **Step 2: Write the "every RowKind renders, none panics/dropped" smoke test** — materialize one of each `ViewBlock` variant (stubs for T-3/T-4-owned) and assert `item_count` matches and no panic.

- [ ] **Step 3: Run, verify fail.**

- [ ] **Step 4: Implement `render`** — lift the spike's `list(list_state, move |ix, window, app| entity.update(...))` `'static` closure (spike `app.rs:662`); the closure captures the replica entity + reads the owned presentation via `RowStore`. `set_scroll_handler` drives Following/Paused (spike pattern). Stub renderers per `RowKind` (a `div` with the presentation text + a kind tag — T-3/T-4 replace these).

- [ ] **Step 5: Implement the pill + follow-mode** — from the spike's `FollowMode`; `N` counts rows appended since pause.

- [ ] **Step 6: `focused_transcript_tab` + mount** — build a `TabHandle { view: replica-render.into(), title: "chat".into(), focus_handle }`; in `board/mod.rs:266`, replace `.child("chat")` with `.child(self.chat_tab.view.clone())` where `chat_tab` is populated from the focused replica when `ShellMode::Focused`.

- [ ] **Step 7: Run to green + gate.** Run the app to confirm on-device (`/run` or the wave measure rig): focusing mounts a transcript; blurring tears it down.

- [ ] **Step 8: Cross-family review + commit**

Cross-family: `codex exec -s read-only "Review focused/view.rs: is the list() render closure genuinely 'static (captures entity + owned order, no borrow)? Do all four scroll contracts hold — especially reset ONLY for new-session (splice for live changes so a paused reader isn't yanked)? Does every RowKind have a non-panicking stub?" < /dev/null`

```bash
git add crates/lens-ui/src/focused/view.rs crates/lens-ui/src/slot/mod.rs crates/lens-ui/src/board/mod.rs
git commit -m "feat(focused): list() surface + four §16 scroll contracts + mount in #chat-slot (T-2 §7)"
```

---

## Task 14: `ReconnectBreak` marker + temporal anchor

A UI-only synthetic marker (no backing `Item`, never persisted) injected into the row order on `Reconnected { gap != Some(0) }`, carrying a `{ after_ordinal, seq }` temporal anchor so **every full reprojection re-inserts it deterministically** at the same position (Imp-5) rather than floating to the tail or vanishing.

**Files:**
- Modify: `focused/mod.rs` (`Reconnected{gap}` fold → inject marker), `focused/rowsource.rs` (`RowId::Marker` merged into order by `after_ordinal`)
- Test: inline (marker-position persistence across N reprojections; `Some(0)`→no marker; `None`/`Some(N>0)`→exactly one; gap-while-unfocused→none).

**Interfaces:**
- Produces: `pub struct Marker { pub after_ordinal: i64, pub seq: u64, pub kind: MarkerKind }` (`MarkerKind::ReconnectBreak`); a monotonic `marker_seq` on the replica.

- [ ] **Step 1: Write the failing tests** — (a) `Reconnected{gap:Some(0)}` injects nothing; `None` and `Some(3)` each inject exactly one; (b) a marker survives N full reprojections at its `after_ordinal` anchor (neither floats to the tail nor vanishes); (c) a gap while unfocused (no detailed frames) produces none (narrowed criterion).

- [ ] **Step 2: Run, verify fail.**

- [ ] **Step 3: Implement injection** — on `fold_detailed(StreamUpdate::Reconnected { gap })` with `gap != Some(0)`, push `Marker { after_ordinal: last_rendered_ordinal, seq: next_marker_seq(), kind: ReconnectBreak }`.

- [ ] **Step 4: Implement deterministic re-insertion** — in `materialize`/order-build, merge markers by `after_ordinal` (a synthetic `RowId::Marker(seq)` outside the item-id space) so reprojection places each after its anchored ordinal.

- [ ] **Step 5: Run to green + gate + cross-family review + commit**

Cross-family: `codex exec -s read-only "Review ReconnectBreak: does the {after_ordinal,seq} anchor survive full reprojection deterministically (no float-to-tail, no vanish)? Is the gap!=Some(0) condition matched to the reducer (None and Some(N>0) inject, Some(0) does not)? Never persisted as an Item?" < /dev/null`

```bash
git add crates/lens-ui/src/focused
git commit -m "feat(focused): ReconnectBreak marker with {after_ordinal,seq} temporal anchor (T-2 §3.5)"
```

---

## Task 15: edge states (`syncing…` debounce) + perf gate

Close the §9 disk-paint→reconcile edge with a **debounced `syncing…`** indicator (shows only if reconcile takes >~150 ms, driven off `reconcile_in_flight`), and prove the frame budget with a **release-mode benchmark**: steady-state re-project + upsert + paint at realistic transcript sizes stays within budget, and per-frame cost is O(visible), not O(resident).

**Files:**
- Modify: `focused/mod.rs` + `focused/view.rs` (debounced indicator), `crates/lens-ui/benches/` (or the existing bench harness) for the perf gate
- Test: inline debounce test (shows only >150 ms; cancels if reconcile finishes sooner) + a release benchmark.

**Interfaces:**
- Consumes: `reconcile_in_flight` (Task 8), the two-path projection (Task 12).

- [ ] **Step 1: Write the failing debounce test** — `reconcile_in_flight=true` for 100 ms then false → **no** `syncing…`; true for 200 ms → `syncing…` shows then clears. 150 ms debounce + cancellation.

- [ ] **Step 2: Write the perf benchmark** — build a realistic resident transcript (via the `bench_push_message` seam, [[benchmark-validity-audit-2026-07]]); measure a steady-state `ScratchChanged` re-project + upsert + a paint pass; assert it stays within `.agents/performance.md` 8.3 ms / 11.1 ms and that the per-delta work is bounded by the **live section** size, not resident size (compare a small-resident vs large-resident run — the delta cost must not scale with resident count).

- [ ] **Step 3: Run, verify fail / establish baseline.**

- [ ] **Step 4: Implement the debounced indicator** — a 150 ms `cx.background_executor().timer` armed on the `reconcile_in_flight` rising edge, cancelled on the falling edge (spike/poller timer pattern).

- [ ] **Step 5: Run debounce test to green; run the benchmark in release** — `cargo bench -p lens-ui focused 2>&1 | tail -20` (or the project's bench invocation); confirm the O(visible) property holds.

- [ ] **Step 6: Full gate + final synthesis review + commit**

Run: `cargo run -p xtask -- gate 2>&1 | tail -15`
Opus synthesis: `Agent(subagent_type: claude)` — "Final review of the whole T-2 branch diff against docs/specs/2026-07-21-transcript-t2-focused-view-scaffold-design.md §12 success criteria: every criterion has evidence; no T-2b/T-3/T-4 scope leaked in; ContentTab left inert; the three disk signals cover every write; the staged finalize is structurally flash-free."

```bash
git add crates/lens-ui
git commit -m "feat(focused): syncing… debounce + release perf gate (O(visible) steady state) (T-2 §9/§10)"
```

---

## Self-Review (against the spec)

**Spec coverage** — every §12 success criterion maps to a task:
- Mount before Promote / teardown on blur → Task 9 (install in `focus_session` pre-`Promote`) + Task 13 (mount/teardown).
- Finalized-from-disk + live-tail + staged finalize (no absent frame / no remount) → Tasks 6, 9, 12 (mandatory real-window test).
- Below-watermark changes (rewrite/rekey/delete) reflected → Task 5 (`TranscriptRewritten`), Task 9 (reconcile re-read), Task 12 (id-keyed reconcile).
- Serialized read-only busy-timeout transactional `(ordinal,Item)+watermark` focus-gated → Tasks 6, 10.
- Four §16 scroll contracts + pill + invalidation → Task 13.
- Frame budget / O(visible) → Task 15.
- `ReconnectBreak` on gap≠Some(0), never persisted → Tasks 2, 14.
- Every `ViewBlock` variant renders (stubs), none dropped/panics → Tasks 1, 11, 13.
- `xtask gate` green → every task.
- No windowing/paging/bounded-reconcile leak; `ContentTab` inert → scope fences (Global Constraints) + Task 13.

**Type consistency** — `AccId` (Task 3) is the `Retired.acc_id` (Task 4), the `RowId::StreamTail` key (Task 11), and the `pending_finalize` key (Task 12). `ReadRange`/`RangeRead` (Task 6) are the reader worker's target/result (Task 10) and the replica's `apply_read` input (Task 9). `TranscriptRewritten { ordinal }` (Task 5) → `ReadRange::One { ordinal }` (Task 9). `StreamingReasoning { response_id, acc }` (Task 1) → the replica's live-section placement (Task 12).

**Placeholder scan** — the lens-core tasks (1–6) carry exact code (verified verbatim 2026-07-22); the lens-ui tasks (7–15) carry exact signatures + test intents and cite the spike (`spikes/transcript-virtual`) as the lift source for gpui glue rather than re-deriving unrun `list()`/`ListState` code (calibration: [[plan-detail-vs-delegation-calibration]] — over-specifying unrun gpui glue is false precision; the spike is the proven reference).

**Known deliberate gap (user-approved):** none open — the re-fire signal is closed by Task 5 (`TranscriptRewritten`), not left to reconcile.

