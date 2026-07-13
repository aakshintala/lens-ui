# Handoff — state-model P3-3a EXECUTED & COMPLETE → next is P3-3b — 2026-07-10

## TL;DR

**P3-3a (lifecycle core) is done and merged to `main`.** 21 commits, full `xtask gate`
green, D17 live-verify PASSED. The session lifecycle core ships: disk-canonical transcript,
actor-owned forward `/items` catch-up, transport-only reader, command sleep/wake, skeletal
`FleetScheduler`. **Next session: grill + plan P3-3b.**

- **Plan executed:** `docs/plans/2026-07-10-state-model-p3-3a-lifecycle-core.md`
  (8 tasks, all `- [x]`).
- **Spec SSOT:** `docs/specs/2026-07-08-state-model-engine-design.md` §2.3 (D19–D23).
- **Execution ledger (as-built, per-task + every review/fix wave):** `.superpowers/sdd/progress.md`
  — the P3-3a section is the authoritative blow-by-blow.
- **Memory:** `state-model-p3-3a-executed` (the golden-byte dual-id gotcha + as-built decisions).

## As-built — what changed vs the plan

The plan was followed faithfully; **five deviations/discoveries**, all forced by review and all
folded in:

1. **`ActorOutcome::Slept` is a UNIT variant** (not `Slept { last_seen_seq }`). The plan's Task 6
   Step 4 carried a pre-deletion leftover; `last_seen_seq` was deleted in Task 1 as vestigial
   (D19 makes the disk item-id frontier the resume cursor; the server has no seq-based resume).
   Controller override, user-confirmed at kickoff.
2. **Reconcile upsert split** (Task 3): the plan said "change the conflict clause to
   `ordinal=items.ordinal` everywhere; reconcile unaffected." **That was wrong** — reconcile parks
   ordinals negative then re-stamps; a preserve clause there would let `DELETE WHERE ordinal < 0`
   wipe rows. Composer caught it → `upsert_item_stmt_inner(preserve_ordinal: bool)`: commit path
   preserves, reconcile re-stamps. Verified by both reviewers.
3. **`call_id` supersession** (Task 3, the big one — see below): the commit-terminal-prefix design's
   locked precondition ("an in-progress function call completes before any later item finalizes;
   golden-capture true") was **FALSE against the golden bytes**. Fixed with a `call_id`-keyed
   supersession in `push_item`. **User decided the fix approach** (kept store-twin `fc_*` deferred).
4. **Defer-commit on `Reconnected` batches** (Task 4): the live tail must not commit before catch-up
   (else ordinal inversion once Task 5 removed the reader replay masking it). Main + nested paths.
5. **N1 fix** (whole-branch): replay buffered live events **before** deferred commands, so a `Sleep`
   deferred through catch-up rechecks quiescence against true state.

## The seam review earned its keep — 3 correctness bugs spec-vs-code review could not see

grok-4.5 reviewed Tasks 3/4/5 (per the plan's mandate + the user's extra grok pass on Task 4):

- **C1 — dual-id `function_call` stranding (CRITICAL, golden-byte-confirmed).**
  `docs/spikes/captures/2026-06-26-sse/happy_path.stream.sse` L38–50: the `in_progress`
  function_call (`fc_52f8…`) and its `completed` twin (`fc_5a32…`) have **different `id`, same
  `call_id`**, with a terminal `message` finalizing between them. `push_item` keys on `id`, so the
  in-progress twin becomes a **permanent non-terminal zombie** at the front of `state.items` →
  `commit_terminal_prefix` freezes forever (live disk transcript stuck at the first tool call; RAM
  working set unbounded — defeats D20). **Fix:** `push_item` supersedes a resident in-progress
  function_call by `call_id` when the completed twin (any different id) arrives — collapse to one
  item that flips terminal and commits, landing at wire-order position. Store-twin (`live fc_5a`
  vs `/items` store id) stays **P3-3b**.
- **C2 — re-fire ordinal gap (CRITICAL).** A far-back re-fire of a pruned id got a
  conflict-preserving upsert but still bumped `next_ordinal` → permanent ordinal gap + false
  watermark. **Fix:** commit upsert uses `RETURNING ordinal`; advance/watermark only when
  `stored == requested` (fresh insert), else pop without advancing.
- **Reconnected greedy-drain inversion (CRITICAL, Task 4).** On `Reconnected` the actor
  greedy-drained queued live events and committed them **before** running catch-up → live tail at
  low ordinals, catch-up history at higher ones (`item_0, item_3, item_1, item_2`). **Masked until
  Task 5 removed the reader `/items` replay.** **Fix:** defer the transcript commit on any batch
  containing `Reconnected` (main + nested buffered-replay paths) until after catch-up.
- **Two false-green tests** caught by grok and rewritten as true fail-pre/pass-post regressions
  (the reconnect greedy test's spawn-catch-up consumed the scripted page; the nested test's outer
  catch-up wrote all history before replay).

**Opus whole-branch review** then found **N1** (deferred-Sleep-rechecks-stale-state) — fixed +
grok-verified. All other findings triaged **OK-TO-DEFER**.

## D17 live-verify — RAN & PASSED (informational, never gated)

Against a live omnigent **0.5.1** server (`omnigent server start` + host daemon `host_e0b4c26`,
drove a headless `claude-sdk` turn → session `conv_8647debd…`, idle, 3 items):
- forward `/items?order=asc` → 3 items, `has_more=false`;
- `after=<first_id>` → 2 items (**cursor EXCLUSIVE** — the catch-up property, now live-confirmed,
  previously only openapi-confirmed);
- `StopSession` → HTTP 202 `{"queued":false}`;
- **post-stop forward `/items` → IDENTICAL 3 ids** — the transcript is durably re-fetchable via
  forward catch-up (the D17 claim). Codified in gated `crates/lens-client/tests/live_sleep_wake.rs`.
- **Live finding:** `StopSession` drives the server session `status` to `failed` (runner torn
  down) — irrelevant to D17 (our `lifecycle=Slept` is our own control-store field; the transcript
  persists regardless).

> A throwaway omnigent server was left running from the verify; `omnigent stop` to tear it down.

## Deferred to P3-3b (its own grilling + plan)

All documented, none block P3-3a:
- **Scaffold `fc_*` store-twin double-commit** — the live-committed `fc_5a` id vs the `/items`
  store-minted id differ for scaffold/native; catch-up could double-commit under two ids.
  `TODO(P3-3b, scaffold-id)` at `runloop.rs` frontier-seed + `commit_terminal_prefix`. The omnigent
  web-UI dedupes live-vs-`/items` by `call_id`/`itemId` at render (memory
  `omnigent-two-id-space-reconciliation` — the working reference).
- **N1-class hardening** — the deferred-command/buffered-replay interleaving is correct for Sleep
  now; a fuller command-ordering model is P3-3b.
- **Disk `RowSource` viewport/UI** (D23) — the focused replica reading `(last_rendered,
  committed_ordinal]` off `TranscriptStore`; windowed read, scroll-back paging, negative-ordinal
  prepend (D22 never-seen-huge). The renderer consumes `TranscriptAdvanced` (today an apply no-op).
- **`frontier()`-Err fail-loud** — currently seeds `next_ordinal=0` on error (non-silent via ring
  `PersistError`, and `UNIQUE(ordinal)` makes a mis-seed loud not corrupting; both reviewers judged
  the path effectively unreachable). Fail-closed (park/skip commits) is the hardening.
- **catch-up recursion→iteration** — `finish→invoke→replay→finish` recurses per buffered
  `Reconnected`; realistic backoff bounds depth, but convert to iteration.
- **`RunCtx` arg-bundling** — 5 `#[allow(clippy::too_many_arguments)]` in the runloop.
- Held-bubble resume, `SendLost` re-derivation, command-path `Auth403`/`NotFound` §9 escalation,
  parked-feeder drain (all inherited from P3-2 forward-notes).

## Process notes

- **cursor-async backend dies when the laptop sleeps** — grok jobs return empty `ERROR`. Fix:
  reconnect the MCP (`/mcp`) after a sleep. Fallback when grok is down: Opus Agent as the
  cross-family reviewer (per the P3-2 precedent).
- **grok's final findings block truncates in transport** — resume the session with "restate your
  complete verdict" to recover it. Happened on every grok seam review this session.
- **composer's scoped gate omits `cargo fmt --check`** — have it run `cargo fmt` (write) before
  committing, or the controller fmt-amends. Bit us on Tasks 1/2.
