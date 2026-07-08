# State-model engine — implementation spec

**Date:** 2026-07-08
**Component:** the per-session state-model engine (`lens-core` + `lens-store`)
**Consumes:** `lens-client` (typed omnigent client, feature-complete on `main`)
**Design source of truth:** `docs/design/app-architecture-and-state-model.md`
(LOCKED sections §2, §3, §4, §6, §7, §8, §13). This spec does **not** re-decide
that architecture — it records how to *build* it, phased, with the per-phase
implementation plans deferred.

---

## 1. Scope

Builds the complete state-model machinery for **one** `(connection, session)`:

```
EventStream ─▶ reduce(&mut SessionState) ─▶ canonical state
                 ├─▶ SessionPersistence write-through (SQLite)
                 └─▶ StreamUpdate ─▶ SessionStore replica (gpui) ─▶ notify
UI action  ─▶ SessionCommand ─▶ ActiveSession actor
```

**In scope** (per-session engine): §2 domain types, §3 lifecycle transitions,
§4 pure reducer + render transforms, §6 persistence, §7 command flow, §8
`ActiveSession` actor + `SessionStore` replica + the two-direction seam, §13.1
error→state mapping.

**Out of scope** — the app-level orchestration that manages *many* engines,
carved into their own specs (each builds ON this one):

- §5/§9 multi-connection `AppState`/`ConnectionApp`/registry/navigation → the
  **immediate next spec**. This spec exposes the seam (a `SessionHandle`-shaped
  handle + a coarse-update apply path) but does not spec the registry internals.
- §10 cross-session list poll → with §9.
- §11 Bridge router, §12.3 Concierge → separate features (own consumer surfaces).

Rationale for the cut is in §7 Deferred below and the design discussion: the
per-session engine is the load-bearing, highest-risk, independently-verifiable
unit — it is exactly what `lens-client` was hardened for. §9/§10 wrap engine
types and would inherit the engine's contract-stability risk if specced before
the engine contract is proven against real bytes.

---

## 2. Decisions

- **D1 — Two crates.** `lens-core` (framework-neutral: domain, reducer,
  transforms, persistence, `ActiveSession` actor, seam types) + `lens-store`
  (gpui: `SessionStore = Entity<SessionState>` replica + the `cx.spawn` drain
  bridge). Localizes the design's §14 *three* framework touch-points to one
  small crate and keeps the pure engine testable without pulling gpui into every
  test. The actor is "plain off-thread Rust, not a gpui entity" (§14), so it
  lives in `lens-core`.
- **D2 — Scope = per-session engine.** §9 registry / §10 poll = **named seam
  only**, next spec. Bridge/Concierge deferred.
- **D3 — Single-writer, re-stated from LOCKED §8.** For each Active session,
  canonical `SessionState` is owned and `&mut`-mutated by exactly one
  `ActiveSession` actor off the foreground thread; every mutation flows through
  `reduce()` in that actor's run-loop and nowhere else. `SessionStore` is a
  read-only foreground replica: it applies self-contained `StreamUpdate`s and
  notifies, never reduces, never originates state. Not re-litigated here (memory
  `state-model-single-writer-decision`).
- **D4 — Persistence.** `SessionPersistence` trait, SQLite v1 impl, portable
  (jsonb-mappable) schema per LOCKED §6.2. Backing-store swap is behind the trait.
- **D5 — Blocking OS thread, no tokio** on the core data path — matches
  `lens-client` D2 (the typed client's `EventStream` is a blocking reader).
- **D6 — `StreamUpdate` frozen at P1 exit.** The `StreamUpdate` enum (the
  reducer's output, the seam the upper layers hang on) is defined *empirically*
  by the P1 reducer against the golden SSE corpus and frozen when P1 lands green.
  Upper-layer plans (P2/P3) are written against the frozen contract, not a guess
  — this is the phasing-risk mitigation the whole cut is built around.
- **D7 — Walking skeleton = P3 task 1.** The off-thread→foreground handoff
  (blocking OS thread → channel → gpui `Entity` → `cx.notify`) is proven by a
  minimal end-to-end slice before the real reducer volume lands in P3 — no
  separate throwaway spike (the pattern is well-trodden in gpui/Zed; the risk is
  our specific blocking-thread↔executor shape + backpressure, which the skeleton
  exercises).

---

## 3. Workspace layout

```
crates/
  lens-client/     # existing — typed omnigent client
  lens-core/       # NEW — gpui-free; depends on lens-client
    src/
      domain/      # §2: SessionState, Item/ItemKind, BlockContext, ids, Usage/Cost, StreamScratch
      reduce/      # §4: reduce(), scratch accumulation, folds, transforms (§4.3)
      persist/     # §6: SessionPersistence trait + SQLite impl + schema
      actor/       # §8/§7: ActiveSession, SessionCommand, command semantics
      lib.rs       # StreamUpdate / SessionCommand seam types re-exported
  lens-store/      # NEW — gpui; depends on gpui + lens-core
    src/
      lib.rs       # SessionStore (Entity<SessionState> replica) + cx.spawn drain bridge
  xtask/           # existing
```

`lens-core` has **no gpui dependency** — the framework touch-points are all in
`lens-store`. Branded ids reuse `lens-client`'s (`ConnectionId`, `SessionId`,
…) and add engine-local ones (`ItemId`); a session's engine identity is the
composite `(ConnectionId, SessionId)` (persistence PK), but the *registry* that
holds many of them is out of scope (D2).

---

## 4. Build order (each phase lands independently, green)

Strict dependency order **P0 → P1 → P2 → P3** (the actor write-throughs to
persistence, so persistence precedes the actor).

### P0 — Domain types (`lens-core/domain`, §2)
Branded ids, `SessionState`, `Item` + `ItemKind` enum (the §6.2 `kind` set:
message | function_call | function_call_output | reasoning | native_tool |
compaction | slash_command | terminal_command | resource_event | agent_changed),
`BlockContext { agent, depth, turn }`, `Usage`/`Cost`/`ErrorInfo`,
`StreamScratch` (§4.2). Pure data + serde (payloads jsonb-mappable per §6.2).
No logic. **Gate:** serde round-trips; fmt · clippy · tests.

### P1 — Pure reducer + render transforms (`lens-core/reduce`, §4) — *contract-proving*
`reduce(&mut SessionState, &ServerStreamEvent) -> SmallVec<[StreamUpdate; 2]>`:
text accumulation (`OutputTextDelta`→`MessageAcc`→finalized `Message` on
`ResponseCompleted`); tool pairing by `call_id`; reasoning bracketing (open →
delta → synthetic `ReasoningClosed`); `BlockContext` attribution stamped at item
creation; identity/ordering/**dedup by `id`** (persisted items carry no
`sequence_number`; `seq` is an SSE-only overlap hint); session-field folds
(status/usage/todos/model/model_options/reasoning_effort/collaboration_mode/
skills/elicitation/child_session/presence/sandbox_status/terminal_pending/
agent_changed); `AgentChanged` item insertion (synthesize `from` from prior
state); the `SnapshotRestored` fold (bootstrap + reconnect chrome — **scalar
restore only, no transcript side-effects**). Plus §4.3 render transforms
(`hide_reasoning`, `flatten_sub_agents`, `merge_text_for_display`,
`only_agent`/`by_depth`, `with_agent_changed_markers`) as pure fns over
`&[Item]`.

**P1 also owns normalization** (flagged from Plan 4, memory
`plan4-pre-consumer-hardening`): the two status vocabularies (`SessionStatusValue`
6-val live vs `SessionStatus` 3-val snapshot) and the two usage representations
are normalized here into `SessionState`'s canonical fields.

No threads, no gpui, no SQLite. **TDD against the golden SSE corpus**
(`docs/spikes/captures/2026-06-26-sse/` + `…-live-recapture/`). **Gate:** reducer
is deterministic/replayable (same event sequence → same state); fmt · clippy ·
tests. **`StreamUpdate` is frozen when this phase lands (D6).**

### P2 — Persistence (`lens-core/persist`, §6)
`SessionPersistence` trait + SQLite v1 impl over the §6.2 schema
(connections/sessions/items/cost_samples/meta). Write-through upsert by
`(connection_id, session_id, item_id)`; session-field fold into `sessions`;
reconcile-by-`id` on wake (disk may lag the server); `meta.schema_version`
migration gate (unknown future version → read-only-degraded, never corrupted).
In-progress `StreamScratch` is RAM-only, never persisted. **Gate:** temp-db
write-through + reconcile tests; schema_version gating test; fmt · clippy.

### P3 — Actor + store + commands (`lens-core/actor` + `lens-store`, §8/§7/§13.1)
**Task 1 = walking skeleton (D7):** one fake event → `reduce` → `StreamUpdate`
over a bounded channel → `SessionStore` replica applies → `cx.notify` → observed
on the foreground. Proves the blocking-thread↔`cx.spawn` handoff + backpressure
shape end-to-end.

Then: `ActiveSession` OS-thread actor driving `lens-client`'s `EventStream`
(reduce → persist write-through → emit `StreamUpdate`); `SessionStore` replica
(`StreamUpdate::apply` = cheap assignment/insert only, no parse/reduce/IO on the
foreground) + `cx.observe` granularity; `SessionCommand` inbound channel with the
§7 command semantics — **send** (optimistic actor-owned `pending_user`, FIFO
reconcile on `session.input.consumed`, rollback on POST failure), interrupt,
compact, approve, stop_session, fork, switch_agent; bootstrap + reconnect wiring
(the actor consumes the crate-synthetic `Reconnecting`/`Reconnected`/
`SnapshotRestored`/`Disconnected` lifecycle from `lens-client` §7); §13.1
`ClientError`→app-state mapping; bounded-channel backpressure + delta coalescing
(drain all pending `StreamUpdate`s before one `cx.notify`).

**Gate:** scripted-mock actor tests (reuse the `lens-client` `Reopen`-style seam
for deterministic reconnect/bootstrap without a server) + the walking-skeleton
integration; **no foreground blocking** (all I/O off-thread — AGENTS.md
MANDATORY); fmt · clippy · tests.

---

## 5. Local verification

- **Per phase:** `cargo test -p lens-core` (P0–P2) / `-p lens-store` (P3),
  `cargo clippy --all-targets`, `cargo fmt --check`. `generated.rs` untouched.
- **P1 corpus:** the reducer replays the captured `.stream.sse` corpora; add a
  determinism test (replay twice → identical `SessionState`).
- **P3 skeleton:** a gated integration example/harness exercising the full
  off-thread→foreground path.
- **Live:** a gated `--features live-tests` run driving one real session through
  the engine end-to-end is deferred to after P3 lands (needs a running pinned
  0.4.0 server; `installing-omnigent-from-source` skill). Unit + corpus coverage
  is the phase gate; live is confirmation.
- **Review:** cross-family diversity review at each phase seam (P1 reducer and P3
  actor are load-bearing); consolidate where cheap (`review-spend-policy`). P1 and
  P3 warrant an Opus synthesis pass given they set the seam contracts.
- **Perf:** benchmark-or-it's-not-done on the hot paths (AGENTS.md) — reducer
  throughput (corpus/frame budget) and `StreamUpdate::apply` cost. The 120fps /
  90fps-regression contract applies to the foreground apply path.

---

## 6. Seam contracts (what this engine exposes)

- **Up to the UI (§13.2):** `SessionStore` read/observe access — `items` (through
  §4.3 transforms), status/usage/model/todos/presence/cost/sandbox scalars,
  `pending_elicitations`. Surfaces never receive `&mut SessionState`.
- **Down to `lens-client`:** the actor consumes `ServerStreamEvent` (incl. the
  synthetic lifecycle) and issues `SessionEventInput` commands + the REST
  fork/switch-agent endpoints.
- **To the future §9 registry (named, not specced):** a `SessionHandle`-shaped
  handle `{ SessionStore replica, Option<ActiveSessionHandle> }` and a
  coarse-summary apply path the list-poll can fold into a Slept session's store
  without an actor. This is the only forward hook this spec commits to.

---

## 7. Deferred / recorded (clean seams)

- **§9 registry / §10 poll** → immediate next spec (seam named in §6 above).
- **§11 Bridge, §12.3 Concierge** → separate features.
- **WS terminal byte stream** (§13.2) — direct from the typed-client WS client,
  not through the reducer; and `lens-client` has no `terminal.rs`/`tungstenite`
  yet (known build-order deferral). This engine carries only the
  `terminal.activity`/`terminal_pending` *notifications*.
- **Presence broadcast** (§12.1) — receive-only in v1.
- **`client_os_*` inbound bidirectional tools** (§7) — reserved extension.
- **Disk retention/pruning policy**, **auto-sleep threshold**, **poll cadence**
  (§15 open questions) — tune in the verification pass; the schema supports either.
- **`lens-client` residuals the reducer will eventually want** (memory
  `plan4-pre-consumer-hardening`): `last_task_error` type-ambiguity, minimal
  wrappers to grow with golden captures — resolve as the reducer consumes them.

---

## 8. Reversibility

- The two-crate split is cheap to collapse or re-cut — `lens-store` is thin.
- `StreamUpdate`/`SessionCommand` are the only cross-layer contracts; frozen at
  P1/P3 exits respectively, versionable if they must change.
- SQLite is behind `SessionPersistence` — a backing-store swap is a trait impl,
  not a rewrite (D4).
- Every phase lands green and independently, so a phase can be revised without
  unwinding its predecessors (the `lens-client` per-plan precedent).
