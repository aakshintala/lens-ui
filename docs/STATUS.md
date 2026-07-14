# Lens — STATUS

Lean, living status for the Lens design effort. Keep this doc **small**: it holds
the current forward-looking state only. **Full dated session entries live in
[`STATUS-ARCHIVE.md`](./STATUS-ARCHIVE.md)** — write each session's detail there
and roll older "Recent" pointers off this page as they age.

---

## Open threads & next up

- **▶ ACTIVE: shared terminal workstream (2026-07-14):** user-approved design
  in [`specs/2026-07-14-terminal-workstream-design.md`](./specs/2026-07-14-terminal-workstream-design.md).
  Deliver a standalone, renderable GPUI terminal tab backed by the real
  omnigent terminal WS attach path — ready for `lens-ui` to host, but not
  integrated into it. The initial plan gate is a thorough, attribution-aware
  audit of the narrow `gpui-ghostty`/Ghostty-VT port (FFI, GPUI version delta,
  reproducible Zig/source pin, behavior tests, and perf); native-harness
  rendered-stream/raw-TUI toggle remains a separate spec.

- **📋 SPEC-GAPS backlog (2026-07-13):** nine independent, un-specced/partial
  subsystems parked in [`SPEC-GAPS.md`](./SPEC-GAPS.md) — app release/signing/update,
  omnigent bundling, Lens observability, secrets lifecycle, TUI-native harness toggle,
  first-run product UX, settings surface, data lifecycle, multi-machine identity. Each
  gets its own spec→plan cycle; pick one to brainstorm. (TUI decision recorded: per-session
  rendered-stream ↔ raw-TUI toggle.)

- **✅ DONE (2026-07-12): state-model P3-3b EXECUTED + merged to `main`** (branch
  `state-model-p3-3b`, 16 commits, base 02d6d96; **push deferred — user call**). All 6 code/doc
  tasks landed via `subagent-driven-development` (composer-2.5 author, grok-4.5 seam review each,
  gpt-5.5/codex whole-branch final pass). Shipped: D24 park=actor-exit + D25/D26 user-gated
  reconnect (`FleetScheduler::reconnect` returns `Option<SessionStatus>`, exited-only reap); D27
  send 3-fate split (SendRejected deleted); `crates/lens-drive` headless driver; D30 scaffold-id
  dedup at persist (provisional flag, dual cursor, kind-scoped id→call_id fold, real ALTER
  migration) + C2/C3/C4; D28 held-landed reconcile. **lens-core 189 tests + lens-client 150,
  product-crate gates green.**
  - **⚠ AS-BUILT DEVIATION needing your eyes — D28 is UNIQUENESS-ONLY, temporal bound DROPPED.**
    Discovered mid-execution: omnigent `/items` rows carry **no per-item timestamp** (verified
    live), so the planned temporal screen is unimplementable. Path-2 silent drop-as-landed is
    KEPT (per-episode+frontier bounds the residual; always-SendLost would duplicate every
    successful landing). Accepted narrow residual → deferred client-message-id. Rationale +
    grok's independent agreement in memory `state-model-p3-3b-contract-gaps`; as-built doc in
    app-arch §13.1 "P3-3b as-built deltas".
  - **✅ LIVE-VERIFY RIDERS DONE (2026-07-12 follow-up):** driven live against omnigent 0.5.1 via
    `lens-drive` + a headless host daemon. **D24 park=exit** (server-stop → RetriesExhausted →
    `Parked{retries_exhausted}` → actor thread exits); **D26 live-status re-read** on reconnect
    (`{"kind":"reconnect","live_status":"idle"}`); **forward catch-up** + on-disk persistence
    (10 `/items` rows, provisional=0); **D27 SendPending** via a fault-injection proxy (503 on
    `POST …/events` → `SendPending`, held bubble retained, content preserved). **D30 tool-fold**
    captured live (real billable claude-sdk shell turn; two-id fold `fc_7ad9`→canonical `fc_9bb8`
    proven on disk) → golden fixtures `crates/lens-core/tests/fixtures/d30/` + **2 hermetic
    replay tests** (store-layer `d30_golden_reconcile_folds_tool_call` + end-to-end
    `d30_golden_end_to_end_attach_turn_reconnect_fold`). gpt-5.5 review caught a false-green +
    race in the first cut; fixed. **lens-core now 191 tests.**
  - **⚠ NEW FINDING (2026-07-12): D28 Ambiguity-B unstamped-hold can linger (narrow zombie).**
    The path-1 genuine-ambiguity branch (`reconcile.rs:109`, two identical-content
    `pending_inputs`) keeps a bubble unstamped "re-evaluated next reconnect" — but the LIVE path
    never resolves a both-ids-`None` held bubble (`reconcile_consumed` is content:None-inert;
    Signal B retains). So with no further reconnect it lingers as a visible duplicate. Narrow
    preconditions; SAME fix as the D28 residual (client-message-id). Detail in memory
    `state-model-p3-3b-contract-gaps` #5.
  - **DEFERRED (still open):** the omnigent **client-message-id** contract request (robust fix for
    both the D28 residual + Ambiguity-B + D30 heuristic content-match); `lens-ui` Bucket B viewport.
  - **✅ CLIPPY DEBT FIXED (2026-07-12):** the gpui spike crates are clippy-clean (`97dcabd`), and
    `AGENTS.md` now pins the gate to **workspace-wide** `cargo clippy --workspace --all-targets
    -- -D warnings` with a clean-before-push + fix-red-before-start rule (`1b75dd0`).

- **▶ NEXT: `lens-ui` (Bucket B viewport)** — first rendering consumer of the §13.2 seams
  (StreamUpdate/SessionCommand/ActorOutcome), incl. the arch-B composer-owns-durability draft.
  lens-drive is the headless precedent proving the seams are drivable.

- **lens-client benchmarks: DONE** (2026-06-27, composer-2.5 build + free codex
  cross-family review → 4 Important + 1 Nit, all applied). Closes the MANDATORY
  perf-doc gap. Two categories, split by gate-ability:
  - **Category 1 — criterion micro-benches** (`benches/sse_pipeline.rs`, `bench`
    feature, `bench_api` doc-hidden wrappers over the `pub(crate)` pipeline):
    `sse_frame_parse`, `event_decode`, `normalize`, `full_pipeline` over the golden
    SSE corpus. **Baseline (Apple Silicon, release):** full pipeline of a complete
    happy-path session = **~23µs** (~333 MiB/s); event_decode ~8µs/~1 GiB/s;
    normalize ~1µs/~7 GiB/s. The typing pass is ~0.3% of one 8.3ms frame — **I/O-bound
    confirmed by number**. Run: `cargo bench -p lens-client --features bench`.
  - **Category 2 — live overhead harness** (`tests/live_overhead.rs`, behind
    `live-tests`, informational/not-gated): REST p50/p90, send-ack→first-frame,
    inter-frame-gap-vs-parse ratio. Needs a live server + idle session.
  - Baseline detail + bench gotchas in memory `lens-client-benchmarks`. Plan:
    `docs/plans/2026-06-27-lens-client-benchmarks.md`. **Follow-up:** grow
    the golden corpus with longer sessions / a real Lens work pass (current ~22KB is
    thin); benches load any corpus file via `include_bytes!`. WS + state/render benches
    deferred to those paths. CI regression gate deferred (no CI yet).
  - **`crates/lens-capture` (new binary `lens_capture`):** spawns the harness, taps
    the SSE stream, writes a `.stream.sse` (+ snapshot/items) corpus on exit. Two modes:
    *default* (`lens_capture omnigent claude`) auto-detects the session by polling —
    best-effort subscribe-first, fine for corpus growth; *race-free*
    (`lens_capture --session conv_abc omnigent claude`) opens the stream before
    launching and auto-appends `--resume <id>`, so no events are missed. **Resolved:**
    session id IS the `conv_` id (`GET /v1/sessions` → `id: conv_...`), so the same
    value drives the stream subscription and `omnigent claude --resume`; harnesses take
    `--resume <conv_id>`, not create-with-id. Cross-family reviewed (codex). README has
    usage + limitations. **Next:** drive a real session through it to grow the corpus.
- **lens-client Foundation: DONE** (Plan 1 executed, 9 commits `043214e..f12050f`) —
  crate skeleton/error/ids/connection, typify codegen (`generated.rs`, 88 schemas,
  rustfmt-canonical via xtask), HTTP core + contract gate + ready-ladder handshake.
  16 serverless tests, clippy/fmt clean, live handshake green vs pinned `0.3.0.dev0`.
  Both seam reviews applied (gpt-5.5 codegen; gemini-3.1-pro final → 3 error-soundness
  fixes). Gotchas in memory `lens-client-foundation-gotchas`.
- **lens-client REST surface (Plans 2a–2e): DONE** (executed subagent-driven,
  composer-2.5 build + Opus per-task review + gpt-5.5 cross-family; 31 commits
  `b69e3d8..299ff72`). 2a=events write path; 2b=sessions read; 2c=lifecycle;
  2d=resources/terminals/comments; 2e=registries. 47 serverless tests, clippy
  `--all-targets` + fmt clean, `generated.rs` untouched. Live-verified vs pinned
  `0.3.0.dev0`: send_event, sessions read (get/list/child), create→patch→delete
  lifecycle. Reads are typed wrappers (private fields + getters, **no `Value` to
  consumers**); writes reuse generated request types. Cross-family review caught
  4 real shape bugs (hosts `{hosts}` envelope, directories `{object,path}` no-id,
  policy id nullable, resources `Value` leak) — all fixed.
  - **Deferred from 2a–2e (no consumer / need runner-backed live capture):**
    `Sessions::items()` (→ Plan 3 typed item union), list endpoints with unknown
    envelopes (`environments`, `terminals`, `changed_files`, `list_runners`), and
    ⚠ minimal wrappers (FileContent/ShellResult/FileResource/Host/Policy*) — grow
    getters + verify field names with golden captures when the state-model consumes
    them. Full rollup in `.superpowers/sdd/progress.md`.
- **Checkpoint RESOLVED (2026-06-25): build Plan 3 on `0.3.0.dev0` now.** No signal on
  `0.3.0` timing or whether it'll materially change the API — not worth idling a project
  with a live server to extract ground truth from. Treat dev0 instability as a *planning
  input*, not a blocker. **Plan 3 approach (decided):**
  1. **Golden-SSE capture spike — DONE (2026-06-26).** Captured 13 stream event types from
     real bytes vs pinned dev0 (happy-path item union, lifecycle, chrome, interrupt, error
     family). Found **3 undocumented events** (`session.input.consumed`,
     `session.changed_files.invalidated`, `session.interrupted`) to fold into §7, confirmed the
     seq-null-vs-int split + no-seq persisted items + full snapshot chrome. **Only claude-sdk
     works on this box** (codex binary quarantined; no `OPENAI_API_KEY`; claude-native is
     TUI-only; cursor needs `pip install omnigent[cursor]`+key) and it folds reasoning into
     `output_text` — so `reasoning_text.delta`/`reasoning_summary_text.delta` get schema-modeled
     (trivial `{delta,seq,type}`, flagged), plus compact/elicitation/sub-agent/terminal deferred
     to config-time. Findings: [`docs/spikes/2026-06-26-golden-sse-capture.md`](./spikes/2026-06-26-golden-sse-capture.md);
     raw corpus under `docs/spikes/captures/2026-06-26-sse/`; memory `plan3-sse-capture-findings`.
  2. **Split by stability** — reader-thread + reconnect plumbing is already de-risked (transport
     spike: subscribe-first + mid-stream-drop recovery), build confidently; gate only the
     semantic event union on the captures. **Plan 3a written** (2026-06-26,
     [`docs/plans/2026-06-26-lens-client-plan3a-sse-transport.md`](./plans/2026-06-26-lens-client-plan3a-sse-transport.md)):
     6 tasks — pure SSE frame parser, `ServerStreamEvent` taxonomy from bytes (incl. the 3
     capture corrections), reader-thread/`EventStream` bridge, schema-derived variants flagged.
     Normalization (§7a) + no-replay reconnect (§7) = Plan 3b; contract-drift CI = Plan 3c.
     - **Plan 3a EXECUTED & COMPLETE** (2026-06-26, subagent-driven: composer-2.5 build +
       per-task cross-family review gpt-5.5/gemini-3.1-pro; `67541a5..f0c5431`, 9 commits).
       85 lib tests + live `live_stream` pass vs warm claude-sdk session; fmt + clippy
       `--all-targets` clean. Final review (gpt-5.5) caught 3 real Important bugs — **split-UTF-8
       corruption** (per-chunk lossy decode → parser reworked to a `Vec<u8>` byte-buffer with a
       mid-codepoint TDD test), an `unwrap_err`/`unreachable!` **panic path** in `Sessions::stream`
       (now a panic-free `match check_status`), and **`Todos`/`SandboxStatus` dropping their
       payload** (now typed subsets, no `Value`) — all fixed + re-reviewed. The cross-family
       review earned its keep: composer's own "drop joins the reader" self-concern was factually
       wrong (JoinHandle drop detaches), caught by the reviewer.
     - **Deferred to Plan 3b** (3 Minors): redundant `serde(default)` on `Option`; `try_recv`
       idle-vs-closed liveness signal; reqwest read-timeout vs reader-thread leak on a hard hang.
     - **Plan 3b split by stability** (decided 2026-06-26): **3b-1 = pure §7a normalization**
       (no new endpoints, de-risked); **3b-2 = §7 no-replay reconnect** (pulls in typed
       `Sessions::items()` + the session snapshot read, both deferred from 2a–2e — folded into 3b-2).
     - **Plan 3b-1 EXECUTED & COMPLETE** (2026-06-26, subagent-driven: composer-2.5 build +
       per-task cross-family review gpt-5.5; `2f9a46e..3b39412`, 4 tasks + 1 fix wave;
       [`plan`](./plans/2026-06-26-lens-client-plan3b1-normalization.md)). A pure
       `stream::normalize::Normalizer` threaded into the reader thread: **`OutputItemDone` re-fire
       suppression** (key `(kind, call_id, status)` — **literal-duplicate only**, so the captured
       `function_call` `in_progress`→`completed` pair is preserved; §7a's "exactly once" wording
       relaxed per the golden bytes) + **synthetic `ReasoningClosed`** (close-trigger byte-grounded
       in `happy_path`; `full_text`/`summary_text` accumulation flagged NOT-byte-verified — claude-sdk
       folds reasoning into `output_text`). 103 lib tests, clippy `--all-targets`/fmt clean,
       `generated.rs` untouched. Final review (gpt-5.5) caught 1 real **Important**: the reader's
       `Err(_)` transport-error path shared EOF's `normalizer.flush()`, falsely emitting a synthetic
       `ReasoningClosed` on a mid-reasoning drop — fixed (`run` now generic over `io::Read`,
       `Err(_) => return`, no flush; +2 regression tests), re-reviewed clean. §7a doc updated to the
       pinned semantics. ⚠ `live_stream` NOT run this session (no server) — unit coverage only.
     - **Plan 3b-2 split (2026-06-26): 3b-2a = typed reconnect *reads* (DONE); 3b-2b = §7 reconnect
       *state machine* (next).** The reads (`Sessions::items()` + grown snapshot) were carved out as
       their own static/byte-grounded plan; the temporal state machine attaches at the reader's
       `Err(_) => return` seam (now reconnect-ready) and is gated on one design decision (below).
     - **Reconnect ownership RESOLVED (2026-06-26, Opus cross-doc).** The §7-vs-§11 ambiguity was
       decided by the consumer doc (app-arch state-model §1/§8: EventStream is "reconnect-safe", "the
       pump just keeps reading"): **the crate owns reconnect end-to-end, internally.** §7's "StreamUpdate"
       term was wrong (crate emits `ServerStreamEvent`; `StreamUpdate` is the state model's reduced
       output, §13); §11's "triggered by the state model's liveness watcher" was wrong (that's the §10
       cross-session poll for *non-active* sessions). **Designed the reconnect-lifecycle event surface:**
       three crate-synthetic `ServerStreamEvent` variants — `Reconnecting { attempt }` → `Reconnected
       { gap }` → terminal `Disconnected` — all on the existing mpsc channel (no `recv()` API break, no
       `ClientError::Disconnected`). Two 3b-2 seams recorded in §7: normalizer `seen_items` must reset on
       `Reconnected{gap≠0}`; lifecycle markers bypass normalization. Docs fixed (typed-client §7/§10/§11,
       app-arch §13.1, server-lifecycle §9.2). 3b-2 plan can be written from these.
     - **Plan 3b-2a EXECUTED & COMPLETE** (2026-06-26, subagent-driven: composer-2.5 build +
       one consolidated gpt-5.5 cross-family review; commits `1360819..2ff93c3`, 4 tasks + plan
       edit + 1 review fix; [`plan`](./plans/2026-06-26-lens-client-plan3b2a-reconnect-reads.md),
       [`handoff`](./handoffs/2026-06-26-lens-client-plan3b2a-execution.md)). The two typed reconnect
       *read* surfaces, byte-grounded from the golden captures: completed the `stream::Item` union
       (`ResourceEvent` variant, `id` on `Other`, total `Item::id()` accessor) so `/items` is
       reconcilable; `Sessions::items()` + typed `ItemList` envelope; `SessionSnapshot` grown with
       bucket-B scalars + `usage_by_model`/`skills`/embedded `items` (`ModelUsage`/`SkillRef`). 110 lib
       tests, clippy `--all-targets`/fmt clean, `generated.rs` untouched, no `Value` to consumers.
       Review caught 1 real bug the plan missed: `/items` carries item payload **flat** but the
       snapshot's embedded `items` **wrap it under a `data` envelope** → `de_items` now hoists `data`
       before `Item::from_value` (test hardened to assert typed payload; memory
       `plan3b2a-embedded-item-envelope`). **Deferred (byte-grounded gaps):** `last_task_error`
       (type-ambiguous null — sibling models it as a map), `todos`/`pending_elicitations`/`model_options`/
       `sandbox_status` (empty/null in the only capture). ⚠ `live_stream` NOT run (no server) — unit only.
     - **Plan 3b-2b design decision RESOLVED + plan WRITTEN (2026-06-26, Opus; commit `74c28fd`).**
       Chrome-restore ownership decided **A2**: the crate emits **one** synthetic
       `ServerStreamEvent::SnapshotRestored(SessionSnapshot)` (NOT consumer-applies-snapshot — B breaks
       the LOCKED state-model §1 boundary "does NOT own reconnect" + §4.1 single-writer; NOT per-field
       `session.*` replay — A1 injects a spurious `AgentChanged` transcript marker on every wake). ADR
       recorded in typed-client §7 (decision block + step 4/6 ordering `Reconnected`→`SnapshotRestored`
       →history + synthetic-markers-bypass-normalization seam) and app-arch §4.1 (reducer
       `SnapshotRestored` fold = scalar restore only, no transcript side-effects). **Plan written:**
       [`plan`](./plans/2026-06-26-lens-client-plan3b2b-reconnect-state-machine.md) — 7 TDD
       tasks (synthetic variants → `Normalizer::reset_seen_items` → frame seq-peek → `reconnect` module
       `Reopen`-trait/`HttpReopener`/backoff/items-replay → state machine in reader → wire
       `Sessions::stream` → docs). The `Reopen` trait makes the state machine unit-testable with a
       scripted mock (no server). Four plan-level decisions flagged for review + §7 reconciliation:
       `Disconnected{reason}` payload, `gap:None` v1 (no `Some(0)` proof), frame-level seq-dedup,
       single-page items replay.
     - **Plan 3b-2b EXECUTED & COMPLETE** (2026-06-26, subagent-driven: composer-2.5 build + Opus
       per-task review + one consolidated gpt-5.5 cross-family review; commits `3d4048b..6d4dde3`,
       6 code tasks + 1 review fix wave + xtask fmt housekeeping + docs;
       [`plan`](./plans/2026-06-26-lens-client-plan3b2b-reconnect-state-machine.md),
       [`handoff`](./handoffs/2026-06-26-lens-client-plan3b2b-execution.md)). The §7 no-replay
       reconnect state machine lives in `stream::reader`, generic over a `Reopen` capability
       (unit-testable with a scripted mock — no server). On a drop it backs off
       (`[100,200,400,800,1600,3000,3000]`ms), re-reads snapshot + `/items`, re-opens the live
       stream, and emits the synthetic lifecycle on the existing channel:
       `Reconnecting{attempt}` → `Reconnected{gap}` → `SnapshotRestored(SessionSnapshot)` →
       replayed `OutputItemDone` history → seq-deduped live tail; terminal `Disconnected{reason}`
       on give-up/stop. 119 lib tests (4 order-asserting reconnect tests + the 2 updated §7a tests
       + 1 review-regression test), clippy `--all-targets`/fmt clean, `generated.rs` untouched, no
       `Value` to consumers. **Cross-family review (gpt-5.5) caught 1 Critical** the author + green
       tests missed: `reconnect` opened the new body *before* fetching `/items`, so a retryable
       `/items` error dropped the already-opened no-replay body → fixed by making `open_stream` the
       last fallible call (`snapshot → items → open_stream`). Two user-decided review fixes:
       failed-status path emits `SnapshotRestored → Disconnected{SessionFailed}` (no `Reconnected`);
       `EventStream::spawn` returns `Result` (`ClientError::ThreadSpawn`, no panic). §7 reconciled
       with the 4 plan decisions + as-built (`gap:None` v1, frame-level seq-peek, single-page items,
       `DisconnectReason` table). **Deferred (flagged):** `gap==Some(0)` proof; `/items` pagination;
       gated live reconnect smoke test (no server-kill harness this session). ⚠ `live_stream` NOT
       run (no server) — unit coverage only.
  3. **Contract-drift CI (outstanding B6): DONE** (Plan 3c, 2026-06-26, subagent-driven:
     composer-2.5 build + Opus per-task review + one consolidated gpt-5.5 cross-family review;
     commits `087ef6f..8a7bb2e`, 5 tasks + 2 live-caught fixes + 1 review fix;
     [`plan`](./plans/2026-06-26-lens-client-plan3c-contract-drift.md)). The passive
     alarm, three layers by what each needs: **`xtask drift`** (`cargo run -p xtask -- drift` —
     semantic path-set + SSE discriminator/member-shape diff vs sibling pin, `/hooks/*`-excluded
     per ADR-0001; green vs identical sibling, red vs synthetic fixture); **offline `taxonomy_drift`
     test** (always-on `cargo test`: pinned openapi `ServerStreamEvent` mapping == `MODELED`(33)∪
     `DEFERRED`(14), disjoint — new upstream event fails with no server); **gated live checks**
     (`--features live-tests`): `live_taxonomy` (wire types modeled, or deferred-as-`Unknown`; a
     **modeled** type as `Unknown` is drift) + `live_reachability` (every consumed read endpoint
     reachable). **LIVE RUN EXECUTED this session** vs a real `0.3.0.dev0` server — **both gated
     tests green**, and the reachability sweep immediately **caught 2 real pre-existing bugs** the
     prior serverless plans missed: `HostObject` deserialized `id` from wire `id` (real key is
     `host_id`; `/v1/hosts` is openapi-untyped so live bytes are truth) and `SessionSnapshot`
     collections failed on the server's explicit `null`-for-empty (`labels`/`usage_by_model`/`skills`/
     `items` — `#[serde(default)]` covers missing, not `null`). The consolidated gpt-5.5 review
     caught 1 Important the author + green tests missed: `live_taxonomy` allowed `Unknown` for any
     *accounted* type, masking a **modeled** event degrading to `Unknown` on payload drift → fixed
     by the MODELED/DEFERRED split (only deferred types may be `Unknown`), re-verified live. 122 lib
     tests + 2 xtask tests, clippy `--all-targets`/fmt clean, `generated.rs` untouched, no `Value`
     to consumers. **CI surface = local `xtask` only** (design D3; no `.github/workflows` — drift
     needs the sibling checkout). **Deferred (flagged):** `xtask drift` member-shape diff is
     property-*names* only (deliberate scope bound); `ResourceList` live decode not exercised (no
     runner-bound session — `/v1/sessions/{id}/resources` returned a typed 409). **Plan 3 / B6 thread
     CLOSED.**
  - Plan 3b-2b is temporal/stateful (reconnect state machine), so **cross-family review stays
    mandatory** at the seams (`[[composer-delegation-profile]]`) — it caught the envelope bug in 3b-2a
    that author + green test both missed. (The earlier "composer is weak on temporal logic" claim was
    retracted as unsupported N=1.) Mind the Cursor-credit cost (`[[review-spend-policy]]`).
  - Now on branch `feat/lens-client-streaming` (off `main` @ `78fdaa3`).
- **Doc walkthrough complete** (all 11 design docs in `docs/design/` reviewed);
  every surfaced decision is resolved or consciously deferred.
- **Deferred, with a clean seam:**
  - **lens-client review deferrals (Plan 4 triage, 2026-06-26):**
    - **#5 event-surface recapture (capture spike) — CAPTURE DONE (2026-06-26).** Drove the live
      pinned server with the now-available native harnesses (`omnigent claude`/`cursor`/`polly`,
      persistent runner + REST `message` injection) + a Cursor **SDK** key (`crsr_`, keychain) for
      real reasoning. **Byte-verified 13 previously-`SCHEMA-DERIVED`/`Unknown` families:**
      `reasoning_text.delta`, `agent_changed`, `child_session.updated` (+child spawn `session.created`),
      `resource.deleted`, `session.model`/`reasoning_effort`/`todos`, `compaction.in_progress`,
      `cancelled`/`interrupted`, `terminal.activity` (via **SSE — no WS**), elicitation
      request+resolved (policy agent), `skills`/`heartbeat`/`failed`. **Findings + deltas:**
      [`docs/spikes/2026-06-26-live-event-recapture.md`](./spikes/2026-06-26-live-event-recapture.md);
      raw corpus `docs/spikes/captures/2026-06-26-live-recapture/`; memory `live-event-recapture-findings`.
      **Key correction:** native TUI mirrors (claude/cursor-native) FOLD reasoning like claude-sdk —
      real `reasoning_text.delta` needs a reasoning-emitting *inner executor* (cursor SDK here).
      **Still blocked (no codex sub / no OpenAI key / subscription `llm_model:null`):** `turn.*`
      (codex-native only), `response.created`/`queued` (openai-scaffold), `reasoning_summary_text.delta`
      (codex), `compaction.completed` (needs configured model). **Deliverable was capture-only** —
      a follow-on modeling plan flips byte-verified families `SCHEMA-DERIVED→MODELED` and grows the
      two under-modeled payloads (`child{}` object; elicitation `params`).
    - **Small hardening:** `info.databricks_features: Value` (one read-side `Value` leak — type or
      make opaque); `ClientError::NotFound` false-friend rename + typed `Validation`/422 variant;
      `gap==Some(0)` proof, `/items` pagination, gated live reconnect smoke (no server-kill harness).
    - **Document for the reducer:** two status vocabularies (`SessionStatusValue` 6-val live vs
      `SessionStatus` 3-val snapshot) + two usage representations must be normalized by the consumer.
    - **WS terminal attach client (Plan 7)** — no `terminal.rs`/`tungstenite`; the workspace/terminal
      half of the contract can't be built on the crate yet (known build-order deferral).
  - **Notifications v2** — server-side push for the *fully-quit* case (needs an
    upstream omnigent push channel; v1 covers resident/backgrounded, shell §17.4).
  - **Server-stability spike** (capability §0.8) — **trending PASS; the
    Rust-sidecar contingency does not reopen.** Full findings:
    [`docs/spikes/2026-06-25-transport-stability.md`](./spikes/2026-06-25-transport-stability.md).
    Warm cold-start ~1.6s, ready ladder <5ms; runs agents end-to-end; live SSE
    parses clean (subscribe-first/no-replay); **mid-stream-drop reconnect
    recovers with zero persisted-item loss** (typed-client §7); failures typed
    (`runner_failed_to_start`); daemon/runner lifecycle confirms
    server-lifecycle §3.1/§6. Not separately driven: server-crash reconnect
    (P7), RSS under sustained load. Throwaway harness discarded.
  - **Markdown renderer — SPIKED 2026-07-07 → PARTIAL (lock holds).** Architecture
    passes (retained id-keyed state, no remount, flat ~25µs/frame across 17KB), but
    3 hardcoded module behaviors break naive streaming (200ms trailing debounce;
    `clear_selection` on reparse; `list_state.reset`→scroll-to-top) → confirms
    **vendor-just-the-markdown-module**. **Follow-up** = the vendor-and-patch (3
    localized fixes) + mdstitch safe-prefix (deferred, needs Rust 1.95). Findings:
    [`docs/spikes/2026-07-07-markdown-streaming.md`](./spikes/2026-07-07-markdown-streaming.md).
  - **Variable-height virtualization (§4.1c/d) — SPIKED 2026-07-08 → GO on native
    gpui `list()`.** Head-to-head (native `list()` vs gpui-component `v_virtual_list`)
    behind one `RowSource` seam: native `list()`/`ListState`/`ListAlignment::Bottom`
    passes **all four transcript §16 contracts (7/7)** incl. the 1b off-screen-above
    anchor holding; gpui-component fails the whole bottom-anchoring family. Retires
    the "needs a custom virtualizer" residual. Divides the dep story: **native
    `list()` for the transcript scroll surface, gpui-component for markdown + §4.3
    forms.** Findings:
    [`docs/spikes/2026-07-07-transcript-virtualization.md`](./spikes/2026-07-07-transcript-virtualization.md);
    memory `transcript-virtualization-spike-2026-07`.
  - **JSON-Schema elicitation form (§4.3) — SPIKED 2026-07-08 → GO** on native gpui
    + `gpui-component` inputs (**6/6** probes). The doc's "arbitrary/nested JSON-Schema
    form" framing was wrong: MCP elicitation is a **flat object of primitives**, and
    the real surface is a **discriminated set** (url/binary/AskUserQuestion/plan/codex),
    not a general renderer. Proved the runtime flat-schema→`gpui-component`-inputs mapper
    reads back into valid flat `ElicitationResult.content` (required-gate, default, enum,
    oneOf, never panic) + composes with the discriminated cards + raw key/value fallback.
    ⚠ fixtures source-derived (not byte-verified; live captures were url-mode). Findings:
    [`docs/spikes/2026-07-08-elicitation-form.md`](./spikes/2026-07-08-elicitation-form.md);
    memory `elicitation-form-spike-2026-07`. **No load-bearing framework residual
    remains — the framework spike series is closed.**
- **Tunables for the verification pass:** auto-sleep threshold (~10m), poll cadence
  (~10s), ring-buffer size (10 MB), transcript truncation tiers, `cost_samples` cadence.
- **Small undecided UX:** terminal-`transfer` UX, managed-provider selection,
  policy/skill in-app authoring, multi-depth breadcrumb, exact-vs-range version pin.
- **Build artifact:** render icons are unicode placeholders — ship a real
  status + harness-provider icon set.

## Recent

- **2026-07-11** — **state-model P3-3b GRILLED & CLOSED → spec amended; next = `writing-plans` +
  execute (NEW session).** Full grilling pass over **Bucket A** (recovery semantics) + **Bucket C**
  (scaffold-id + tech-debt). All decisions locked in **spec §2.4 D24–D31** (SSOT); app-arch §3.5 +
  §13.1 amended (P3-3b amendment blocks). **Bucket B** (viewport/render) carved into its own future
  plan = a NEW `lens-ui` crate/cluster. **Recovery model:** park = actor **EXITS** (Disconnected
  resting state, `lifecycle` stays `Active`, RAM-only reason — dissolves the feeder-wedge); **ONE
  user-gated `reconnect` = respawn** (no auto-retry); **nothing auto-terminal** (even 403/404
  re-test on reconnect); **Slept persisted / Parked = RAM fault**; re-read live status on attach.
  **Send-recovery:** D-0 no silent text drop (3 fates: definite→`SendFailed{content}` restore,
  held→soft-pending, lost→`SendLost{content}`); D-a held landed-detection = content-match +
  conservative-landed bias + FIFO dup-match (runs on the in-actor reconnect); D-b build **no**
  survival persistence, defer park→respawn held-survival to the `lens-ui` composer-draft layer
  (**arch B**: composer owns durability, feeds the engine DOWN the spawn port, engine never names
  lens-ui). **Scaffold-id (C1):** dedup at **PERSIST**, **uniform `id → call_id`** + provisional
  flag + store-frontier cursor (no content-stamp). C2 frontier-fail-closed / C3 catch-up→iteration /
  C4 arg-bundling = DO; C5 = DEFER. **3 omnigent 0.5.1 live probes** settled the unknowns:
  `failed` = resumable-in-place (never 404s, heals on next send, resets to `idle` on restart),
  organic-crash == `stop_session`, 503 host-gated, and **messages do NOT split** (id-match, no
  content-stamp). **Deferred:** `lens-ui` composer-draft layer (arch B), an omnigent
  **client-message-id** contract feature-request, Bucket B viewport.
  Handoff: [`docs/handoffs/2026-07-11-state-model-p3-3b-grilling.md`](./handoffs/2026-07-11-state-model-p3-3b-grilling.md);
  memory `state-model-p3-3b-grilling`. **NEXT: `writing-plans` for P3-3b from spec §2.4, then execute
  subagent-driven** (composer per task + cross-family seam review on the D24/D27/D30 subtractive seams).

- **2026-07-10** — **state-model P3-3a (lifecycle core) EXECUTED & COMPLETE — merged to `main`.**
  8-task subagent-driven plan executed (composer-2.5 author + Opus per-task + **grok-4.5 seam
  review on Tasks 3/4/5** + Opus whole-branch); branch `feat/state-model-p3-3a-lifecycle-core`
  **21 commits** off `main@8a13057`, full `xtask gate` green (lens-core 157 / lens-client 149 /
  lens-store 7, fmt+clippy `-D warnings`, drift, `generated.rs` untouched). Ships: **disk-canonical
  transcript** — reducer stops emitting item deltas (deleted `ItemAppended`/`ItemUpdated`, added
  `TranscriptAdvanced{committed_ordinal}` watermark), actor commits the **terminal-prefix** of
  `state.items` to `TranscriptStore` at contiguous ordinals seeded from `frontier()`, prunes RAM
  (D20/D23); **actor is the sole `/items` fetcher** — forward `order=asc` catch-up from the disk
  frontier on spawn + `Reconnected`, buffer-live-then-drain-after (D19); **reader demoted to
  transport-only** (`Reopen` 3→2, item replay deleted); **`SessionCommand::Sleep`** (in-loop
  `is_quiesced` recheck, flush `lifecycle=Slept`, best-effort `StopSession`, unit
  `ActorOutcome::Slept`/`SleepDeclined`) + skeletal **`FleetScheduler`** (sleep/wake registry) +
  wake-respawn round-trip (D21); D15 `created_at` fold+guard + deleted vestigial `last_seen_seq`.
  **The seam review earned its keep**: grok caught **3 correctness bugs invisible to spec-vs-code
  review** — (1) **dual-id `function_call` stranding** (golden `happy_path.stream.sse` L38–50:
  `in_progress` `fc_52…` and `completed` `fc_5a…` carry **different ids / same `call_id`** with a
  terminal message between → the in-progress twin becomes a permanent non-terminal zombie freezing
  all commits + unbounding RAM; **locked decision #1's own pin-and-verify precondition was FALSE
  against real bytes**) → fixed by **`call_id` supersession** in `push_item` (completed FC supersedes
  the resident in-progress twin; store-twin `fc_*` deferred P3-3b); (2) **re-fire ordinal gap**
  (conflict-preserving upsert still bumped `next_ordinal`) → RETURNING insert-vs-conflict; (3)
  **Reconnected greedy-drain ordinal inversion** (live tail committed before catch-up history,
  masked until Task 5 removed the reader replay) → defer-commit on Reconnected batches. Plus **two
  false-green tests** caught + rewritten to true regressions. Opus **whole-branch review** found one
  more cross-task Important (**N1**: a `Sleep` deferred through catch-up rechecked `is_quiesced`
  against stale pre-buffered state + dropped buffered live events → defeats D21) → fixed by replaying
  buffered events before deferred commands; grok-verified. **D17 live-verify RAN & PASSED** vs a live
  0.5.1 server (drove a headless claude-sdk turn → idle session, 3 items; `StopSession`→202
  `queued:false`; **post-stop `/items` forward re-fetch IDENTICAL** — durable; `after` cursor
  **exclusive** confirmed live; gated `live_sleep_wake.rs` codifies it). **Deferred to P3-3b**
  (all documented, none block): scaffold `fc_*` store-twin double-commit; N1-class refinements;
  the disk `RowSource` viewport/UI; `frontier()`-Err fail-loud hardening; catch-up recursion→iteration;
  `RunCtx` arg-bundling. Handoff:
  [`docs/handoffs/2026-07-10-state-model-p3-3a-execution.md`](./handoffs/2026-07-10-state-model-p3-3a-execution.md);
  memory `state-model-p3-3a-executed`. **NEXT: P3-3b** (grilling + plan) — scaffold-id
  reconciliation, held-bubble resume, `SendLost`, the RowSource UI.

- **2026-07-10** — **Contract-coverage gap analysis (post-0.5.1).** lens-client models a
  **deliberate consumer-driven subset** of the omnigent contract (ADR-0001 / reuse-only-ids),
  NOT the whole surface — the unmodeled routes/events are model-on-demand, **not debt**. Gap:
  **14 contract routes with no typed method** (all non-store: infra/registry `/v1/harnesses`,
  `/v1/runners`(+`/{id}/token`), `/v1/hosts/{id}/worktrees`; session config/admin
  `/sessions/projects`, `/agent/contents`, `/agent/mcp-servers*`, `/codex_goal*`, `/v1/sharing`;
  env editing `/resources/environments`(+`/changes`), `/resources/files:copy`) + **11 DEFERRED
  SSE events** (`turn.*`, `response.created/queued/retry/heartbeat/output_file.done/client_task.cancel`,
  `session.collaboration_mode` — deferred because absent from golden captures). **Nothing in the
  gap blocks P3-3a** — all store deps (`/stream`, `/items`, snapshot, send) are modeled; safety
  net = unmodeled routes never called, unmodeled events → `Unknown` + `live_taxonomy` alarm.
  **turn.* capture-check — DONE this session, deferral VALIDATED.** Brought a host daemon
  online (`omnigent host …` — the "offline runner" was just no daemon attached) and drove a
  real "say hello" turn across **4 harnesses** (claude-sdk/codex/opencode completed; cursor
  failed on its own auth), capturing raw SSE. **No `turn.*` fired on any** — every harness
  expresses turn lifecycle via `response.in_progress` → deltas → `response.completed`/`.failed`
  + `session.status`. So the quiescence model (`response.*` + `session.status`) is correct and
  `turn.*` staying DEFERRED is provably safe. (Other deferred events — `output_file.done`,
  `queued`, `client_task.cancel`, `collaboration_mode` — need triggers a trivial turn doesn't
  exercise; not observed, `live_taxonomy` alarms if one ever surfaces.) See memory
  [[contract-coverage-gap-2026-07]].
- **2026-07-10** — **omnigent pin bumped `v0.4.0` → `v0.5.1`** (`bumping-the-omnigent-pin`
  runbook; tag `v0.5.1`, Source HEAD `08285468`; pinned the latest patch since it's
  contract-identical to `v0.5.0` — only a web-shell UI fix on top). Contract delta: +3 routes
  (`/v1/hosts/{id}/worktrees`, `/v1/sessions/{id}/resources/files:copy`, `/v1/sharing`) +
  **2 new SSE event types** — `response.policy_denied` + `session.mcp_startup` — both modeled
  SCHEMA-DERIVED in `stream/event.rs` (+ `MODELED_EVENT_TYPES` + unit tests); lens-core
  `folds.rs` exhaustive matches rippled (both **marker-only** — no state-model field home
  yet). Re-vendored `vendor/omnigent-0.5.1/`, re-codegen'd `generated.rs` (119 schemas),
  bumped `PINNED_OMNIGENT_VERSION` + test literals, re-grounded docs. **Live-verify caught a
  latent contract bug offline gates can't see:** `GET /v1/sessions/{id}` decode blew up
  (`invalid type: null, expected i64`) — hand-authored `ModelUsage` in `sessions.rs` used
  non-`Option` `i64` + `#[serde(default)]`, but the schema declares those token fields
  `anyOf[integer,null]` and `#[serde(default)]` rejects an explicit `null` (only fills a
  *missing* key). Latent since ≥0.4.0; fixed → `Option<i64>`/`Option<f64>`, accessors +
  lens-core `snapshot.rs` map through (`null` bucket stays `None`). **Gate:** clippy(0 warn) ·
  149 lens-client + 139 lens-core tests · `no drift: 60 client paths match` — green (one
  pre-existing spike-file fmt diff, unrelated). Daemon reinstalled editable + restarted →
  serves `0.5.1 (08285468)`; handshake + reachability live-verified. **Follow-ups:** the 3
  new routes + 2 new SSE events are unmodeled/marker-only in lens-core — the state model may
  want to surface mcp-startup + policy-denied; `live_taxonomy` not runnable here (no attached
  runner). See memory [[omnigent-pin-bump-0-3-0]].
- **2026-07-10** — **state-model P3-3a PLAN WRITTEN + 5 design decisions locked + D19
  source-verified against omnigent `31669e1b` (Opus, this session).** Ran `writing-plans`
  for P3-3a from spec §2.3 (D19–D23) → **8-task subagent-driven plan**
  ([`docs/plans/2026-07-10-state-model-p3-3a-lifecycle-core.md`](./plans/2026-07-10-state-model-p3-3a-lifecycle-core.md)),
  regrouped from the spec's flat 7 into: (1) D15 `created_at` + **delete vestigial
  `last_seen_seq`**; (2) pure `is_quiesced`/`transient_work_outstanding`; **(3)[GROK]**
  actor item-lifecycle D20+D23 (commit-terminal-prefix + `TranscriptAdvanced` watermark +
  prune + drop item deltas); **(4)[GROK]** D19 actor forward catch-up (sole `/items`
  fetcher, mode-switched buffer-then-drain); **(5)[GROK]** reader transport-only; (6)
  `Sleep`/wake respawn; (7) `FleetScheduler` seam + round-trip + gated D17 live-verify;
  (8) docs + push. **Cross-family review = grok-4.5 via cursor-delegate at all three seams**
  (user asked for the 3rd pass on Task 4). **5 owned decisions locked via grilling:**
  commit-terminal-prefix ordinals; reducer emits no item signal (actor scans `state.items`);
  `ordinal=items.ordinal` idempotent re-fire; **`last_seen_seq` deleted** (vestigial post-D19);
  scaffold `fc_*` double-commit deferred to P3-3b (key on `Item::id()`). **D19 SOURCE-VERIFIED**
  at the user's request before locking: `/items` = item-id cursor (no seq), `/stream` =
  no-replay ("clients reconcile via the snapshot endpoint", `sessions.py:19387`),
  `sequence_number` = per-stream wire metadata (not a durable resume cursor) → item-id
  frontier is the ONLY durable resume path; holds. **Bonus finding:** omnigent's web UI hits
  the identical scaffold two-id-space merge and dedupes by `call_id`/`itemId` in one ephemeral
  `blocks` list, never persisting live items — a working reference for the P3-3b fix (memory
  `omnigent-two-id-space-reconciliation`). Handoff:
  [`docs/handoffs/2026-07-10-state-model-p3-3a-execution.md`](./handoffs/2026-07-10-state-model-p3-3a-execution.md).
  **NEXT: execute P3-3a subagent-driven** (composer-2.5 per task + Opus inline + grok-4.5 at
  seams 3/4/5); **push after completion** per user directive. Docs-only commit, **not pushed**.

- **2026-07-10** — **state-model D23 (disk-sourced render) DECIDED + doc drift
  consolidated + grok-verified + committed (Opus, this session).** Coherence audit
  of the P3-3a-era design drift → three outcomes. **(1) Sweep:** D19's "reader
  transport-only, actor sole `/items` fetcher" reversal left stale `/items`
  attributions in the two docs the amendment pass missed (server-lifecycle §6/§9,
  conversation-transcript §17) + app-arch narrative that predated its own D19/D21
  blocks — reattributed to actor forward catch-up. **(2) Drift diagnosis:** the
  reversals (D11→D20, 3b-2b→D19, D17→D21) are **convergent, not thrash** — all
  unwind *premature layer-boundary bindings* (producer-side decisions locked before
  the consumer's shape existed); all subtractive; each cites a mechanism invisible
  at lock time. Real risk = rising **consolidation tax** (accreting supersede
  markers + manual cross-ref). **(3) D23 — the render-window "hole" dissolved:** the
  focused replica reads its transcript from disk (`TranscriptStore`) via an
  **id-keyed-upsert `RowSource`**, NOT shipped item deltas. Delete `ItemAppended`/
  `ItemUpdated` (index-addressed deltas go unsound once the actor prunes — actor
  `items.len()≈1` vs replica window ≈thousands); add `TranscriptAdvanced
  {committed_ordinal}` watermark; `Rebased`→scalars-only; actor **commits on
  terminal status only**. **No `TranscriptInvalidated`** — omnigent 0.4.0 items are
  append-only/immutable (compaction/`/clear`/fork all additive; no `item.*` mutation
  event; store append-only), a pin-and-verify seam. **Spike** (`transcript-virtual
  -- --handoff`, ran green): scratch→disk handoff is flash-free under id-upsert;
  clear-recreate remounts (negative control). **Grok-4.5 cross-family pass** (vs
  `../omnigent` source): immutability premise **CONFIRMED safe to lock**; 4 findings
  folded in (softened `taxonomy_drift` guard; captured scaffold live-id≠store-id
  dedup hazard; marked surviving D20 emit-as-StreamUpdate prose superseded; D10
  wording). Commits `20c67de` (spike) + `1d0e97f` (docs), **not pushed**. D23 =
  P3-3a scope (subtractive; MANDATORY cross-family review seam with D19). **NEXT:
  `writing-plans` for P3-3a** now incorporating D23. **Deferred (viewport/UI plan):**
  the disk `RowSource` (windowed read, scroll-back paging, id-upsert).

- **2026-07-10** — **state-model P3-3 SLICED + P3-3a GRILLED → LOCKED-doc amendments
  written (Opus, this session).** P3-3 split into **P3-3a lifecycle core** / **P3-3b
  recovery semantics**; 3a grilled to shared understanding, producing **four new
  decisions (D19–D22) in spec §2.3** and a **material revision of D8/D9/D11 + the D14
  rationale**. **Key outcomes:** (D19) reconcile = **bounded wake-load + unbounded
  actor-owned forward catch-up** (`GET /items` `after=frontier, order=asc` until
  `has_more=false`, on the actor thread, live buffered+drained); the **actor is the
  sole `/items` fetcher** and the `lens-client` reader goes **transport-only**
  (delete item-replay from `reconnect`+`bootstrap`, shrink `Reopen` 3→2, delete
  `items_to_replay` — subtractive, still a MANDATORY cross-family-review seam; amends
  the 3b-2b "reader owns item recovery" decision). (D20 — **category-error fix**) the
  actor holds a **small pruned working set, NOT an 8 MB byte-window**; **disk is
  canonical** for finalized items (write-through + emit + prune; far-back re-fire =
  blind disk upsert-by-id); the ~8 MB render window is a **deferred replica concern**
  (live tail = actor→replica RAM, scroll-back = disk) → 3a drops actor-side
  eviction/byte-accounting. (D21) **sleep = `SessionCommand::Sleep`** (in-loop
  re-check → flush → best-effort `stop_session` → stop → `Slept`), **wake = respawn**
  from disk, external §9 trigger; 3a ships a **skeletal `FleetScheduler` seam** + a
  deterministic round-trip test. (D22) **never-seen-huge first-attach deferred whole**
  (snapshot-tail-paint + negative-ordinal scroll-back; `i64` ordinal leaves the door
  open, no migration); **D15** (`created_at` fold+guard, still unfixed) rides in 3a.
  **Amendments written** to `spec §2.3/§4/§7.1`, `app-arch §3.4/§4.1/§6.3/§8`,
  `typed-client §7 + Bootstrap` (34 markers; +246/−7, docs-only). **3a task order**
  (build catch-up *before* deleting reader replay): D15 → pure `is_quiesced`/
  `transient_work_outstanding` → actor catch-up+prune+`Rebased`-drops-items → reader
  transport-only (review seam) → `Sleep`+wake → `FleetScheduler` seam + round-trip
  test + gated D17 live-verify → docs. **NOT committed→committed docs-only (not
  pushed).** **NEXT (fresh session): `writing-plans` for P3-3a** from spec §2.3;
  then execute subagent-driven. **P3-3b** (held-bubble resume, `SendLost`
  re-derivation, cmd-path 403/404 §9 escalation, parked-feeder drain / outcome-channel
  wedge; coupled to composer send-recovery) gets its own grilling+plan later.

- **2026-07-09** — **state-model P3-2 (command semantics, D16/D18) EXECUTED & MERGED to
  main** (ff-only, `d5df2a1..51b10af`, 16 commits). Subagent-driven: composer-2.5 author +
  Opus inline review per task; **seams (Tasks 6–9) each got an Opus-subagent cross-family
  review** (grok/cursor async was erroring mid-session → used Opus Agent); user then
  revamped+reloaded cursor-delegate (plugin 0.1.0, +`doctor`/+`cursor_answer`) and the
  **whole-branch consolidated review ran on grok-4.5-xhigh** — the 3rd family that found the
  cross-task defects per-commit reviews structurally miss. **Delivered:** `SessionCommand::Send`
  (optimistic bubble → blocking POST via injected `SessionApi` → stamp-whichever-ack-id →
  `CommandOutcome`, Table-B rollback); reconcile precedence (1)pending_id (2)item_id
  (3)content live + snapshot; D18 Table A park/stop + actor-owned `ActorTransport`/
  `reconcile_in_flight`; Table B `map_client_error` + non-blocking `OutcomeRing`. **Cross-family
  review caught real bugs the author+inline missed:** grok — `send_event` had no request
  timeout (actor hang, risk 5a) + `lens_pend_` id collision on reconnect; Opus — same-batch
  reconnect delta overwriting a terminal park (zombie transport); grok whole-branch — held
  Table-B bubbles silently dropped on snapshot + Send accepted while parked. All fixed.
  **Gate:** lens-client 146 / lens-core 141 / lens-store 7, fmt+clippy(-D warnings) clean,
  `generated.rs` untouched. **NOT pushed** (awaiting call). **NEXT: P3-3** — D17 quiesce/
  sleep/wake (`is_quiesced` = `transport==Connected && !reconcile_in_flight`), D11 byte-window
  eviction, blocking `GET /items` tail-pagination; plus deferred **composer send-recovery +
  input-history** (memory `composer-send-recovery-and-history`) and the P3-3 forward-notes in
  `.superpowers/sdd/progress.md` (held-bubble resume, `SendLost` re-derivation, cmd-path
  403/404 §9 escalation, parked-feeder drain policy, outcome-channel wedge).

- **2026-07-09** — **state-model P3-2 PLANNED + D16 live-verify rider RESOLVED.** Plan
  `docs/plans/2026-07-09-state-model-p3-2-command-semantics.md` (`bc3082d`,
  10 TDD tasks). **Authored by grok-4.5-xhigh** (cursor-delegate, model-eval experiment),
  **reviewed by Opus cross-family** with every claim verified against the tree — satisfies
  the MANDATORY diversity rule (grok = non-Claude author). Grok independently found **two
  verified prerequisites** the brief missed: `cleared_pending_id` dropped at
  `event.rs:314` (`RawInputConsumedData` parses only `item_id`+`type`) and `pending_inputs`
  unmodeled on `SessionSnapshot` — both on the wire, load-bearing for D16 reconcile
  (Tasks 2–3). Opus revisions applied: split Task 7 live-`Consumed` precedence vs snapshot
  keep/drop/lost table + decisive reducer-placement; **Risk 5a** (actor `Select` deaf to
  `Stop` while blocked in `send_event` → require finite HTTP timeout + matrix case);
  **Risk 8a** (`SessionApi` injection ripple across P3-1 spawn surface, `Box<dyn
  SessionApi+Send>` per the Clock precedent); M1 marked optional (self-heals, hot reduce
  arms); `SendLost` = actor-diffed not reducer-emitted. **D16 rider CLOSED:** live 0.4.0
  (`31669e1b`) + route source (`sessions.py:19368`) — POST ack is a non-empty bare dict,
  exactly ONE of `item_id` (non-native / native-terminal-down) or `pending_id` (healthy
  native) per message POST; precedence (1)/(2) are common paths, (3) defensive-only.
  **GOTCHA: native ⇏ pending_id.** Memory `state-model-p3-grilling` updated. **Env note:**
  `../omnigent` checkout moved to the pinned `v0.4.0` (`31669e1b`); editable install +
  daemon now serve 0.4.0. **NEXT: execute P3-2 in a fresh session** (subagent-driven,
  composer-2.5 per task, cross-family review at the Task 6/7 send-reconcile + Task 8/9
  lifecycle seams).
- **2026-07-09** — **state-model P3-1 (actor foundation) EXECUTED & MERGED to main.**
  All 7 TDD tasks done via subagent-driven-development (composer-2.5 build per task +
  Opus per-task cross-family review + fixes; codex used for Task 1's mandatory seam
  until credits ran out, then Opus = the cross-family diversity reviewer). **12 commits
  `1096a8c..f7c9a64`**, fast-forward merged to `main` (**not pushed**). Gate green:
  lens-client 139 / lens-core 89 / lens-store 6, fmt + clippy `all=deny` clean (spikes
  excluded — no `lints.workspace`). Delivered: **D13** lens-client reader `mpsc`→crossbeam
  + `receiver()`; **D8/D9** value-carrying `StreamUpdate` + `items: Vec<Arc<Item>>` +
  `Rebased`; new **`crates/lens-store`** `SessionStore` replica + O(1) copy-assign `apply`
  (~102ns, §5 met); **D7** off-thread→foreground `spawn_apply_bridge` (greedy-coalesced,
  one `cx.notify()`/frame); the **`ActiveSession` actor** (`lens-core/actor`, gpui-free)
  — crossbeam `Select` ingest + persist write-through + coalesce; **D10** dual-mode
  `Detailed|Summary` + promote/demote. **Reviews caught real bugs:** Task 5 batched
  multi-append ordinal collision (on-disk transcript corruption under load) → fixed +
  regression test; whole-branch I1 = actor never emitted a `Rebased` after a
  `SnapshotRestored` fold (Detailed replica silently missed ~20 chrome scalars) → fixed;
  I2 = `last_task_error` had no delta variant (stale error banner) → `LastTaskErrorChanged`
  added; plus `context_window` value-carrying gap + gpui `test-support` dev-dep scoping.
  **DEFERRED to P3-2** (documented in `.superpowers/sdd/progress.md`): M1 `current_agent`/
  `turn` non-guaranteed `ScratchChanged` (self-heals); M2 `Demote` on a Detailed-only
  `spawn_actor()` handle silently kills the thread (guard when D16 lands); reserved
  `CollaborationModeChanged`/`TitleChanged` variants have no producer. **NEXT: P3-2**
  (D16 optimistic-send/reconcile + D18 §13.1 error-mapping), then **P3-3** (D17
  quiesce/sleep/wake + D11 byte-window eviction + the blocking `GET /items` tail-pagination
  dep from the Task 0 spike).
- **2026-07-09** — **state-model P3 sliced + Task 0 spike DONE + P3-1 plan written.**
  P3 (actor + store + commands) is too big for one plan → sliced into **P3-1 actor
  foundation** (channel swap + skeleton + run-loop + dual-mode, D7/D8/D9/D10/D13),
  **P3-2 command semantics** (D16 optimistic-send/reconcile, D18 §13.1 map), **P3-3
  lifecycle** (D17 quiesce/sleep/wake) — plus the **Task 0 spike** as a throwaway
  (not a plan). **Task 0 (D12) large-transcript latency spike — DONE** (`cb56f38`,
  background subagent; findings `docs/spikes/2026-07-09-large-transcript-latency.md`;
  harness `spikes/large-transcript/`, 516 MiB `.db` gitignored; memory
  `large-transcript-latency-spike-2026-07`): 100k-item/500 MiB corpus →
  windowed page-load sub-ms, byte-budgeted cold-hydrate tail 4.88ms, **reconcile
  full-history 1062ms vs tail-bounded ≤2.85ms (370–3100×)**. **LOCKED P3-3 contract:
  reconcile bounded-tail, never full history**; blocking dep = lift `GET /items` tail
  pagination (deferred from 3b-2b) in P3-3. D11 byte-window premise held. Paged-load
  SQL shapes captured. **P3-1 plan written** (`28b73ab`,
  `docs/plans/2026-07-09-state-model-p3-1-actor-foundation.md`, 7 TDD
  tasks; grounded in real gpui-0.2.2 bridge API + reader.rs + P1/P2 surfaces; scratch
  representation decided `ScratchChanged(Arc<StreamScratch>)`+coalesce). Tasks 1 & 5
  are the MANDATORY cross-family-review seams (lens-client channel swap; run-loop).
  **Bench-hardening + `xtask gate`: DONE** (2026-07-09, memory
  `benchmark-validity-audit-2026-07`; inline-authored + free-codex cross-family
  review → 2 catches applied). (1) `reduce_throughput.rs` — added
  `reduce_window_scale/build_1500_item_tail` variant that makes `push_item`'s O(n)
  linear dedup scan visible: **1.20ms** to build a 1500-item tail vs **1.20µs** for
  the whole happy-path replay (the O(n²) tripwire, previously hidden); `fresh_state()`
  moved to `iter_batched` setup. Seam = doc-hidden `reduce::bench_push_message`
  (always-compiled, no feature). (2) `persist_throughput.rs` — DB-open (WAL+DDL) +
  teardown (close/file-delete/dealloc) moved OUT of the timed body via `iter_batched`
  setup + return-outputs; bimodal corpus (5×200KB blobs + 195×~130B markers, spike-
  matched) → **~15ms** now measures a realistic 1MB write+reload, not open cost. (3)
  New **`cargo run -p xtask -- gate`**: fmt → clippy (feature matrix: default +
  lens-client `bench`/`live-tests`) → test → `cargo bench --no-run` (compile-only,
  no criterion sampling) → `drift`. Scoped to production crates (`spikes/*` opt out);
  a missing sibling omnigent spec **hard-fails** (via `read_json`), never silently
  skips. Codex caught both benches charging teardown to the timed body (fixed) +
  overstated reduce comment; the gate caught its own unformatted code + a dead import.
  **P3-1: DONE & merged 2026-07-09** (see the entry above; plan
  `docs/plans/2026-07-09-state-model-p3-1-actor-foundation.md` fully executed).
- **2026-07-09** — **state-model P3 GRILLING — CLOSED (session 2).** The 4 open
  branches resolved as **D15–D18** in new
  [`spec §2.2`](./specs/2026-07-08-state-model-engine-design.md)
  (+§7.1 §13.1-amendment row + §4 P3 batched live-verify gate); memory
  `state-model-p3-grilling`. **Spec still UNCOMMITTED (working tree only).**
  **D15** `created_at` = first-non-zero-wins upsert guard **+** a found P1 defect
  (`fold_snapshot` never sets `state.created_at` → add the fold). **D16**
  optimistic-send reconcile keyed by server ack ids (`SendEventAck` *already*
  carries `pending_id`/`item_id`; `PendingUserMessage` gains
  `server_pending_id`/`store_item_id`; precedence native-id → item-id → content
  fallback). **D17** `is_quiesced` = pure `transient_work_outstanding()` ∧
  actor-owned `transport==Connected` ∧ `!reconcile_in_flight`; pinned=§9 gate not
  predicate; sleep = re-check-abort → flush-durable → best-effort `stop_session`
  fire-and-forget → stop actor → drop RAM. **D18** §13.1 splits into two
  path-keyed tables — stream `Disconnected{reason}` (park Unauthorized/Failed/
  RetriesExhausted, stop Forbidden/NotFound) vs `ClientError` command-outcome (fill
  `Server`/`ThreadSpawn` gaps, drop phantom `Ws`). **Two live-verify riders
  batched** into one gated 0.4.0 P3 run (ack id population; post-`stop_session`
  wake-refetchability) — not spec-blocking. **NEXT:** commit spec → do the §7.1
  LOCKED design-doc amendments (§8/§9/§13.1) → `writing-plans` for P3.
  — — — (session 1, D8–D14, still current:) **Decided:** value-carrying `StreamUpdate`
  (option A) + `items: Vec<Arc<Item>>` (share bodies actor↔replica; rejected
  whole-state snapshot swap = O(n²)/turn); one-shot `Rebased(Box<State>)` baseline
  at attach (reducer only appends/updates-in-place → no remove variant);
  **focus-scoped fidelity** — full replica only for focused (≤~10), background-warm
  gets a coarse actor-emitted `SummaryUpdate` (dual-mode `Detailed|Summary`,
  promote/demote; policy is §9); **byte-windowed** in-RAM transcript (~8 MB tail,
  older paged from `TranscriptStore`; user confirmed real sessions hit ~600 MiB /
  10k–100k items); a **large-transcript latency spike as P3 Task 0** (page-load /
  cold-hydrate / **`reconcile`-scope** — the real unknown, likely tail-bounded);
  **crossbeam `Select`** ingest (swap lens-client reader channel + `receiver()`;
  the one hardened-lens-client touch → cross-family review); and the §8 rationale
  correction (two copies decouple N warm background streams from the gpui
  foreground executor, NOT "reduce is expensive" — it's 1.36µs). Built an
  architecture **Artifact** (threads/ownership/memory map) as the shared mental
  model. **(These 4 branches are now CLOSED as D15–D18 above.)**
- **2026-07-08** — **state-model engine P2 (lens-core persistence) EXECUTED & MERGED
  to main** (`25e4e09..978fb85`, 9 commits, ff-merge + push; composer-2.5 full-plan
  build + **Opus-only reviews** — Codex/gpt-5.5 + non-Composer Cursor out of credits,
  so cross-family diversity came from Opus-reviewing-composer). The §6 two-tier local
  store in `crates/lens-core/src/persist/`: role traits `ControlStore` (`lens.db`:
  connections/sessions/cost_samples/meta) + `TranscriptStore` (per-session file
  `transcripts/<conn>/<conv>.db`: items + self-describing meta), SQLite impls over
  `rusqlite` **bundled** + WAL + `foreign_keys=ON`. Exposes load/upsert/**reconcile-by-
  item-id** primitives; per-file `schema_version` gate (unknown/corrupt future version →
  **read-only-degraded**, never a hard open failure). **79 tests** (77 unit + 2
  integration), clippy `-D warnings` + fmt clean, `generated.rs`/lens-client untouched;
  `persist_throughput` bench ~13.7ms/(200 upserts+load), I/O-bound. Plan:
  [`docs/plans/2026-07-08-state-model-p2-persistence.md`](./plans/2026-07-08-state-model-p2-persistence.md).
  **Reviews:** plan Opus pre-build review (SHIP-WITH-FIXES → 9 findings `REVIEW#n`
  applied incl. 2 §6.3-contract bugs: corrupt-version hard-Err on open; WAL/DDL mutating
  a file before the version gate — column-mapping + reconcile SQL verified correct);
  Opus end-of-branch review (SHIP-WITH-FIXES → 1 IMPORTANT: **`HostType`/`SessionLifecycle`
  lacked `#[serde(other)]`** so an unknown host_type/lifecycle token aborted the whole
  `list_sessions` — fixed + regression test). **Key decisions (D-P2-1..9, in the plan):**
  two role traits (no god-trait); lossless `cost_json` companion + denormalized Bridge
  projections; `terminal_pending` persisted (P1 contract vs §6.2 sketch); store-managed
  cols (`pinned`/`last_status`/`tombstoned_at`) preserved via ON-CONFLICT omission;
  live-stream chrome (`model_options`/`sandbox_status`/`pending_elicitations`) +
  `presence`/`stream`/`pending_user` RAM-only, re-derived on wake; `load_session` returns
  a disk-snapshot (items empty). **Post-merge hardening DONE (`ff55e48`):** resilient
  loads — `list_sessions`/`load_items` now return `Loaded<T> { rows, skipped:
  Vec<SkippedRow{id,reason}> }` via a shared `collect_skipping` helper: a corrupt/unknown
  row is skipped + reported BY ID (observable, not silent — lens-core stays logger-free,
  app decides) instead of aborting the whole load; also covers the internally-tagged
  `ItemKind` unknown-`kind` case (can't `#[serde(other)]`). **Still deferred to P3
  (upsert-timing, can't decide until the actor's write cadence exists):**
  `created_at=excluded` re-upsert could clobber a good creation time with 0 if the actor
  upserts a fresh state pre-bootstrap → add a `COALESCE`/non-zero guard when wiring P3. **Next: P3 — actor + store + commands (`lens-core/actor` + `lens-store`,
  §8/§7/§13.1): walking skeleton first (fake event → reduce → StreamUpdate over bounded
  channel → SessionStore replica → cx.notify), then actor run-loop, command semantics
  (§7 optimistic-send × reconnect reconcile), bootstrap/reconnect wiring that CALLS the
  P2 primitives. Fresh session (cost/context policy).**
- **2026-07-08** — **state-model engine P1 (lens-core pure reducer) EXECUTED & MERGED
  to main** (`7959391..8a9a456`, 13 commits, ff-merge + push; subagent-driven:
  composer-2.5 per-task TDD + gate + Opus/gpt-5.5 dual end-review, per CLAUDE.md). The
  §4 contract-proving phase: `reduce(&mut SessionState, &ServerStreamEvent, &dyn Clock)
  -> SmallVec<[StreamUpdate;2]>` — pure, deterministic (injected `Clock`; **8 real SSE
  corpus files replay twice → identical state**), total (never panics on decodable data).
  Folds every modeled event: text/reasoning accumulation → finalized items; tool items by
  `call_id`; session-field folds (status/usage/todos/model/effort/sandbox/terminal_pending/
  presence/elicitation/agent-changed/child); `SnapshotRestored` **scalar-only** bootstrap/
  reconnect; `AgentChanged` transcript marker (synthesized `from`); §4.3 render transforms.
  **`StreamUpdate` drafted** (D6 — ratified at the P3 skeleton). **64 tests, clippy/fmt
  clean, `generated.rs` untouched; bench ~1.36µs/full-turn.** Plan:
  [`docs/plans/2026-07-08-state-model-p1-reducer.md`](./plans/2026-07-08-state-model-p1-reducer.md).
  **Reviews:** plan cross-family-reviewed BEFORE build (codex/gpt-5.5, 12 findings incl.
  2 Critical — turn-bump order, clock-based synthetic-id collision — applied); consolidated
  end-of-branch **Opus + gpt-5.5 dual read** → 1 fix wave (7 items: collision-probing
  synthetic ids, `ScratchChanged`-on-preview-clear, `last_task_error` clear, saturating
  turn, terminal-activity marker, merge agent-gate). **The one lens-client touch:** a
  `test-util`-gated `stream::decode_all(&[u8])` byte-decode seam (private `parse_event` was
  unreachable from lens-core tests). **P1 contract-proving findings (lens-client wrapper-
  widening backlog, all degraded-not-dropped + flagged):** `stream::Item` models 5 concrete
  + `Other` while domain `ItemKind` has 11 → native_tool/slash_command/terminal_command
  payloads degrade to a `NativeTool` catch-all; `ItemKind::ResourceEvent` un-materializable
  (no `SessionResourceObject` on the wire) → marker-only; `PresenceViewer` fills `user_id`
  only (joined_at/idle dropped); `session.collaboration_mode` is a *deferred* wire type →
  domain field stays `None`; `depth` fixed at 0 (sub-agent topology = §9). Memory
  `state-model-p1-reducer`. **Next: P2 — persistence (`lens-core/persist`, §6): two-tier
  `ControlStore` (`lens.db`) + per-session `TranscriptStore` (rusqlite/WAL), spec §4 "P2".
  Fresh session (cost/context policy).**
- **2026-07-08** — **state-model engine P0 (lens-core domain types) EXECUTED & MERGED
  to main** (`ff554d7..2069e88`, ff-merge + push; plan-first → composer-2.5 build →
  Opus review, per CLAUDE.md). New gpui-free crate `crates/lens-core` with the full
  LOCKED §2 domain model — pure data + serde, no logic. **Reuse boundary (the
  architectural call):** reuse `lens-client`'s 9 branded ids + `generated::SessionResourceObject`;
  **domain-own every other value/aggregate type** — because `lens-client`'s read
  wrappers (`TodoItem`/`PresenceViewer`/`SessionStatusValue`/…) are deserialize-only
  with private fields, unusable as a mutable, persistable view-model. `branded_id!`
  is not exported → local macro for the 4 new ids (`ItemId`/`CallId`/`ResponseId`/`AgentId`).
  Modules: ids · scalars · usage · controls · item · session. **23 tests, clippy
  clean, fmt clean, full-workspace gate green, `generated.rs` untouched.** Plan
  cross-family reviewed **before build** (free codex/gpt-5.5, 2 Important applied):
  enriched `ModelUsage` to the wire-faithful shape (cache buckets + per-model
  `total_cost_usd`, all optional — was dropping spend/cache data), and flagged a
  **P1 blocker** (below). Plan:
  [`docs/plans/2026-07-08-state-model-p0-domain-types.md`](./plans/2026-07-08-state-model-p0-domain-types.md).
  **P1 handoff notes (in the plan):** (1) `lens_client::stream::PresenceViewer`
  wrapper exposes only `user_id` — drops `joined_at`/`idle` the generated contract
  carries, so P1 can't fill the domain `PresenceViewer` from `ServerStreamEvent::Presence`
  until lens-client's stream wrapper is widened (or P1 reads the generated type);
  (2) `ModelUsage` is now wire-1:1 for P1's usage normalization. **Next: P1 — pure
  reducer + render transforms (`lens-core/reduce`, §4), the contract-proving phase;
  TDD against the golden SSE corpus. Fresh session (per cost/context policy).**
- **2026-07-08** — **state-model engine spec GRILLED → implementation-ready.** After
  the gpt-5.5 cross-family review (6 Important + 3 Minor, commit `05329a8`), a
  focused grilling pass over the implementation-risk seams the review didn't reach.
  Four branches, all resolved (no blocker; no second pass warranted):
  1. **Storage is now two-tier** (design §6 revised, LOCKED-with-marker) — one
     control-plane `lens.db` (connections/sessions/cost_samples/meta) + **one
     SQLite file per session** for `items`, actor-owned WAL connection. Makes each
     actor's writes contention-free by construction, retention/tombstone a file op,
     corruption blast-radius one (re-fetchable) session. `rusqlite`, WAL, single
     serialized writer for the control plane only.
  2. **Transcript key = `(ConnectionId, conv_id)` — safe** (verified in omnigent
     source): `/clear` rotates the runner-internal `external_session_id`, **not**
     `conv_id`; `/clear` is a non-destructive `SlashCommandData` item on the same
     conversation.
  3. **`BlockContext.timestamp` dropped** (design §2.3/§2.4) — vestigial (no
     consumer, never reviewed, can't round-trip as monotonic `f64`); durable "when"
     is now `Item.created_at: i64` epoch on the item envelope, injected-clock-sourced.
  4. **Optimistic-send × reconnect reconcile** (spec P3b note) — the one collision
     §7's FIFO left open (a gap-dropped `consumed` event dup/orphans the optimistic
     bubble); resolved by a reconnect-aware, session-type-asymmetric rule using the
     snapshot's `pending_inputs` (native) / replayed `GET /items` (non-native). One
     P3 live-verify item logged (does `POST /events` return `pending_id`).
  Bonus: the §6.2 `items.kind` comment now lists `error` (resolves the P0
  doc-correction). Edits in `app-architecture-and-state-model.md` (§2.3/§2.4/§6) +
  the spec (D4/P0/P1/P2/P3b). Memory `state-model-grilling-revisions`.
  **Implementation started: P0 DONE (see 2026-07-08 P0 entry above); next = P1.**
- **2026-07-08** — **§4.3 JSON-Schema elicitation form spike EXECUTED → GO on native
  gpui + `gpui-component` inputs (6/6 probes)** (throwaway harness
  `spikes/elicitation-form/`, subagent-driven: composer-2.5 build + headless probe
  auto-run + Opus reframe/probe-validity/interpretation; spec-only, no plan/TDD per the
  throwaway-spike calibration). **The pivotal finding was a ground-truth reframe** (read
  from omnigent 0.4.0 source, not the doc): §4.3 is **not** an arbitrary/nested
  JSON-Schema form — MCP elicitation is a **flat object of primitives**, and omnigent's
  own client renders a **discriminated set** (url/binary/AskUserQuestion/ExitPlanMode/
  codex), with the genuine runtime-schema case firing only for third-party MCP servers.
  So the build is a bounded flat-primitive schema→inputs mapper + structured-payload
  cards, not a hand-rolled renderer. Headless auto-run = **6/6**: runtime dynamic form
  (crux — heterogeneous runtime `InputState`/`SelectState` Entities read back into valid
  flat content, defaults/enum/oneOf, no panic), type coverage, constraints, content
  round-trip (independent oracle; default-flow proven un-seeded), AskUserQuestion carousel,
  composition + raw key/value fallback. Probe-validity guard caught 1 false-FAIL (multi-select
  array order — form sorts, oracle used insertion order; answers are an unordered `list[str]`
  set → order-insensitive compare). ⚠ fixtures source-derived, not byte-verified (both live
  captures were url-mode); opportunistic live capture not run. Reconciled framework §4.3/§4/§5
  + permissions §3 (added the discriminated modes + the AskUserQuestion "cosmetic-for-native-
  Claude" caveat). **Closes the framework spike series — no load-bearing residual remains.**
  Findings: [`docs/spikes/2026-07-08-elicitation-form.md`](./spikes/2026-07-08-elicitation-form.md);
  memory `elicitation-form-spike-2026-07`.
- **2026-07-08** — **§4.1c/d transcript-virtualization spike EXECUTED → GO on native
  gpui `list()`** (throwaway harness `spikes/transcript-virtual/`, subagent-driven:
  composer-2.5 Phase 0–2 build + Opus probe design/interpretation; spec-only, no
  plan/TDD per the throwaway-spike calibration). A+B head-to-head behind one
  `RowSource` seam. **Backend A (native `list()`): 7/7** — windowing (`renders ≪ N`),
  variable heights, stick-to-bottom, **1b off-screen-above anchor held**
  (`logical_scroll_top()` unchanged under above-viewport height mutation — the true
  go/no-go), jump-to-bottom, identity-across-recycle, markdown nesting, UX demo.
  **Backend B (gpui-component `v_virtual_list`):** windowing + identity pass, whole
  bottom-anchoring family fails (no `ListAlignment::Bottom`, pixel-offset only, 1b
  drift, opens at top). Retires the "needs a custom virtualizer" fear (§4.1c/d /
  transcript §19 note 3): `uniform_list` was the wrong primitive, `list()` is the
  right one — no fork, no extra dep. The probe-validity guard earned its keep (2
  probe bugs caught + fixed before they poisoned the verdict: dead keybinds until a
  focused `FocusHandle`; a false identity FAIL from a pre-first-paint baseline).
  Framework §4.1c/d + §5 seam table + transcript §19 note 3 updated. Merged to main
  (`825d462..9a5af61`). Findings:
  [`docs/spikes/2026-07-07-transcript-virtualization.md`](./spikes/2026-07-07-transcript-virtualization.md);
  memory `transcript-virtualization-spike-2026-07`.
- **2026-07-07** — **§4.1 markdown-streaming spike EXECUTED → PARTIAL; gpui lock
  holds** (throwaway harness `spikes/markdown-stream/`, subagent-driven: Task 1 +
  render controller-built, Tasks 2–3 composer-2.5, verdict = probe-facts + user
  eyeball). gpui-component 0.5.1 builds on gpui 0.2.2 (= §3 pin). **Stable-identity
  architecture PASS** (retained `Entity` keyed by `ElementId`, no remount; async
  debounced parse off the render path — probe measured **flat ~25µs/frame across a
  17KB doc**, correlation −0.39 ⇒ no O(n) reparse). **But 3 hardcoded, vendorable
  module behaviors break naive streaming:** 200ms *trailing* debounce that resets
  on each update (`text_view.rs:628`) → fast streams show nothing until a pause;
  `clear_selection()` on reparse (`:610`); `list_state.reset()` on content change
  (`node.rs:1123`) → **scroll-to-top on every render** (violates transcript §5).
  **Verdict confirms framework §4.1's "vendor just the markdown module"** (3
  localized patches) over raw-dep or from-scratch. `sanitize`/`replay` unit-tested
  (5 tests); `mdstitch` safe-prefix deferred (needs Rust 1.95, lower priority given
  the debounce). Merged to main (spike commits `420a91d..ae4b307`). Findings:
  [`docs/spikes/2026-07-07-markdown-streaming.md`](./spikes/2026-07-07-markdown-streaming.md).
  **Open follow-ups:** vendor-and-patch the module; §4.1d variable-height
  virtualization (still un-spiked); §4.3 JSON-Schema form spike.
- **2026-07-07** — **gpui lock re-pressure-tested + markdown-ecosystem survey
  (sets up the spikes)** (memory `gpui-markdown-ecosystem-2026-07`). Following the
  web-app re-read, the live framework question narrowed to greenfield
  **all-gpui vs Tauri+React** (fork is structurally dead). Turned on one axis —
  the React alt's liftable widgets — which a verified crate survey then largely
  neutralized *gpui-side*: `pulldown-cmark` 0.13.4 + **`mdstitch`** 0.1 (Apache,
  streaming safe-prefix) + **`gpui-component`** 0.5.1 (Apache → LIFTABLE: native
  markdown w/ tree-sitter, virtualized `List`/`Table`, form inputs) are all
  liftable → gpui gets widget acceleration without the IPC/type-loss seam.
  **Lock holds, better-supported.** `framework.md` reworked (§1 four pillars
  ordered by load; §4.1/§4.3 survey folded in). **The markdown spike shrank** from
  "hand-roll a renderer" to "integrate + verify **one thing**: does
  `gpui-component`'s markdown accept incremental updates with **stable element
  identity** (no remount on append)?" — plus the Lens-owned sanitization boundary
  (§2.5) and gpui-version-pin compat (prefer vendoring just the markdown module).
  Also: `gpui-form` 0.5.1 is struct-derive (not runtime JSON-Schema), so §4.3's
  residual = runtime schema→inputs mapping over `gpui-component` inputs.
- **2026-07-06** — **Re-read omnigent's shipped desktop app; corrected our stale
  framing** (cursor-delegate read of `../omnigent` @ `62b4254a`, `v0.4.0.dev0-104`;
  memory `omnigent-web-app-state-2026-07`). Prior read was ~6wk stale. Findings:
  `ap-web/` was renamed `web/` (PR #1333); the app is **not** a "half-baked web
  wrapper" — it's a *polished, actively developed* React/Vite SPA (Electron =
  thin shell over the server-served bundle; also iOS/Android/embed targets) with
  Monaco diff+comments, xterm terminals, sub-agent tree, a cross-session approval
  inbox, ~209 tests. **But** the wedge survives, precisely located: it's
  **single-server, single-warm-stream, chat-shaped** — `switchTo` aborts the
  prior session's SSE stream (`chatStore.ts:1411-1417/:2786-2792`, one-warm-stream-
  at-a-time), one server origin per SPA (`host.ts:6-8`; multi-server only via
  separate Electron windows), sidebar+single-`ChatPage` shell, no list
  virtualization. **Corrected wedge (now in docs):** Lens = multi-server,
  **N-warm-streams** (every session live off-thread → zero switch latency, cards
  always live), board-shaped — the differentiator is concurrent *warm state*, not
  concurrent *display*. A fork buys a mature widget toolkit but forces a rewrite
  of connection model + live-state fan-out + navigation, and re-crosses the type
  boundary + inherits the untested hand-ported SSE parser (`sse.ts:6-9`). Edited
  `docs/design/README.md` (wedge) + `capability-map §0.9 / client row`; historical
  `ap-web` refs in review-findings/plans/ADR left as records. **Open follow-up:**
  decide whether the narrowed-but-real wedge justifies the ground-up widget-toolkit
  rebuild vs a thin fork-and-reshape (no numbers on the fork side yet).
- **2026-07-05** — **0.4.0 client surface modeled: read-state + background_task_count**
  (commit `22857d0`; composer-2.5 authored, cross-family review codex/gpt-5.5 + Opus,
  both LGTM). `background_task_count` (nullable i64) now on `SessionSnapshot` + the SSE
  `SessionEvent::Status`; `put_read_state` → `PUT …/read-state` (204, via new
  `send_no_content` helper — `check_status` only, since `decode_json`'s 2xx `from_str`
  chokes on an empty 204 body); `viewer_last_seen`/`viewer_unread` on `SessionSummary`.
  +4 lib tests. Gate: fmt · clippy(0) · 137 tests · drift clean. **Deferred from the
  0.4.0 bump:** #3 `GET /v1/harnesses` (dynamic harness registry) — no design needed,
  just follow the spec + eat pre-v1 churn when we wire the catalog; #4 runner-token /
  `/hooks/*` routes stay out (runner-side infra, not client API); ~25 leaked sessions
  in the server store (separate cleanup).
- **2026-07-05** — **omnigent pin bumped 0.3.0 → 0.4.0** (`bumping-the-omnigent-pin`
  runbook; tag `v0.4.0`, Source HEAD `31669e1b`). Small, clean contract delta:
  **+3 routes** (`/v1/harnesses`, `/v1/runners/{id}/token`,
  `/v1/sessions/{id}/read-state`), **+1 schema** (`ReadStatePutRequest`), and one
  SSE field — `background_task_count` added to `session.status`. No dropped
  schemas/routes, **no new SSE event type** (`taxonomy_drift` stayed green), and
  `regress` was already a dep (no new-regex work). Re-vendored to
  `vendor/omnigent-0.4.0/`, re-codegen'd `generated.rs` (114 schemas), bumped
  `PINNED_OMNIGENT_VERSION` + http/error test literals, re-grounded AGENTS/.agents/
  install-skill docs. Gate green: fmt · clippy(0 warn) · 133 tests · `no drift: 57
  client paths match`. Installed server reinstalled editable + daemon restarted
  → serves `0.4.0 (31669e1b)`. **Follow-ups:** (1) `background_task_count` is
  tolerated but not surfaced — decide whether `SessionEvent::Status` should carry
  it; (2) the 3 new routes are unmodeled in lens-client (no consumer yet);
  (3) ~25 leaked sessions persist in the server store across restart — separate
  cleanup.
- **2026-06-27** — **state-model concurrency RESOLVED + Sleep/Archive de-overloaded**
  (Opus opinion + GPT-5.5 doc edits across 9 docs; commit `cd474fa` +
  pump-terminology cleanup). Fixes the §8 single-writer contradiction *before* the
  reducer/session-store layer is built. **Decision:** `ActiveSession` actor
  (background **blocking OS thread**, per typed-client D2 — not tokio) owns
  canonical `SessionState` and is the single writer; `reduce()` is a **pure** fn
  returning `StreamUpdate` deltas (no I/O); `SessionStore` is the **foreground
  gpui replica** (read/observe only), never reduces. One seam, two directions
  (`StreamUpdate` out / `SessionCommand` in); optimistic `pending_user` is
  actor-owned. *Why:* gpui `Entity` mutation is foreground-only, so a
  store-as-writer would put `reduce` on the UI thread — forcing the off-thread
  actor + replica split. **Sleep ≠ Archive now:** Sleep = close observation +
  flush + best-effort `stop_session` (server owns runner/PTY); Archive = server
  `archived` flag via `PATCH` (visibility only) — resolves the dual-archived
  M8/T8 caveat. `SessionLifecycle` = `Active|Slept|Deleted`. `items` schema → PK
  by `item_id`, `ordinal` order, nullable `live_seq` hint (reconcile by `id`).
  Memory `state-model-single-writer-decision`. **This unblocks building the
  reducer/session-store (the next component).**
- **2026-06-27** — **omnigent pin advanced `0.3.0.dev0` → `v0.3.0`** (first real
  divergence-infra run; done inline, not subagent-driven). v0.3.0 shipped as a tag
  (commit `4edb4d95`; pyproject semver now a clean `0.3.0`). `xtask drift` flagged
  the delta; verdict = **not cosmetic, not breaking**: +5 additive routes
  (`/sessions/projects`, per-session `agent/mcp-servers`, `codex_goal`), +1 SSE event
  (`session.superseded`), and 6 "upstream dropped" paths that are all **hidden-not-removed**
  (`include_in_schema=False`; incl. the load-bearing `POST …/events`) — the exact
  ADR-0001 pattern. **Infra gap found:** `xtask drift`'s "removed" signal is a
  false-positive generator (diffs openapi presence, can't see hidden routes) — verify
  against route source before believing a removal. Re-vendored `vendor/omnigent-0.3.0/`,
  re-codegen (88→113 schemas). lens-client fixes: hand-authored `ElicitationResult`
  (dropped hidden-route schema, still contract); modeled `SessionEvent::Superseded`
  (+ MODELED list); added `regress` dep (new MCP `Name` pattern); bumped
  `PINNED_OMNIGENT_VERSION`→`0.3.0` (exact-match gate). 133 lib + 2 xtask tests, clippy/
  fmt clean, `drift` → no-drift. New skill `bumping-the-omnigent-pin` captures the
  runbook (weekly cadence); `installing-omnigent-from-source` re-grounded to the tag.
  Installed omnigent reinstalled editable to v0.3.0 (`4edb4d95`). **Cross-family review
  (codex / gpt-5.5) clean — "Findings: none"**; it cross-checked the hand-authored
  `ElicitationResult`, `session.superseded` modeling, version gate, and `regress` dep vs
  omnigent source. **Live-verify vs the v0.3.0 server: handshake + reachability + lifecycle
  green** (driven through a `codex-native` agent); the drive-a-turn `live_taxonomy` check is
  blocked by no network — codex runner `runner_failed_to_start` (offline on a plane), surfaced
  by lens-client as a typed error, not a contract miss. Retry the turn with connectivity to
  fully close. Memory: `omnigent-pin-bump-0.3.0`, `codex-as-reviewer`.
- **2026-06-27** — **Event-modeling branch (`feat/lens-client-event-modeling`) executed,
  final-reviewed, and MERGED to `main`** (fast-forward `82769b7..bb03992`, 12 commits; solo
  workflow — no PR, memory `integration-workflow`). 7 modeling tasks acted on the live recapture
  spike: typed arms for `session.agent_changed` / child `session.created` / `session.resource.deleted`
  (promoted from `DEFERRED`), exposed `child{}` on `child_session.updated` + elicitation `params`,
  flag-flips to byte-verified, §7 reconciled (terminal.activity is SSE). **Final whole-branch
  gpt-5.5 review → 1 fix wave** (commits `7eb90fb`+`bb03992`): the hand-written `Raw*` shapes were
  STRICTER than the generated contract (3 Important) → a contract-valid sparse/null payload would
  silently degrade to `Unknown`. Relaxed `RawChild` (open-dict → all fields `Option`),
  `RawElicitationParams` url/phase/policy_name/content_preview → `Option<String>` (null-tolerant) +
  contract-faithful `"form"` mode default, `RawSessionCreated` agent_id/parent_session_id →
  `Option<String>`; public getters/variant fields → `Option`; +3 sparse/null regression tests.
  gpt-5.5 diversity re-review: 3 target raws clean, caught the `mode` default + a null-test-coverage
  Minor (both folded). FINAL GATE: 133 lib tests, clippy `--all-targets --all-features` zero-warning,
  fmt clean, `xtask drift` green (55 paths), `generated.rs` untouched, no `Value` to consumers.
  **lens-client is now feature-complete on `main` through the recapture-driven event model.**
- **2026-06-26** — **Live event-surface recapture spike (Plan 4 #5) — CAPTURE DONE.** Drove the
  live pinned server headless via native harnesses (`omnigent claude`/`cursor`/`polly` — persistent
  runner survives the launcher, drive via subscribe-first + REST `message` injection) + a Cursor
  **SDK** API key for real reasoning deltas. Byte-verified 13 previously-unverified families
  (reasoning_text.delta, agent_changed, child_session.updated + child session.created,
  resource.deleted, model/effort/todos, compaction.in_progress, cancelled/interrupted,
  terminal.activity **via SSE not WS**, elicitation request+resolved via a `ask_on_os_tools` policy
  agent). Found 2 real under-modeled payloads (child_session drops `child{}`; elicitation drops
  `params`) + 2 deferred types needing typed arms (child `session.created`, `resource.deleted`).
  Still blocked by missing subscriptions: turn.*, response.created/queued, reasoning_summary,
  compaction.completed. Capture-only deliverable; modeling is a follow-on plan.
  [`spike`](./spikes/2026-06-26-live-event-recapture.md), memory `live-event-recapture-findings`.
- **2026-06-26** — **Consolidated lens-client review + Plan 4 (pre-consumer hardening) executed &
  complete.** After lens-client reached feature-complete (Plans 1–3c), ran a whole-crate review
  (gpt-5.5 cross-family **+ Opus architecture synthesis**) before building a consumer on it. Findings
  triaged into a hardening branch `feat/lens-client-hardening` (base `3dfadd9` off main `8a5a8b3` →
  `8fe4dd5`), executed subagent-driven (composer-2.5 build + per-task gpt-5.5 cross-family + Opus
  spot-check on the protocol task + one final whole-branch gpt-5.5 review). **5 tasks:** (1) fix
  phantom `ReasoningClosed` after mid-reasoning reconnect (`reset_transient` clears the open reasoning
  bracket too — real bug); (2) `connect_timeout` + per-request REST timeout (NOT on the SSE body) +
  `get_bytes` panic-free; (3) bounded `sync_channel` backpressure; (4) `EventStream::stop()`
  cooperative shutdown; (5) bootstrap emits `SnapshotRestored`+items like reconnect → reducer is the
  single writer on first open too (`run` split into `bootstrap`+`read_loop`; typed-client §7
  "Bootstrap" + app-arch §4.1 reconciled). Final review caught 1 scoped Important (stop()/bootstrap
  composition → scoped fix, not a try_send rewrite) + 2 doc Minors. 126 lib tests, clippy/fmt clean,
  `xtask drift` green (55 paths), `generated.rs` untouched, no `Value` to consumers. Ledger in
  `.superpowers/sdd/progress.md`.
- **2026-06-26** — **Plan 3c (contract-drift CI / B6) executed & complete — closes the Plan 3
  thread** (subagent-driven: composer-2.5 build + Opus per-task review + one consolidated gpt-5.5
  cross-family review; `087ef6f..8a7bb2e`, 5 tasks + 2 live-caught fixes + 1 review fix). Three
  layers: `xtask drift` (semantic path + SSE discriminator/shape diff vs sibling, `/hooks/*`-excluded),
  always-on offline `taxonomy_drift` (openapi mapping == `MODELED`(33)∪`DEFERRED`(14), disjoint),
  and gated `--features live-tests` `live_taxonomy` + `live_reachability`. **Live run executed vs a
  real `0.3.0.dev0` server — both gated tests green**; the reachability sweep **caught 2 real
  pre-existing bugs** (`HostObject` `id`→`host_id`; `SessionSnapshot` null-collection intolerance).
  gpt-5.5 review caught 1 Important (live taxonomy masked modeled-as-`Unknown` degradation → MODELED/
  DEFERRED split, re-verified live). 122 lib + 2 xtask tests, clippy/fmt clean, `generated.rs`
  untouched. Local `xtask`-only CI (D3). Memory: `plan3c-contract-drift-findings`.
- **2026-06-26** — **Plan 3b-2b (§7 no-replay reconnect state machine) executed & complete**
  (subagent-driven: composer-2.5 build + Opus per-task review + one consolidated gpt-5.5
  cross-family review; `3d4048b..6d4dde3`, 6 code tasks + fix wave + xtask fmt + docs). Reconnect
  lives in `stream::reader`, generic over a `Reopen` mock-able capability: backoff → snapshot →
  `/items` → re-open → synthetic lifecycle (`Reconnecting`/`Reconnected{gap:None}`/`SnapshotRestored`/
  `Disconnected{reason}`) + seq-deduped live tail. 119 lib tests, clippy/fmt clean. Cross-family
  review caught 1 Critical (opened body dropped on `/items` retry → reordered so `open_stream` is
  last fallible). §7 reconciled. ⚠ live reconnect smoke deferred (no server-kill harness). Next:
  Plan 3c contract-drift CI.
- **2026-06-26** — **Plan 3b-1 (§7a SSE normalization) executed & complete**
  (subagent-driven: composer-2.5 + per-task cross-family gpt-5.5; `2f9a46e..3b39412`,
  4 tasks + fix wave). `Normalizer` in the reader thread: `OutputItemDone` literal-re-fire
  suppression (preserves `in_progress`→`completed`) + synthetic `ReasoningClosed`
  (flagged not-byte-verified). 103 lib tests, clippy/fmt clean. Final review caught the
  `Err(_)`-path false-`ReasoningClosed` bug (fixed, reader now `io::Read`-generic +
  reconnect-ready). Two design calls pinned from the captured bytes: dedup = literal-re-fire
  only (relaxed §7a "exactly once"); build+flag `ReasoningClosed` rather than defer.
  Next: Plan 3b-2 reconnect (§7) — resolve the §7-vs-§11 reconnect-ownership ambiguity first.
- **2026-06-26** — **Plan 3 golden-SSE capture spike DONE** (live claude-sdk drive,
  subscribe-first, throwaway bash rig). 13 stream event types captured from bytes; 3
  undocumented events found; bucket A/B/C + seq-split confirmed; error family captured.
  Reasoning-delta + compact/elicitation/sub-agent/terminal blocked by the single-harness
  box (claude-sdk only) → schema-model the trivial reasoning deltas, defer the rest. Next:
  write the Plan 3 plan, model `ServerStreamEvent` from the captures.
- **2026-06-25 (eve)** — lens-client **REST surface 2a–2e executed** end-to-end
  (subagent-driven: composer-2.5 build, Opus per-task review, gpt-5.5 cross-family
  at seams + one consolidated 2c–2e review). 31 commits, 47 tests, live-verified.
  Review caught/fixed 4 real response-shape bugs. Cross-family review cadence
  relaxed to one consolidated pass mid-drive to conserve Cursor credits.
- **2026-06-25 (pm)** — omnigent contract-pinning decided (ADR-0001: freeze a
  commit, not the moving `0.3.0.dev0`; lock to release tags from `0.3.0`).
  Confirmed the "removed" elicitation/permission routes were only hidden from
  the openapi reference (`include_in_schema=False`), still ap-web-used → still
  contract. lens-client foundation brainstormed → spec
  (`typed-client-implementation.md`, decisions D1–D4: typify one-shot codegen;
  sync/blocking, no tokio; local xtask verification; coarse dev0 gate) → Plan 1
  written. Fixed two `typed-client.md` drifts (stale ~8 stream cap; async→sync).
- **2026-06-25** — Cargo workspace stood up (edition 2024, spikes/ vs crates/
  lint wall); omnigent pinned-source install + `installing-omnigent-from-source`
  skill; **transport-stability spike** (throwaway harness, Opus-spec →
  composer-2.5 build → gpt-5.5 review → live-run): validated cold-start, SSE
  parse/taxonomy, subscribe-first; confirmed daemon/runner lifecycle
  (server-lifecycle §3.1). Reconnect probes (P6/P7) next to close the §0.8 gate.
- **2026-06-24** — grilling pass + 11-doc walkthrough + first local renders;
  16 harnesses, lifecycle reshape (Sleep/Archive reclaim), cost two-axis,
  Concierge floating panel, Bridge Inbox layout, residency + notifications,
  new card design. → [`STATUS-ARCHIVE.md`](./STATUS-ARCHIVE.md)
