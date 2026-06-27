# Lens — STATUS

Lean, living status for the Lens design effort. Keep this doc **small**: it holds
the current forward-looking state only. **Full dated session entries live in
[`STATUS-ARCHIVE.md`](./STATUS-ARCHIVE.md)** — write each session's detail there
and roll older "Recent" pointers off this page as they age.

---

## Open threads & next up

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
       single-page items replay. **Next: EXECUTE 3b-2b** (subagent-driven recommended — composer-2.5
       build + mandatory cross-family review at the temporal seams; `[[review-spend-policy]]`).
  3. **Stand up contract-drift CI** (outstanding B6) — the passive alarm that makes tracking
     dev0 safe when `0.3.0` eventually tags.
  - Plan 3b-2b is temporal/stateful (reconnect state machine), so **cross-family review stays
    mandatory** at the seams (`[[composer-delegation-profile]]`) — it caught the envelope bug in 3b-2a
    that author + green test both missed. (The earlier "composer is weak on temporal logic" claim was
    retracted as unsupported N=1.) Mind the Cursor-credit cost (`[[review-spend-policy]]`).
  - Now on branch `feat/lens-client-streaming` (off `main` @ `78fdaa3`).
- **Doc walkthrough complete** (all 11 design docs in `docs/design/` reviewed);
  every surfaced decision is resolved or consciously deferred.
- **Deferred, with a clean seam:**
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
