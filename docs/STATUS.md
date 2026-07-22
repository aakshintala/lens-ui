# Lens — STATUS

Lean, living status for the Lens design effort. Keep this doc **small**: it holds
the current forward-looking state only. **Full dated session entries live in
[`STATUS-ARCHIVE.md`](./STATUS-ARCHIVE.md)** — write each session's detail there
and roll older "Recent" pointers off this page as they age.

_Last curated 2026-07-22 (**B-4a store→replica write-path EXECUTED** on branch `board-b4a` — 10 TDD tasks, subagent-driven (composer implementers + codex per-task + final whole-branch review); gate green; on-device FPS confirmation owed to user; awaiting merge decision. Earlier: B-3 SHIPPED merged-unpushed; B-4a design LOCKED then planned+codex-reviewed)._

---

## Next up

- **▶ Board B-2..B-6 (board-home)** — §4 board is now decomposed into **six specs B-1..B-6**
  (`docs/SPEC-GAPS.md` → "Board (§4) implementation specs"; supersedes the old B6/B7/B8 framing — B7
  "stable ordinal ordering" dissolved into B-1's ordinal slots, no separate sort task).
  **B-1 (data model & persistence) shipped 2026-07-18** (`8100cc8`; `lens-core` `BoardLayout` tree +
  `SqliteBoardStore`, schema v3). Remaining, in dependency order:
  - **B-2 — packing/scroll/culling SHIPPED 2026-07-21** (`db5b7c2..14b474c`, 10 commits, merged to
    main **unpushed**). `lens-core::pack` pure packer (`foot`/`pack`/`cols_for_width`/`intersects_band`);
    `BoardLayout::board_tree` ordered group-aware walk (skips archived); `lens-ui` absolute-masonry
    `overflow_scroll` container (both board N-col + focus rail 1-col via one `pack_and_render`) with
    band-culling; **container-driven visibility gate** (cards init HIDDEN, `set_visible` via `App::defer`)
    that **retired** the paint-time `last_bounds` gate + `recover_viewport_gates_on_reentry` + `last_mode`
    and fixes the scroll/re-entry freeze at the root. **Basis B (locked):** the packer walks an in-memory
    `BoardLayout` fabricated from `FleetStore` by a PROVISIONAL `build_ephemeral_layout` stub — B-4 deletes
    it when it lands the persisted store→replica seam with the first writes. Subagent-driven build: 6 tasks,
    cross-family review each (codex gpt-5.6), Opus whole-branch review **READY**; `xtask gate` green;
    release demo launches clean (live gate confirmed: animating cards tick, Slept frozen). Memory
    [[board-b2-executed]]; plan `docs/plans/2026-07-21-board-b2-packing-scroll-culling.md`; handoff
    `docs/handoffs/2026-07-21-board-b2-executed.md`.
  - **B-3 — group chrome & rollups SHIPPED 2026-07-21** (`3045590..75b78bb`, 7 commits, merged to main
    **UNPUSHED**). Filled the B-2 placeholder arm with real chrome: `board/rollup.rs` pure `GroupRollup`
    fold (Σspend / oldest-`created_at` age / `completed_count`) + formatters; `group_accent` token→color
    resolver (4 SSOT accents + neutral); `absolute_group` renders ring+accent+7%-tint + header-lane
    (`● dot · name · spend·age · ✓N · ⌄`) folded from member cards; `created_at` plumbed onto `SessionCard`
    (Detailed/Rebased); `test_layout` injection seam + `group_chrome_for_test` hook drive a fixture
    integration test (the group path is not runtime-reachable under basis B). Subagent-driven (composer-2.5
    implementers, codex gpt-5.6 cross-family review of board logic = clean + 1 Minor age-overflow fixed,
    Opus whole-branch review = **SHIP**). `xtask gate` green. **Design deviations from the spec bullet:**
    (a) the `group_of(&SessionCard)` seam was NOT built — group membership is threaded as `GroupMeta`
    through `pack_and_render` from the `board_tree` walk, so a card-keyed reverse lookup is unnecessary;
    (b) `✓N` renders `completed_count: 0` — the real Archive-side count wires in **B-6**. Plan
    `docs/plans/2026-07-21-board-b3-group-chrome-rollups.md`. **3 Minors carried into B-4** (Opus review):
    the render-dead `group_header_text`/inline-header duplication (add a live rendered-chrome assertion,
    render from one source); the integration test proves data-wiring not pixels (correct under
    NoopTextSystem — B-4 adds the live check); spec §3 fidelity nits (`.border_1()` 1px vs 1.5px; flat
    wash vs glow/vignette — gpui 0.2.2 has no radial gradient, [[wave-card-body-wash]]).
  - **B-4 — drag/move + context-menu grouping — decomposed into B-4a…B-4d.** At design time B-4 was split
    into a **foundation slice B-4a** (store→replica write-path; NO interactions) + interaction follow-ons
    B-4b (collapse + §7 collapsed-tile) / B-4c (drag/move) / B-4d (context-menu grouping).
    - **B-4a — store→replica write-path foundation — EXECUTED 2026-07-22** on branch `board-b4a`
      (base `0f18ea7`, 20 commits). Plan `docs/plans/2026-07-22-board-b4a-store-replica-write-path.md`
      (v2, codex-reviewed REWORK folded). Subagent-driven: composer-2.5 implementers + codex gpt-5.6
      per-task cross-family review + final whole-branch review + Opus controller adjudication. `BoardReplica`
      (`board/replica.rs`, ~930 lines) = in-memory `BoardLayout` + serialized single-in-flight off-thread
      `run_op` pump (`cx.spawn`→`background_executor().spawn`→`WeakEntity::update`), typed BUSY-retry w/
      backoff, recovery force-reopen, suppress-stuck reconcile (no tombstone loop), deterministic
      session-sorted placement; retired `build_ephemeral_layout` + `test_layout` seam; non-blocking
      `ReplicaState` banner; demo seeds a "Demo group" (B-3 chrome live). **Reviews caught (all fixed):**
      ~10 false-green tests (composer blind spot → controller load-bearing rewrites w/ sabotage-verify),
      the C1 tombstone infinite-loop (self-introduced by re-diff-on-reply), a buggy `gate_epoch` composer
      over-reach for a non-existent race, and non-deterministic HashMap placement (flaky acceptance).
      **Perf:** `board_tree` bench 11.8µs @ 1000+group; at-scale demo (`LENS_DEMO_N=125`) launches stable;
      **MANDATORY on-device FPS-at-120 confirmation OWED (headless can't measure — run
      `LENS_DEMO_N=125 ./target/release/lens-app --demo` on a display).** Gate green (clippy -D warnings,
      fmt, lens-core 254 / lens-client 150 / lens-ui 83 lib + 5 acceptance). Memory [[board-b4a-plan-executed]].
      **MERGED + PUSHED 2026-07-22** (board-b4a FF→main, `4d31c9d..c189d4c`, incl. previously-unpushed B-2/B-3). NEXT interaction slices B-4b/c/d; **B-4d blocker:** non-idempotent-retry commit-phase tracking (design §8 seam). Original design spec:
      `docs/specs/2026-07-21-board-b4a-store-replica-write-path-design.md` — grilled + gpt-5.6 codex
      spec-review folded + §3 re-grilled. Replaces `build_ephemeral_layout` with a persisted `BoardLayout`
      via a new `BoardReplica` gpui entity; **off-thread store access** (`Arc<Mutex>` + `cx.background_spawn`
      behind a serialized single-in-flight `run_op`; renders read the in-memory replica, never SQLite) —
      the codex review caught that inline SQLite violates AGENTS.md's MANDATORY off-thread rule. Conn pinned
      to the app `Connection.id` (`"lens-app"`) so FleetStore placement converges with `load_layout`'s
      sessions-table reconcile; explicit `ReplicaState` (Loading/Writable/Degraded/LoadFailed/Stale) + always-
      allowed recovery `Load` + non-blocking banner; demo seeds a group (B-3 chrome renders live for the first
      time); MANDATORY frame-budget benchmark = E2E lens-ui on-device measurement (not just the pure lens-core
      pack bench). Verifies the B-3 `.cached()` member-read-during-render carryforward now that groups render
      for real. Memory [[board-b4a-design]]; handoff `docs/handoffs/2026-07-21-board-b4a-design-locked.md`.
    - **B-4b/c/d** — collapse (+§7 collapsed-tile), drag/move (spike candidate: gpui `on_drag`/`on_drop` vs
      packer geometry), context-menu grouping. Each adds `write()` op variants via B-4a's `run_op` seam.
  - **B-5 — multiple boards + rail switcher** — board CRUD (B-1 seeds only the default board), the
    externally-discovered-session landing policy, and `FleetStore` connection-scoping.
  - **B-6 — archive-as-board surface.**
  - **Wiring gap (partly closed by B-2):** `BoardView` now reads a `BoardLayout` (via `board_tree`) and
    renders from it — but under **basis B** that layout is the ephemeral `build_ephemeral_layout` stub, NOT
    the persisted `SqliteBoardStore`. The real store→replica wiring (spec §6) + all board **write** paths
    ride with **B-4**, which deletes the stub.
  - **Freeze RESOLVED by B-2:** the scroll-into-view / focus↔board re-entry freeze is fixed at the root —
    the container-driven visibility gate (cards init HIDDEN, `set_visible` via `App::defer`) replaced the
    paint-time `last_bounds` gate + `recover_viewport_gates_on_reentry`. [[viewport-reentry-freeze]] closed.
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

- **✅ Fixed 2026-07-21 — turn counter only bumped on `response.completed`.** Cancel/incomplete turns
  now bump `state.stream.turn` so the card flashes `Wave::Ready` ("just finished"). As-built (differs
  from the original handoff shape after two codex reviews): **Incomplete/Cancelled** bump the counter;
  **Failed** does NOT (it surfaces via `Wave::Failed`, and status is not folded atomically with the
  event — bumping would flash a transient green). All three **discard** open scratch (not finalize) —
  committing a synthetic local partial would permanently duplicate omnigent's durable `interrupted`
  `/items` row (messages reconcile by `item_id` only). `crates/lens-core/src/reduce/{mod,folds}.rs`;
  handoff `docs/handoffs/2026-07-21-turn-counter-non-completed-terminal-bug.md` (as-built appended);
  memory [[turn-counter-noncompleted-bug]]. **Live-verified 2026-07-21:** interrupted a streaming
  claude-sdk turn vs omnigent 0.5.1 — the partial is flushed via `response.output_item.done` (durable
  `/items` row under a **server** id) BEFORE `response.cancelled`, so the reducer commits it under that id
  and the Cancelled-arm discard is a no-op for the message → partial preserved, no loss, no duplicate
  (discard validated; the omnigent source's "Phase 2 TODO / not persisted" docstring is wrong). **One
  follow-up remains:** native `turn.completed/failed/cancelled` are deferred → `Unknown` → ignored, so the
  same bug persists on the native-runner surface. Merge-collision with T-0
  (`lens-transript`) in the same `reduce` match block stands — logically independent, textual merge only.

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

- **Board B-3 — group chrome & rollups (2026-07-21):** filled the B-2 group placeholder with real chrome —
  `board/rollup.rs` pure `GroupRollup` fold (Σspend / oldest-`created_at` age / `completed_count`) +
  formatters; `group_accent` token→color resolver; `absolute_group` ring+accent+7%-tint + header-lane folded
  from member cards; `created_at` plumbed onto `SessionCard`; `test_layout` injection seam + `group_chrome_for_test`
  hook + fixture integration test (path runtime-dormant under basis B). `group_of` seam dropped (membership
  threaded as `GroupMeta` via `board_tree`); `✓N`=0 until B-6 Archive source. Subagent-driven (composer-2.5,
  codex gpt-5.6 board-logic review clean+1-Minor-fixed, Opus whole-branch SHIP). `xtask gate` green. **`3045590..75b78bb`,
  merged to main (UNPUSHED)**. Plan `docs/plans/2026-07-21-board-b3-group-chrome-rollups.md`.
- **Board B-2 — packing/scroll/culling (2026-07-21):** `lens-core::pack` pure packer + `board_tree`
  walk + `lens-ui` absolute-masonry `overflow_scroll` container (board N-col + rail 1-col via one
  `pack_and_render`) with band-culling + container-driven visibility gate that retired the paint-time
  `last_bounds` gate/`recover_viewport_gates_on_reentry`/`last_mode` (freeze fixed at root). Basis B:
  ephemeral `build_ephemeral_layout` stub feeds the tree (real store→replica = B-4). Subagent-driven
  (6 tasks, composer-2.5 implementers, codex gpt-5.6 per-task review, Opus whole-branch **READY**);
  `xtask gate` green; release demo launches clean. **`db5b7c2..14b474c`, merged to main (UNPUSHED)**.
  Memory [[board-b2-executed]]; handoff `docs/handoffs/2026-07-21-board-b2-executed.md`.
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
