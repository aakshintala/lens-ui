# Handoff — Transcript T-1 (ViewBlock projection pipeline) executed

**Written:** 2026-07-21 · **Branch:** `lens-transript` · **HEAD:** `b595106` (**UNMERGED**; not pushed this session)
**Slice range:** `2ab0976..b595106` (4 build + 1 review-fix + 1 doc commit)
**Spec:** `docs/specs/2026-07-21-transcript-t1-viewblock-projection-design.md` (status: EXECUTED)
**Plan:** `docs/plans/2026-07-21-transcript-t1-viewblock-projection.md`
**Memory:** [[t1-viewblock-projection-executed]] · [[t0-turn-identity-executed]] · [[transcript-turn-identity-response-id]] · [[transcript-workstream-decomposition]]

## TL;DR

T-1 lands the **pure ViewBlock projection** — the render spine every transcript slice (T-2..T-7) and the
History view read from. New `crates/lens-core/src/reduce/view.rs`: a staged, borrow-only pipeline that
projects a session's canonical `&[Item]` (+ RAM-only `StreamScratch` + T-0's `active_response`) into
`Vec<ViewBlock>`. Code-complete, cross-family reviewed (codex gpt-5.6, converged with an independent Opus
pass), **gate green**, 21 tests. **Deliberately unmerged** — the user is driving T-1..T-7 on this branch
before merging to `main`. **T-2 is unblocked and is next.**

**Verify green before any follow-up:**
```bash
cargo run -p xtask -- gate                          # fmt + workspace clippy -D warnings + tests + drift. NO `cargo xtask` alias.
cargo test -p lens-core reduce::view                # the 21 T-1 unit tests
```

## What shipped

- **`crates/lens-core/src/reduce/view.rs`** (new, ~710 lines incl. tests) — the whole slice:
  - **`ViewBlock<'a>`** enum, 5 variants: `Item(&Item)` passthrough · `ToolSpan { call, output: Option }`
    (paired by `call_id`) · `WorkSection { response_id: &ResponseId, blocks }` · `StreamingReasoning(&ReasoningAcc)`
    · `StreamingMessage(&MessageAcc)`. Borrows everything; **no clone in the tree**.
  - **Staged pipeline** (§4): Stage-1 filters are the existing `transforms.rs` (`hide_reasoning`,
    `with_agent_changed_markers`, `only_agent`) — **untouched**. Stage-2 `project` / `project_filtered` /
    `project_all` = `pair_tool_spans` + streaming-tail splice. Stage-3 `group_work_section` = response-keyed fold.
  - **`pair_tool_spans`** — pairs `FunctionCall`↔`FunctionCallOutput` by `call_id`; span takes the call's
    position; orphan/duplicate outputs pass through as `Item` (never dropped/merged); output-before-call still
    pairs (global `consumed` set, order-independent); **first call per `call_id` claims the output, later
    same-id calls get `output: None`** (the review fix — preserves exactly-once).
  - **streaming-tail splice** (§5.2) — appends `StreamingReasoning` then `StreamingMessage` from `scratch`;
    reasoning tail is suppressed when the caller applied `hide_reasoning` (via `project_filtered(splice_reasoning=false)`);
    streaming is never grouped.
  - **`group_work_section`** (§5.3) — folds each response's consecutive agent-work run (`Reasoning`,
    `ToolSpan`, `NativeTool`, `AgentChanged`) into one `WorkSection` keyed on the shared `response_id`, EXCEPT
    the response `== active_response` (the live turn) which stays flat; when `active_response == None` (idle /
    disk-only paint) **all** responses fold. Siblings (`Message` user+assistant, `ResourceEvent`, `Compaction`,
    `Error`, `SlashCommand`, `TerminalCommand`) break runs and stay flat. `grouping_key` matches `ItemKind`
    **exhaustively — no wildcard** (a server-added kind is a compile error).
- **`reduce/mod.rs`** — `pub mod view;` + re-exports. **`lib.rs`** — crate-root re-exports of `ViewBlock`,
  `project`, `project_all`, `project_filtered`, `pair_tool_spans`, `group_work_section`.

## Deviations from design §3 (annotated as-built in `docs/design/conversation-transcript.md` §3)

`WorkSection` drops `open`+`meta`, keys on `response_id` (T-6 owns meta/expansion); Compaction/AgentChanged
markers ride as `Item` passthroughs; streaming variants borrow the whole accumulator (not `&str`);
`OptimisticUser` removed (composer-owned → T-7); `SubAgentSpan` removed (child-session model → T-5);
`ReconnectBreak` deferred (→ T-2). Rationale in T-1 spec §3.1.

## Review + verification

- **Build:** composer-2.5 (one dispatch for the cohesive single-file tasks 1–4 — the plan carried complete code).
- **Cross-family review (codex gpt-5.6, the mandated non-Claude path):** converged **exactly** with an
  independent Opus pass — 1 **Important**, 1 **Minor**, both **fixed** (`028ef68`):
  - Important — `pair_tool_spans` double-counted the output when two `FunctionCall`s shared a `call_id`
    (both spans paired the same first output, `consumed` suppressed the passthrough) → **violated §7
    exactly-once**. Fixed with a `paired: HashSet<&CallId>` gating the output to the first call.
  - Minor — the test named `user_message_and_resource_are_siblings_before_section` contained no
    `ResourceEvent`. Rewrote it to include one and assert it stays a flat sibling.
  - All other axes (exhaustive match, work grouping, filter consistency, borrow/clone discipline) clean.
- **No separate whole-branch review** — single new file, already cross-family reviewed + Opus-synthesized;
  disproportionate to re-run (per [[review-spend-policy]]). The Opus pass WAS the synthesis.
- **Gate:** green at `028ef68` (composer postcondition, exit 0); final `b595106` is docs-only markdown.

## Next up — T-2 (focused view scaffold + virtualized disk-sourced surface)

Spec **not yet written** (STATUS §"T-2"). T-2 is the **first real consumer** of `Vec<ViewBlock>`: mount the
focused `ContentTab`, virtualized transcript on native gpui `list()` (per [[transcript-virtualization-spike-2026-07]]),
disk-sourced via `RowSource`. **T-2 owns** (flagged in the T-1 spec §1/§8):

- **`ReconnectBreak` emission** — zero-field marker, needs reconnect timing (no backing item).
- **`ActiveResponseChanged` replica consumption** — T-0 produces the delta; T-2's RowSource consumes it to
  supply `active_response` to `group_work_section`.
- **Live in-progress tool sourcing** — in-progress `FunctionCall`s sit in `state.items`; whether the actor
  feed (detailed feed exposes scratch + committed-disk watermark, `actor/runloop.rs:~1177`) can *supply* them
  to the projector is T-2's problem. T-1's `ToolSpan { output: None }` contract is already correct as a pure fn.

**Recommended:** brainstorm → spec → plan (writing-plans) → SDD execute, same as T-1.

## Gotchas / carry-forward

- **Not pushed this session.** T-0's commits are on `origin/lens-transript`; T-1 (`8bccbad..b595106`) is local
  only. Push when ready (user's call — [[commit-when-finished]]: push is separate).
- **Merge coordination (design §9):** `terminal-ws` concurrently rewrites `reduce/mod.rs` → textual merge
  surface (T-1 only added `pub mod view;` + re-exports there). Logically independent; second-to-merge reconciles.
- **`project_filtered`'s `splice_reasoning` bool is the seam** for Stage-1↔streaming filter consistency — T-2's
  History-view caller (which runs `hide_reasoning`) MUST pass `false`, else live reasoning leaks past the filter.
- **Author-time plan review misses lifetime bugs:** the plan's test code used `Some(&ResponseId::new(..))`
  which won't compile under the projector's shared `'a`; composer correctly bound them to `let`s. Real code
  compiling is the only proof for borrow-lifetime shapes.
- **SDD ledger** at `.superpowers/sdd/progress.md` (git-ignored) records the T-1 loop as COMPLETE.
- **Gate invocation:** `cargo run -p xtask -- gate` — there is **no** `cargo xtask` alias.
- **codex reviewer hang:** run `codex exec ... < /dev/null` or it blocks on stdin ([[codex-as-reviewer]]).
