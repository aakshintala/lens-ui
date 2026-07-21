# Lens — STATUS

Lean, living status for the Lens design effort. Keep this doc **small**: it holds
the current forward-looking state only. **Full dated session entries live in
[`STATUS-ARCHIVE.md`](./STATUS-ARCHIVE.md)** — write each session's detail there
and roll older "Recent" pointers off this page as they age.

_Last curated 2026-07-20 (B-2+B-3 DESIGN locked; container/culling spike DONE → GO; two plans B-2/B-3 next)._

---

## Next up

- **▶ Board B-2..B-6 (board-home)** — §4 board is now decomposed into **six specs B-1..B-6**
  (`docs/SPEC-GAPS.md` → "Board (§4) implementation specs"; supersedes the old B6/B7/B8 framing — B7
  "stable ordinal ordering" dissolved into B-1's ordinal slots, no separate sort task).
  **B-1 (data model & persistence) shipped 2026-07-18** (`8100cc8`; `lens-core` `BoardLayout` tree +
  `SqliteBoardStore`, schema v3). Remaining, in dependency order:
  - **B-2 + B-3 — DESIGN LOCKED 2026-07-20** (merged brainstorm; spec
    `docs/specs/2026-07-20-board-packing-and-group-rendering-design.md`; pixel SSOT
    `docs/design/renders/board-home.html` rewritten). **Model B (groups-as-tiles) + grid-snap**,
    **header-lane geometry** (uniform `[lane][body][gap]` cells → aligned, zero dead space, no overlap),
    `foot(n)` footprints, hole-backfill first-fit, full-size 280×160 members + ring-in-gap,
    **✓N-completed badge** (active count dropped), focused rail = 1-col vertical stacks. Cost still
    **derived at render** (Σ members' `cumulative_cost`).
    - **Container/culling SPIKE — DONE 2026-07-20 → GO** (all 4 unknowns; `spikes/board-container/`,
      verdict folded into spec §4/§8; memory [[board-container-spike]]). Absolute-positioned masonry in
      a stateful `overflow_scroll` div (explicit-content-height child = scroll extent) works; culling =
      absent-from-child-vec (gpui never builds culled tiles, 9/56 built); container-driven timer gate via
      `App::defer` (cards init HIDDEN) **retires the paint-time `last_bounds` gate + edge-trigger at the
      root** and fixes the scroll-into-view freeze; overdraw = 1×CELL_H; culling ~halves idle CPU.
    - **NEXT = write TWO plans** → `docs/plans/`: **B-2** (packer→lens-core + `board_tree` read-API +
      scroll container + cull + timer-gate + retire old gate) and **B-3** (group chrome + rollups +
      `group_of` seam).
    - Still owed: `board_tree(board_id)` ordered-walk read-API on `BoardLayout` (B-1 has only
      `children`); `group_of(&SessionCard)` seam. **Collapse (render+toggle) moved to B-4**; palette/picker
      to B-3.
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
    The B-2 spike **retires** this gate (container-driven visibility replaces the paint-time
    `last_bounds` gate + edge-triggered recovery) — see spec §4.3.
  - Grounding: specs `2026-07-18-board-data-model-persistence-design.md` (B-1) +
    `2026-07-20-board-packing-and-group-rendering-design.md` (B-2+B-3); handoff
    `docs/handoffs/2026-07-20-board-b2-b3-design-and-spike.md`; memories [[board-b1-executed]],
    [[board-b2-b3-design]].

- **▶ `lens-ui` transcript fan-out** — the first real consumer of the Detailed feed + the disk
  `RowSource`/D23 render window (markdown + virtualization spikes both GO). Plugs into the slot API the
  shell skeleton publishes; sibling parallel surfaces (terminal via `lens-terminal::open`, workspace,
  permissions) can fan out against `ContentTab`/`TabHandle`.
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
