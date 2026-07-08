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
- **D4 — Persistence, two-tier (grilling revision).** `SessionPersistence` trait
  splits into a **`ControlStore`** (one `lens.db`: connections/sessions/
  cost_samples/meta) + a per-session **`TranscriptStore`** (one SQLite file per
  `(connection, session)`, actor-owned WAL connection — items only). `rusqlite`,
  portable (jsonb-mappable) schema per LOCKED §6.1/§6.2. The split makes actor
  writes contention-free by construction and retention/tombstone a file op;
  transcripts are re-fetchable via `GET /items` so isolating them is safe.
  Backing-store swap is behind the trait.
- **D5 — Blocking OS thread, no tokio** on the core data path — matches
  `lens-client` D2 (the typed client's `EventStream` is a blocking reader).
- **D6 — `StreamUpdate` drafted at P1, ratified at the P3 skeleton.** The
  `StreamUpdate` enum (the reducer's output per LOCKED §4.1 —
  `reduce() -> SmallVec<[StreamUpdate; 2]>`) is *drafted* empirically by the P1
  reducer against the golden SSE corpus: P1 freezes the **semantic deltas** (which
  events produce which state changes). Its **apply-side representation** may still
  refine when the P3 walking skeleton exercises `StreamUpdate::apply` on the gpui
  replica (and P2 write-through) — so the contract is *ratified* at the P3
  skeleton, not P1 exit. The de-risking still holds: P0/P1 land without waiting on
  P3, and P2/P3 plans are written against the P1 draft and reconciled at the
  skeleton. (Reviewer note, gpt-5.5: an early hard freeze would hide the apply/
  write-through coupling — hence draft-then-ratify, not freeze-at-P1.)
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
      persist/     # §6: SessionPersistence trait → ControlStore (lens.db) +
                   #     per-session TranscriptStore (rusqlite/WAL) + schema
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
Branded ids, `SessionState`, `Item` + `ItemKind` enum — the full LOCKED **§2.3**
union: message | function_call | function_call_output | reasoning | native_tool |
compaction | slash_command | terminal_command | **error** | resource_event |
agent_changed. `BlockContext { agent, depth, turn }` — **pure attribution**; the
durable "when" is `Item.created_at: i64` (epoch millis) on the item **envelope**,
stamped from an injected clock at reduce time (§2.3/§2.4, grilling revision — the
old `BlockContext.timestamp: f64` monotonic field is **dropped**: no consumer,
non-round-trippable). `Usage`/`Cost`/`ErrorInfo`/`PresenceViewer`, `StreamScratch`
(§4.2). Pure data + serde (payloads jsonb-mappable per §6.2). No logic.
**`presence` is RAM-only — never a persisted column** (§2.5/§6.2); carry it on
`SessionState` but exclude it from the P2 schema. **Gate:** serde round-trips;
`Item` round-trips through `created_at` (no monotonic value); fmt · clippy · tests.

> **Doc correction (resolved 2026-07-08):** the LOCKED §2.3 `ItemKind` includes
> `Error { source, code, message }` ("persisted error banner") but the §6.2
> schema `kind`-column comment originally omitted `error`. **Now fixed in the
> design §6.2** (the `items.kind` comment lists `error`); the P2 `item_kind`
> enum must include it.

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

No threads, no gpui, no SQLite. `reduce()` stays **pure** (§4.1 "does no I/O"), so
the reduce-time `Item.created_at` epoch (§2.3, grilling revision — replaces the
dropped `BlockContext.timestamp`) comes from an **injected clock** (a `Clock`
seam / passed-in instant), never a direct wall-clock read — otherwise replay is
non-deterministic.

**TDD against the golden SSE corpus** (`docs/spikes/captures/2026-06-26-sse/` +
`…-live-recapture/`) for the wire-event arms. The **reconnect/bootstrap arms are
crate-synthetic** (`Reconnecting`/`Reconnected{gap}`/`SnapshotRestored` are
lens-client §7 synthetics, NOT in the wire corpus) → tested with **hand-authored
synthetic-event fixtures**. Required cases: gap-cleared `StreamScratch` on
`Reconnected{gap != Some(0)}` (§4.2); `SnapshotRestored` scalar-only fold with
**no transcript side-effects** (§4.1); replayed `GET /items` dedup; duplicate-`id`
dedup against hydrated items. **Gate:** reducer deterministic/replayable under the
fixed clock (replay twice → identical `SessionState`); fmt · clippy · tests.
**The reducer-emitted `StreamUpdate` semantic deltas are drafted at this phase's
exit (D6); ratified at the P3 skeleton.**

### P2 — Persistence (`lens-core/persist`, §6) — *two-tier (grilling revision)*
`SessionPersistence` trait, **factored into two roles** over the §6.2 two-file
schema (§6.1):

- **`ControlStore`** (one `lens.db`): `connections`/`sessions`/`cost_samples`/
  `meta`. Session-field folds write here through a **single serialized
  control-plane writer** (low volume). `error` is in the `sessions.status` /
  transcript `item_kind` vocabularies as applicable (P0 doc correction, now fixed
  in §6.2).
- **`TranscriptStore`** (one SQLite file per `(connection_id, session_id)`,
  path `transcripts/<connection_id>/<conv_id>.db`): only that session's `items`.
  Write-through upsert **by `item_id`** into the file the **actor owns** (its own
  WAL write connection → zero cross-actor contention). The file's own `meta`
  carries `schema_version` + `(connection_id, session_id)` (self-describing).

**Impl:** `rusqlite` (blocking, matches D5) + WAL both tiers. `meta.schema_version`
migration gate in **each** file (unknown future version → read-only-degraded,
never corrupted). In-progress `StreamScratch` and `presence` are RAM-only, never
persisted (§2.5/§4.2). P2 exposes **load / upsert / reconcile primitives** on
`TranscriptStore` (reconcile keyed by item `id`, since disk may lag the server)
plus control-plane load/upsert; the **wake wiring that calls them is P3** (a
lifecycle/actor concern, §6.3). Retention/pruning/tombstone become **file ops**
on the transcript file (§15 open q; still deferred). **Gate:** temp-db
write-through + reconcile-primitive tests **on both stores**; per-file
schema_version gating test; open/close transcript file across the Active
lifecycle; fmt · clippy.

### P3 — Actor + store + commands (`lens-core/actor` + `lens-store`, §8/§7/§13.1)
**Task 1 = walking skeleton (D7):** one fake event → `reduce` → `StreamUpdate`
over a bounded channel → `SessionStore` replica applies → `cx.notify` → observed
on the foreground. Proves the blocking-thread↔`cx.spawn` handoff + backpressure
shape end-to-end.

Then, in three parts:

**(a) Actor run-loop.** `ActiveSession` on its OS thread **multiplexes two
inputs** — the `EventStream` receiver (the mpsc from `lens-client`'s reader
thread) and the `SessionCommand` receiver — via non-blocking select/`try_recv`,
so commands are serviced even while a turn streams (D5 is a blocking *thread*, not
a blocking *read* on one channel). Per event: reduce → persist write-through →
emit `StreamUpdate`. `SessionStore` replica applies (`StreamUpdate::apply` = cheap
assignment/insert only — no parse/reduce/IO on the foreground) with `cx.observe`
granularity; bounded-channel backpressure + delta coalescing (drain all pending
`StreamUpdate`s before one `cx.notify`).

**(b) Command semantics (§7).** `SessionCommand` inbound — **send** (optimistic
actor-owned `pending_user`, FIFO reconcile on `session.input.consumed`, rollback
on POST failure), interrupt, compact, approve, stop_session, fork, switch_agent;
bootstrap + reconnect wiring (the actor consumes the crate-synthetic
`Reconnecting`/`Reconnected`/`SnapshotRestored`/`Disconnected` lifecycle from
`lens-client` §7); §13.1 `ClientError`→app-state mapping.

> **Optimistic-send × reconnect reconcile (found in review — the one collision
> §7's FIFO leaves open).** §7's FIFO reconcile assumes client posts and
> `session.input.consumed` events stay ordered — but a reconnect **gap** can drop
> the `consumed` event, which otherwise duplicates the optimistic bubble (the
> replayed `GET /items` re-adds the message while the un-reconciled `pending_user`
> entry still shows) or orphans it. Rules:
> 1. **Do not clear `pending_user` on `Reconnected { gap != Some(0) }`** — unlike
>    `StreamScratch` (§4.2). It is user intent; resolve it against the reconnect
>    replay, never by dropping it.
> 2. **Reconcile `pending_user` against the reconnect replay, not only the live
>    `consumed` stream, and split by session type:**
>    - **Native-terminal** (claude-native / codex-native): the message is *not*
>      persisted at POST time, so trust the `SnapshotRestored` snapshot's
>      `pending_inputs` (`{pending_id, content}[]`) — present ⇒ still pending
>      (keep/re-hydrate); absent from both `pending_inputs` and the replayed items
>      ⇒ lost (re-send or drop-with-marker). Live clears use the consumed event's
>      `cleared_pending_id` **by id**.
>    - **Non-native**: the message *is* persisted at POST time, so match ordered
>      un-reconciled `pending_user` (role=user, `created_by == me`) against the
>      ordered trailing user-message items in the replay not already canonical;
>      content validates. Drop the optimistic bubble on match (kills the dup).
> 3. **No client-supplied correlation id exists on the message input** (only the
>    elicitation path has one) — do not design assuming one. Reconcile-repopulating
>    `pending_user` from `pending_inputs` is **not** a §4.1 transcript side-effect
>    (it pushes no canonical `Item`), but call it out so the `SnapshotRestored`
>    "scalar-only" wording isn't read as forbidding it.
>
> **P3 verification item (not spec-blocking):** confirm against a live native
> session whether `POST /events` **returns** the `pending_id`, letting Lens stamp
> the optimistic bubble at POST time for fully by-id native reconcile. Add to the
> P3 gate.

**(c) Session lifecycle mechanics (§3).** The engine owns the *mechanics*: an
`is_quiesced` predicate (strict — no open scratch / pending tools / pending user /
unreconciled reconnect / non-`idle` status / live terminal, §3.2), **sleep**
(flush persistence → best-effort `stop_session` → stop actor → drop heavy RAM,
§3.4/§6.3), **wake** (disk-paint input + fresh stream bootstrap → reconcile-by-id,
calling the P2 primitives), and `SessionLifecycle = Active | Slept | Deleted` +
tombstone state (§3.1). The **trigger/scheduler** that *fires* auto-sleep (the
~10min timer, the active-set) is the **§9 seam**, deferred — the engine exposes
`is_quiesced()` + `sleep()`/`wake()` for that scheduler to call.

**Gate:** scripted-mock actor tests (reuse the `lens-client` `Reopen`-style seam
for deterministic reconnect/bootstrap without a server) + the walking-skeleton
integration; a **command-interleaving matrix** — send/interrupt/stop exercised
while the stream is idle, running, and reconnecting; sleep/wake round-trip
(quiesce → sleep → flush asserted → wake → reconcile); **no foreground blocking**
(all I/O off-thread — AGENTS.md MANDATORY); fmt · clippy · tests.

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
  handle `{ SessionStore replica, Option<ActiveSessionHandle> }`, plus a
  **`SummaryUpdate`** — a type *distinct* from `StreamUpdate`, applied to a Slept
  session's store by the §10 list-poll **without an actor and without touching the
  reducer** (not a backdoor reduce path). Its invariant is committed here even
  though its allowed-field set is defined by the §9/§10 spec: it carries **only
  coarse card-summary fields** (status/title/last_total_tokens/host_id/
  needs-attention) and an Active session **ignores** it for any field its live
  stream owns (§10 — the stream is authoritative). This is the only forward hook
  this spec commits to.

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
- `StreamUpdate`/`SessionCommand` are the only cross-layer contracts *this spec
  owns* (`SummaryUpdate` belongs to the §9/§10 spec): `StreamUpdate` drafted at P1
  / ratified at the P3 skeleton (D6), `SessionCommand` at P3 — versionable if they
  must change.
- SQLite is behind `SessionPersistence` — a backing-store swap is a trait impl,
  not a rewrite (D4).
- Every phase lands green and independently, so a phase can be revised without
  unwinding its predecessors (the `lens-client` per-plan precedent).
