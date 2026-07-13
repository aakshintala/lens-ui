# Handoff — state-model P3-1 DONE; P3-2 next (2026-07-09)

## Where we are

**P3-1 (actor foundation) is DONE, merged to `main`, and pushed** (origin in sync).
- 12 commits `1096a8c..f7c9a64` + STATUS commit `70fc52d`.
- Gate green: lens-client 139 / lens-core 89 / lens-store 6 tests; `cargo fmt --check`
  + `cargo clippy --all-targets` (`all = deny`) clean on the three production crates
  (spikes deliberately don't set `lints.workspace`, so their warnings don't gate).
- Full task-by-task ledger + every review finding + deferred minors:
  **`.superpowers/sdd/progress.md`** (search "STATE-MODEL P3-1").
- Durable learnings: memory **`state-model-p3-1-actor-foundation`** (the
  value-carrying-completeness gotcha + gpui-0.2.2 actor facts). STATUS entry dated
  2026-07-09 at the top of `docs/STATUS.md`.

### What P3-1 delivered (map for reading the code)
- **`crates/lens-client/src/stream/reader.rs`** — reader channel is now
  `crossbeam_channel::bounded` + `EventStream::receiver()` (single-consumer; the
  actor `Select`s over it). D13.
- **`crates/lens-core/src/reduce/update.rs`** — `StreamUpdate` is value-carrying
  (each delta carries its reduced value) + `Rebased(Box<SessionState>)`. `items:
  Vec<Arc<Item>>` (shared bodies). D8/D9.
- **`crates/lens-core/src/actor/`** (gpui-free) — `runloop.rs` = the `ActiveSession`
  actor: `crossbeam::Select`(events, commands) → greedy-drain → `reduce` →
  `persist_write_through` → `coalesce` → emit; `summary.rs` = `SummaryUpdate` +
  `from_state`. `SessionCommand { Stop, Promote, Demote }`. Dual-mode
  `Detailed|Summary` (`spawn_actor` = Detailed wrapper, `spawn_actor_dual` = explicit).
  D10/D13.
- **`crates/lens-store/`** (NEW crate, gpui) — `SessionStore` `Entity<SessionState>`
  replica; free fn `apply` = pure copy-assignment (exhaustive match, O(1) ~102ns);
  `spawn_apply_bridge` = foreground drain (`recv().await` + greedy `try_recv` batch →
  one `entity.update` + `cx.notify()`). D7/D8.

### P3-1 deferred items P3-2 should sweep up (from the whole-branch review)
- **M2 (touches D16/D18 command semantics — fix in P3-2):** `Demote` sent to a
  `spawn_actor()` (Detailed-only, its summary receiver was dropped) → the next event's
  `summaries.send_blocking(...)` hits a closed channel → `Err` → **actor thread dies
  silently**. Guard it when you build the command surface (either reject `Demote` on a
  Detailed-only handle, or make a missing summary consumer non-fatal).
- **M1 (Minor, self-heals):** `stream.current_agent`/`stream.turn` are written in a
  few arms (`AgentChanged` fold, `Completed` turn-bump, `OutputItemDone` completed-
  FunctionCall) without a guaranteed `ScratchChanged`, so live-preview attribution can
  lag until the next scratch delta. Snapshot case is already covered by I1.
- **Nit:** `CollaborationModeChanged` / `TitleChanged` variants have no producer
  (reserved; `title` only mutates via snapshot). Add a clarifying comment or wire a
  producer if P3-2 introduces one.

## P3-2 scope — command semantics (D16 + D18)

Authoritative spec: **`docs/specs/2026-07-08-state-model-engine-design.md`
§2.2 (D16, D18), §7 (commands), §13.1**. No P3-2 plan exists yet.

### D16 — optimistic-send reconcile keyed by server ack ids (spec lines 210–231)
- The actor gains a **send command** (extend `SessionCommand`): POST via
  `lens-client Sessions::send_event`, which returns `SendEventAck`
  (`lens-client/src/sessions.rs:697`) that **already carries `pending_id` (native
  bypass) and `item_id` (persisted store id)**.
- **Restructure `PendingUserMessage`** (`crates/lens-core/src/domain/controls.rs:71`):
  keep `pending_id: String` (Lens-local, addresses the optimistic bubble for
  rollback/UI); ADD `server_pending_id: Option<String>` (← `SendEventAck.pending_id`)
  and `store_item_id: Option<String>` (← `SendEventAck.item_id`), both stamped at
  POST-return.
- **Reconcile precedence** (same for live `consumed` stream and reconnect replay):
  (1) `server_pending_id` present → native by-id (`cleared_pending_id` /
  snapshot `pending_inputs[].pending_id`); (2) `store_item_id` present → non-native
  by-id (replayed `GET /items` item whose `id ==` it → drop the bubble); (3) ack empty
  → §4 P3(b) content/ordinal match (defensive floor). Sits inside the tail-bounded
  reconcile scope (bubbles are always at the tail — see D12 / the large-transcript
  spike memory).
- **LIVE-VERIFY RIDER (do this early):** confirm the POST ack actually populates
  `pending_id`/`item_id` at runtime — `#[serde(default)]` masks an empty body as
  `None`, and there is **no POST-ack body in the golden corpus**. If confirmed, (1)/(2)
  are the common path and (3) is defensive-only; if not, (3) carries it. Needs a live
  omnigent (`installing-omnigent-from-source` skill).

### D18 — §13.1 error mapping splits into two path-keyed tables (spec lines 260–282)
- **Table A — terminal `Disconnected{reason}` (stream path) → actor lifecycle:**
  `Unauthorized` / `SessionFailed` / `RetriesExhausted` → **park** (close stream, keep
  actor + state resident, await re-auth/user-retry); `Forbidden` → **stop** + remove
  from registry; `NotFound` → **stop** + local read-only tombstone. A **parked session
  is NOT quiesced** (`transport != Connected`) so it won't auto-sleep. (`DisconnectReason`
  already carries the 5 variants — `lens-client stream/event.rs`.)
- **Table B — `ClientError` on command/REST → command outcome** (NOT stream teardown):
  `Server{status,body}` 5xx = transient/retry-eligible, other-4xx = denied; `ThreadSpawn`
  = fatal at stream-open; drop the design's `Ws` row (no such variant); `Network` on
  `send` → **roll back the optimistic bubble per D16**. → also a **design-doc §13.1
  amendment** (§7.1).
- Introduce the **actor introspection ring** (fire-and-forget outcomes, never blocks
  the emit — persist errors are currently swallowed with `let _ =`; D18 gives them a home).

### Note the ordering vs P3-3
- D17 quiesce/sleep/wake (spec lines 232–259) and D11 byte-window eviction are **P3-3**,
  NOT P3-2. But `is_quiesced` cares that a **parked** session (`transport != Connected`)
  never auto-sleeps — so the D18 park/stop split (P3-2) sets up a flag
  (`reconcile_in_flight`, actor-owned, never-persisted) that P3-3's `is_quiesced()`
  consumes. Keep the transport/park state P3-2 introduces clean for P3-3.
- P3-3 also has a hard external dep flagged in the Task 0 spike: lifting a **blocking
  `GET /items` tail-pagination** path (deferred from lens-client 3b-2b).

## Recommended first move next session
1. `superpowers:grilling` or straight to `superpowers:writing-plans` for P3-2 (the
   spec decisions D16/D18 are already grilled/locked — a plan may be enough).
2. Do the **D16 live-verify rider first** (spin omnigent, POST an event, dump the ack
   bytes) — it decides whether reconcile precedence (1)/(2) is the common path or (3)
   carries it. Cheap and de-risks the whole plan.
3. Same execution shape as P3-1: subagent-driven, composer-2.5 build per task, Opus
   cross-family review at seams (the temporal send/reconcile path is a review seam).
   Watch the value-carrying-completeness rule (memory) for any new
   `PendingUserMessage`/state fields.
