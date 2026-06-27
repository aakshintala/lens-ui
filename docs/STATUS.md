# Lens — STATUS

Lean, living status for the Lens design effort. Keep this doc **small**: it holds
the current forward-looking state only. **Full dated session entries live in
[`STATUS-ARCHIVE.md`](./STATUS-ARCHIVE.md)** — write each session's detail there
and roll older "Recent" pointers off this page as they age.

---

## Open threads & next up

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
    `docs/superpowers/plans/2026-06-27-lens-client-benchmarks.md`. **Follow-up:** grow
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
     [`docs/superpowers/plans/2026-06-26-lens-client-plan3a-sse-transport.md`](./superpowers/plans/2026-06-26-lens-client-plan3a-sse-transport.md)):
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
       [`plan`](./superpowers/plans/2026-06-26-lens-client-plan3b1-normalization.md)). A pure
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
       edit + 1 review fix; [`plan`](./superpowers/plans/2026-06-26-lens-client-plan3b2a-reconnect-reads.md),
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
       [`plan`](./superpowers/plans/2026-06-26-lens-client-plan3b2b-reconnect-state-machine.md) — 7 TDD
       tasks (synthetic variants → `Normalizer::reset_seen_items` → frame seq-peek → `reconnect` module
       `Reopen`-trait/`HttpReopener`/backoff/items-replay → state machine in reader → wire
       `Sessions::stream` → docs). The `Reopen` trait makes the state machine unit-testable with a
       scripted mock (no server). Four plan-level decisions flagged for review + §7 reconciliation:
       `Disconnected{reason}` payload, `gap:None` v1 (no `Some(0)` proof), frame-level seq-dedup,
       single-page items replay.
     - **Plan 3b-2b EXECUTED & COMPLETE** (2026-06-26, subagent-driven: composer-2.5 build + Opus
       per-task review + one consolidated gpt-5.5 cross-family review; commits `3d4048b..6d4dde3`,
       6 code tasks + 1 review fix wave + xtask fmt housekeeping + docs;
       [`plan`](./superpowers/plans/2026-06-26-lens-client-plan3b2b-reconnect-state-machine.md),
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
     [`plan`](./superpowers/plans/2026-06-26-lens-client-plan3c-contract-drift.md)). The passive
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
  - **Markdown renderer** — the one real build risk (hand-rolled
    `pulldown-cmark`→gpui + sanitization; framework §4.1).
- **Tunables for the verification pass:** auto-sleep threshold (~10m), poll cadence
  (~10s), ring-buffer size (10 MB), transcript truncation tiers, `cost_samples` cadence.
- **Small undecided UX:** terminal-`transfer` UX, managed-provider selection,
  policy/skill in-app authoring, multi-depth breadcrumb, exact-vs-range version pin.
- **Build artifact:** render icons are unicode placeholders — ship a real
  status + harness-provider icon set.

## Recent

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
