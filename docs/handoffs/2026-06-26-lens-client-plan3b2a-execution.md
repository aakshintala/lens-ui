# Handoff — lens-client Plan 3b-2a executed; 3b-2b starts at a design decision

**Date:** 2026-06-26
**Branch:** `feat/lens-client-streaming` (green; not yet PR'd)
**Context:** Plan 3b-2a (typed reconnect *read* surfaces) is executed end-to-end
and reviewed. This doc hands off to **Plan 3b-2b** (the reconnect *state
machine*), which is **not yet an execute task** — it opens on a design decision
(below), not on code.

Ground truth used: omnigent `0.3.0.dev0` (`36b2a11c`); golden captures at
`docs/spikes/captures/2026-06-26-sse/{happy_path.items.json,happy_path.snapshot.json}`.

---

## What shipped (Plan 3b-2a — done)

Plan: `docs/plans/2026-06-26-lens-client-plan3b2a-reconnect-reads.md`.
Subagent-driven: composer-2.5 build (4 tasks, each red→green→commit), one
consolidated gpt-5.5 cross-family review at the end.

Full `lens-client` lib suite: **110 passing**, clippy `-D warnings` + fmt clean.

| Commit | What |
|--------|------|
| `1360819` | the plan (written) |
| `3a05015` | plan edit — defer `last_task_error` (type-ambiguous null) |
| `e2767a7` | Task 1 — `stream::Item` union completed: `ResourceEvent` variant, `id` on `Other`, total `Item::id()` accessor, `from_value` → `pub(crate)` |
| `8b65529` | Task 2 — `Sessions::items()` + typed `ItemList` envelope (`GET /v1/sessions/{id}/items`) |
| `f6c7771` | Task 3 — `SessionSnapshot` bucket-B scalars + `ModelUsage`/`SkillRef` |
| `8315ead` | Task 4 — `SessionSnapshot` bucket-B collections (`usage_by_model`, `skills`) + embedded `items` |
| `2ff93c3` | review fix — hoist snapshot embedded-item `data` envelope before typing |

**The one real bug (review caught, the plan missed):** `GET /items` and the live
stream carry item payload **flat at top level**; `SessionSnapshot` embedded
`items` nest it under a **`data` envelope**. The first `de_items` fed the wrapped
form straight to `Item::from_value` → items kept `id` but silently defaulted
`role`/`content`/`name`/`event_type`. Fixed by hoisting `data` up first; the Task
4 test now asserts typed payload, not just `len`/`id`. Persisted as memory
`plan3b2a-embedded-item-envelope.md`. **Lesson for 3b-2b:** model each endpoint's
*own* capture; assert a real typed field, never just id/len.

## Deferred (intentionally not modeled — known shapes, byte-grounded gaps)

Left out of `SessionSnapshot` because empty/null or type-ambiguous in the only
capture (serde skips unknown wire fields, so the read still parses when present):
- `last_task_error` — null here, but sibling `ChildSessionSummary` models it
  `Option<BTreeMap<String,String>>` (`sessions.rs:309`); `Option<String>` would
  risk a live deser break. Model when a non-null shape is captured.
- `todos` (wire key `activeForm`; `TodoItem` not `Deserialize`),
  `pending_elicitations` (likely objects, not id strings), `model_options`,
  `sandbox_status` (null). Model when non-empty bytes exist.

---

## Next: Plan 3b-2b — resolve the design decision FIRST

**3b-2b is the temporal/stateful half** and has **no written plan yet** because
one open design question gates it. Start the next session on this decision (Opus
brainstorm/design), not on coding.

### Open decision — bucket-B chrome restore on reconnect (ownership)
When the stream reconnects and we re-read the grown `SessionSnapshot`, **who
applies the bucket-B chrome to the UI model?**
- **Option A:** the crate emits synthetic chrome `SessionEvent`s (uniform event
  stream; consumer stays event-driven).
- **Option B:** the consumer applies the snapshot directly (simpler crate; splits
  the update path between events and snapshot-apply).

Resolve A vs B, record it (ADR-style) in `docs/design/typed-client-implementation.md`
§7, *then* write the 3b-2b plan.

### 3b-2b scope (from Plan 3b-2a "Out of scope")
- §7 reconnect **state machine**: disconnect detect at the reader's `Err(_) =>
  return` seam, exponential backoff (~7s), synthetic
  `ServerStreamEvent::{Reconnecting{attempt}, Reconnected{gap}, Disconnected}`.
- **Items-replay:** convert this plan's `ItemList` → replayed
  `ResponseEvent::OutputItemDone`; **`Reconnected` precedes all replayed history**
  (§7a ordering).
- **Seq-dedup** of the live overlap + **normalizer `seen_items` reset** on
  `Reconnected { gap != Some(0) }` (the two seams recorded in typed-client §7).
- **Reader architecture change:** reader thread gains a re-open capability
  (`Client` + `SessionId` or a reopen closure) so it can drive snapshot / items /
  re-open internally.
- Chrome restore wiring — per the decision above.

---

## Pointers
- Plan (3b-2a): `docs/plans/2026-06-26-lens-client-plan3b2a-reconnect-reads.md`
- Design §7 (reconnect/wake protocol) + §7a (ordering): `docs/design/typed-client-implementation.md`
- State model §6.3 (reconcile-by-id / wake): `docs/design/app-architecture-and-state-model.md`
- Captures: `docs/spikes/captures/2026-06-26-sse/`
- Relevant memory: `plan3b2a-embedded-item-envelope.md`,
  `plan3b1-normalization-decisions.md`, `composer-delegation-profile.md`,
  `review-spend-policy.md`

## Process notes carried forward
- Reads were composer-2.5's wheelhouse (static, byte-grounded) — clean first-try.
  3b-2b is temporal/stateful; spec it concretely (exact ordering, what's
  live-tail/no-replay, what resets on reconnect) as for any implementer.
- Cross-family review (gpt-5.5/gemini) earns its keep — it caught the envelope
  bug the author + a green test both missed. Keep one consolidated review per
  drive (review-spend-policy).
