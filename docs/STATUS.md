# Lens — STATUS

Lean, living status for the Lens design effort. Keep this doc **small**: it holds
the current forward-looking state only. **Full dated session entries live in
[`STATUS-ARCHIVE.md`](./STATUS-ARCHIVE.md)** — write each session's detail there
and roll older "Recent" pointers off this page as they age.

_Last curated 2026-07-21 (transcript T-1 spec written; resliced T-1..T-7 — sub-agent promoted to T-5)._

---

## Next up

- **▶ Board B-2..B-6 (board-home)** — §4 board is now decomposed into **six specs B-1..B-6**
  (`docs/SPEC-GAPS.md` → "Board (§4) implementation specs"; supersedes the old B6/B7/B8 framing — B7
  "stable ordinal ordering" dissolved into B-1's ordinal slots, no separate sort task).
  **B-1 (data model & persistence) shipped 2026-07-18** (`8100cc8`; `lens-core` `BoardLayout` tree +
  `SqliteBoardStore`, schema v3). Remaining, in dependency order:
  - **B-2 — packing / scroll / culling** (NEXT): vertical scroll container + adaptive auto-fill grid over
    B-1's tree. Consumes an ordered walk — the `board_tree(board_id)` read-API is **not yet exposed**
    (B-1 has only `children(board_id, parent)`); add the visitor when B-2 needs it. The one remaining
    perf item — full-scale >8-card off-screen culling, never exercised on-device — rides here (no scroll
    container exists yet, so it's not a separate task).
  - **B-3 — group render + cost/count/age aggregation** (the mockup's `.gwrap` lanes: colored border,
    `$cost · Nd` header + "N done" pill). Cost is **derived at render** (sum members' `cumulative_cost`);
    B-1 stores none. B-3 owns the palette/picker (B-1 just persists the chosen `color_token`).
  - **B-4 — drag/move + context-menu grouping** — drives B-1's `move_item`/`ungroup`/`create_group`.
  - **B-5 — multiple boards + rail switcher** — board CRUD (B-1 seeds only the default board), the
    externally-discovered-session landing policy, and `FleetStore` connection-scoping.
  - **B-6 — archive-as-board surface.**
  - **Wiring gap:** B-1 is `lens-core` ONLY. The `lens-ui` `BoardView` still uses its placeholder
    ordering and is **not yet wired** to read `BoardLayout` / call `BoardStore` ops (spec §6 replica) —
    that consumer wiring rides with B-2/B-4.
  - **Heads-up (carried from the freeze fix) → now B-2's:** the scroll container is a *different* off→on
    transition than focus↔board. The current fix (`BoardView::recover_viewport_gates_on_reentry`) is
    edge-based on the focus↔board mode switch — a card scrolling back into view has **no mode change**,
    so it won't trigger the gate reset. B-2 needs its own scroll-driven gate reset or a revisit (paint-safe
    `on_next_frame` is clean on-device but a no-op in the test platform). Memory [[viewport-reentry-freeze]]
    + `docs/handoffs/2026-07-17-viewport-reentry-freeze.md`.
  - Grounding: spec `docs/specs/2026-07-18-board-data-model-persistence-design.md`, memory [[board-b1-executed]].

- **▶ `lens-ui` transcript fan-out** — the first real consumer of the Detailed feed + the disk
  `RowSource`/D23 render window (markdown + virtualization spikes both GO). Plugs into the slot API the
  shell skeleton publishes; sibling parallel surfaces (terminal via `lens-terminal::open`, workspace,
  permissions) can fan out against `ContentTab`/`TabHandle`. Product design is **complete**
  (`docs/design/conversation-transcript.md`, §1–§21); this workstream is **implementation decomposition +
  gpui/lens-ui specifics**, not product design.
  - **Decomposed 2026-07-20 (brainstorm) into build-slices T-1..T-7** (resliced 2026-07-21: sub-agent
    span promoted to its own slice T-5 — it's a child-*session* feature, not a depth transform — pushing
    turn-lifecycle→T-6, composer→T-7), each its own brainstorm→spec→plan→build cycle, dependency order
    below. Slices are *internal build increments* (like Board B-1..B-6) — the **surface** is not declared
    done until its closer lands. Two real surfaces fall out: **History view** (read-only transcript, no
    composer — §18, used for archived/sleeping sessions) is complete after **T-6**; **Chat column (full)**
    is complete after **T-7**. No functionality is deferred *out* of the workstream — the earlier
    "composer/interrupt/permissions belong elsewhere" framing was the error and is corrected: they are
    **T-7**, in-scope.
  - **T-0 — Authoritative turn identity (lens-core / lens-client).** *Prerequisite surfaced by the T-1
    cross-family review (2026-07-21).* Make the server **`response_id`** the single authoritative turn
    signal; today the catch-up path **discards** it — `wire_to_domain_item` (`actor/runloop.rs:221-233`)
    hard-codes `turn: 0` + fetch-time `created_at`, so disk-sourced history has no turn boundaries or real
    timestamps. Work: map wire `response_id` + `created_at` on catch-up (stop discarding); live-reduce
    stamps the active `response_id` (from `SessionEvent::Status`, `event.rs:51-54`) onto items; carry
    `response_id` on `BlockContext` (backing/replacing `turn: u32`); expose active `response_id` for
    liveness. Also fixes the "failed/incomplete/cancelled turns never bump turn" merge bug (a new
    `response_id` = a new turn regardless of end reason). **Prereq:** re-capture 0.5.1 `/items` to pin the
    field shape (0.3.0 capture shows field `ln` with user=`turn_` / agent=`resp_` / resource=`conv_`
    namespaces; 0.5.1 openapi says `response_id` required). Blocks T-1.
  - **T-1 — ViewBlock projection pipeline (pure).** §3/§4. Pure staged pipeline over `&[Item]` +
    `StreamScratch` → `Vec<ViewBlock>`; new `reduce/view.rs` in **lens-core**; exhaustive `ItemKind`
    match; no gpui, fully unit-testable. The spine. **SPEC WRITTEN 2026-07-21**
    (`docs/specs/2026-07-21-transcript-t1-viewblock-projection-design.md`); **cross-family reviewed**
    (Grok 4.5 + GPT-5.6) → revised; **plan in a new session.** Key resolutions: staged
    (filter→project→group) not uniform pipe; turn identity = authoritative **`response_id`** (from T-0),
    NOT a `scratch.turn` heuristic; `group_work_section` groups agent work by `response_id`, user messages
    + non-response items are ordinal-positioned siblings; liveness = turn's `response_id` == session active
    `response_id`; `WorkSection` drops `open` (render owns) and drops `meta` entirely (all fields need
    per-turn data → **T-6**); streaming variants carry `&MessageAcc`/`&ReasoningAcc` (stable identity);
    **`OptimisticUser` dropped** (pending is composer-owned → T-7); **`SubAgentSpan` dropped**
    (child-session model → T-5); `ReconnectBreak` emission → T-2.
  - **T-2 — Focused view scaffold + virtualized disk-sourced surface.** §16/§17. Mount focused `ContentTab`
    in `#chat-slot`; lift `RowSource` (id-keyed retained store) from spike to production; native
    `list()`/`ListState`/`ListAlignment::Bottom`; D23 disk-paint (finalized from `TranscriptStore`, live
    tail from actor scratch, id-keyed upsert, no below-watermark invalidation); wake/reconnect reconcile;
    scroll contracts (anchoring/windowing/jump-to-bottom) + "N new" pill. **Bucket-C dep** (`GET /items`
    tail pagination) is flagged here — small/medium sessions work without it.
  - **T-3 — Message & reasoning content.** §5/§6/§7. Vendor+patch gpui-component markdown (3 spots:
    debounce reset, `clear_selection` on reparse, `list_state.reset` scroll-jump); markdown-vs-verbatim
    channels + user backtick-gating; sanitization pre-pass; streaming safe-prefix / stable identity;
    reasoning + capped scroll region.
  - **T-4 — Tool spans, native tools, resource markers.** §8/§9/§12. Tool-span render (archetypes,
    truncation tiers, inline edit diff); native tools; §12 inline resource markers. **Bucket-B stubs
    live here** — "show full → editor/Review", "dock to Canvas", "open terminal" render **inert/disabled**;
    **no invented inline fallbacks** (they'd be ripped out by the real surfaces).
  - **T-5 — Sub-agent spans (child-session model).** §8.6. Sub-agents are child *sessions*
    (`session.child_session.created/updated`, linked by `parent_session_id`), **not** `ctx.depth` items —
    so this is a real feature, not a T-1 transform. Reducer folding of `child_session.*` into a
    parent↔child registry + live status; project `SubAgentSpan` at the spawn point; §8.6 render (collapsed
    span, peek, output-in-transcript); **navigate-into-child** shares the shell's session-focus machinery
    (the one cross-surface seam). Reuses T-4's span/output render. Prereq: reducer child-session fold.
  - **T-6 — Turn lifecycle, compaction, agent-changed, todos, minor items.** §4/§10/§11/§13/§14.
    Work-section collapse lifecycle (expand/override state — T-1 emits no `open`); the whole
    `WorkSectionMeta` (duration/model/tokens/cost/agent-transitions — T-1 emits none). **Prereq for the
    chip's model/token/cost:** model `response.completed.response.usage` — per-turn usage/model IS on the
    wire (`openapi.json:2573+`) but `ResponseEvent::Completed` is currently a unit variant that discards
    it; retain it per-turn. Compaction marker, AgentChanged marker, inline todos (forms 1–3), minor items,
    reconnect break. **← History view complete here.**
  - **T-7 — Composer & complete live turn (the chat closer).** §15/§18. Always-sends composer; optimistic
    user bubble (`⋯ sending` → settle on `session.input.consumed` → `⚠ failed·retry`); **Esc-interrupt**
    (+ new lens-core `Interrupt` command + lens-client call — server already echoes `session.interrupted`/
    `response.incomplete`); **permission/elicitation dock + widget integration** (reuse the GO elicitation
    spike; round-trip binary/form/url/plan/codex; emit `approval{action,content}` — **this workstream owns
    the integration**); **send-recovery** (never drop send text) + **input history** (up/down).
    **← Chat column (full) complete here.**
  - **Carry-forward arch notes:** a Summary-mode card consumer MUST tolerate occasional
    `Detailed(TranscriptAdvanced)` watermarks (catch-up/deferred-commit emit them regardless of mode).
    §3.5 Ready *policy* (seen_turn detector / `last_completed_at` stamp / per-card decay one-shot /
    focus-suppress) is lens-ui work over §3.4's `last_completed_turn`. Design spec REVIEW-CLOSED:
    `docs/specs/2026-07-14-lens-ui-shell-skeleton-design.md` — settled, don't re-litigate.

- **⏳ Terminal Slice 2 (interaction)** — planned + execution-ready on branch `terminal-ws`;
  **being executed by a separate agent.** Don't double-drive. Design = single-owner engine + one
  ordered command stream (memory [[terminal-slice-2-design-ghostty-precedent]]).

- **📋 SPEC-GAPS backlog** — independent, un-specced/partial items tracked in
  [`docs/SPEC-GAPS.md`](./SPEC-GAPS.md) (incl. #10 keyboard shortcuts + macOS app menu, Cmd+Q dead).

## Deferred, with a clean seam

- **lens-client modeling follow-on** — flip the 13 byte-verified SSE families `SCHEMA-DERIVED→MODELED`
  (capture done, memory `live-event-recapture-findings`); grow the two under-modeled payloads (`child{}`,
  elicitation `params`). Still-blocked families (`turn.*`, `response.created/queued`, codex reasoning)
  need a codex sub / OpenAI key.
- **lens-client small hardening** — `info.databricks_features: Value` leak; `ClientError::NotFound`
  rename + typed `Validation`/422; `/items` pagination; gated live-reconnect smoke.
- **WS terminal-attach client (Plan 7)** — no `terminal.rs`/`tungstenite` yet; workspace/terminal half
  of the contract is a known build-order deferral (converging with sibling `lens-terminal-ws`).
- **`session.superseded` reducer-drop** (`folds.rs:136` discards `target_conversation_id`) blocks
  terminal supersession-reattach — lens-core must surface it; terminal-integration-era.
- **Notifications v2** — server push for the fully-quit case (needs an upstream omnigent push channel).
- **Reducer normalization** — two status vocabularies (`SessionStatusValue` 6-val live vs
  `SessionStatus` 3-val snapshot) + two usage representations to normalize consumer-side.

## Open small decisions

- **Tunables (verification pass):** auto-sleep threshold (~10m), poll cadence (~10s), ring-buffer size
  (10 MB), transcript truncation tiers, `cost_samples` cadence.
- **Undecided UX:** terminal-`transfer` UX, managed-provider selection, policy/skill in-app authoring,
  multi-depth breadcrumb, exact-vs-range version pin.
- **Build artifact:** all status/harness/render glyphs are real Lucide SVGs (bell, triangle-alert,
  loader-circle, alarm-clock, check, moon, coffee, circle-dot, folder, git-branch). Only chrome
  furniture is still unicode — the kebab `⋮` and close `✕` (trivially swappable to `ellipsis-vertical`/
  `x` if/when a fully-bespoke set is wanted).

## Recently shipped (all on `main` unless noted)

- **Board B-1 — data model & persistence (2026-07-18):** `lens-core` `BoardLayout` recursive
  Board→(Card|Group) tree + `SqliteBoardStore` (control-tier `lens.db`, schema **v2→v3** additive, lazy
  placement no backfill), ordinal-slot placement, mutation ops (place/move/ungroup/group/archive/…),
  bidirectional startup reconcile (lazy-place live, prune tombstoned). Adversarial review (grok-4.5 +
  probe tests; grok's "HIGH id-collision" refuted empirically) → 5 hardening fixes (high-water-mark id
  seed, tombstone place-guard, cycle seen-guard, deterministic reconcile order, +7 tests). 30 board
  tests, full `xtask gate` green. Committed **`8100cc8` (UNPUSHED)**. Spec
  `2026-07-18-board-data-model-persistence-design.md`; memory [[board-b1-executed]]; handoff
  `docs/handoffs/2026-07-18-board-b1-executed.md`.
- **Wave build B1–B5 + follow-ups (2026-07-17):** Lucide glyph tiles, context pbar, Slept/Wake/Retry
  seams, `loader-circle` spinner, canvas `paint_path` sweep, Scheduled countdown, viewport-gated
  20fps/1Hz anim driver, `demo` feature-gate; on-device visual pass; per-wave card-body wash; header
  3-tier type + host pill + per-wave activity line; **perf 30→20fps** (~35% CPU, `wave-perf-fps-attribution`).
  Spec `2026-07-17-wave-behaviors-design.md` §11. Handoff `2026-07-17-wave-build-visual-pass-merged.md`.
  - **Viewport re-entry freeze — RESOLVED (2026-07-17):** focus→board no longer freezes the off-screen
    card's spinner/pulse. Reset lives in `BoardView`'s fleet-observe effect; 3 regression tests; codex
    review addressed. Memory [[viewport-reentry-freeze]]. **Unpushed on `main`** (see below).
- **§18 Theming substrate (2026-07-16):** `crates/lens-ui/src/theme/` — `LensTheme` global (base+status
  tokens, hex↔Hsla serde, dark+light JSON), `cx.lens_theme()`, gpui-component bridge, external-file load
  + `cmd-shift-t` reload, `shortcuts.rs`. **On `main`, load-bearing for the cards.** Palettes tuned
   during the 2026-07-17 §11 on-device visual pass (bg ramp, wave status colors, context-bar thresholds,
   per-wave wash intensities) — no longer placeholders; residual fine-tuning is cheap via the reload
   loop. Memory [[lens-ui-theming-fork]].
- **`lens-ui` shell skeleton Plan 2 + card/board audit (2026-07-15/16):** §4–§7 skeleton merged; wave
  colors un-swapped, Needs-input=orange, icon-tile readout. Gate now covers lens-ui/lens-app.
- **lens-core §3 ActorFeed gate (2026-07-15):** unified `ActorFeed` FIFO, scheduler dual-mode,
  seed-on-spawn + emit-on-Demote, enriched `SummaryUpdate`. Grok-authored plan, subagent-driven.
  Memory [[grok45-as-plan-author]].
- **state-model engine P0–P3 (2026-07-08 → 07-12):** domain types → pure reducer → two-tier SQLite
  persistence → actor + store + commands + P3-3a/b lifecycle. All merged. Memories `state-model-*`.
- **lens-client (2026-06-25 → 07-10):** REST surface (Plans 2a–2e), SSE event modeling (Plan 3 series),
  benchmarks, pre-consumer hardening (Plan 4), omnigent pin `0.3.0.dev0 → 0.5.1`. Memories `plan3*`, `plan4*`.

## Housekeeping

- **`main` is AHEAD of `origin` by 5 (unpushed, as of 2026-07-18):** `759eb3a` (status fix) ·
  `c855ab6` (SPEC-GAPS §4 board → B-1..B-6) · `c21e669` (docs relocate → specs/plans) · `8100cc8`
  (B-1 board data model) · this docs-status commit. `origin/main` is at `b8727ab`. Push decision
  deferred to the user.
