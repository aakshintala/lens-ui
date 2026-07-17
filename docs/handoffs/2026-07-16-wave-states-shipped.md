# Handoff — `Scheduled` + `AwaitingReview` wave states SHIPPED (structural contract); next = B1–B5 wave build

**Date:** 2026-07-16 (later session)
**Branch:** `feat/lens-app-multi-session` @ `f1f0d6e` — **branch-only, NOT merged/pushed** (user's call).
**Gate:** GREEN (`cargo run -p xtask -- gate`, exit 0). lens-ui now 38 lib + 1 acceptance tests.

Picks up from `docs/handoffs/2026-07-16-theming-substrate-shipped.md`. That handoff's
"NEXT: wave build (B1–B5)" is **still the next workstream** — this session added two
new wave *states* to that build's scope (their structural contract only; behaviors
fold into B1–B5). The B1–B8 deviation ledger remains
`docs/handoffs/2026-07-16-lens-ui-theming-and-card-audit.md`.

---

## What shipped this session

Design → build for **two new card wave states**, brainstormed then composer-built.

**Design (`b7c747a` + `ff9ac7c`):**
`docs/superpowers/specs/2026-07-16-wave-states-scheduled-awaitingreview-design.md`.
Wave-side contract only. Producers filed as **`SPEC-GAPS.md` #11**.

**Implementation (`f1f0d6e`)** — composer-2.5 author, codex/gpt-5.6-sol review (ran
tests, 38 pass, no defects), gate green:

- **`Wave` 6 → 8**: added `AwaitingReview`, `Scheduled` (`crates/lens-ui/src/card/wave.rs`).
- **Two Lens-owned `SessionCard` fields** (`card/model.rs`): `awaiting_review: bool`,
  `scheduled_wake_at: Option<i64>` (epoch ms). Both default false/None in `::new`.
  `scheduled_wake_at` is **source-agnostic on purpose** — the B-seam (a future
  forwarded omnigent `scheduled_until` writes the *same* field, no `derive_wave` change).
- **The 8-step `derive_wave` ladder** (first match wins):
  `NeedsInput > Failed > Working > AwaitingReview > Scheduled > Ready > Slept > Neutral`.
  (Working moved above Ready — behavior-preserving, they're disjoint on status.)
- **Two `StatusTokens` colors** `scheduled` + `awaiting_review` (`theme/tokens.rs`) +
  both `lens-dark.json`/`lens-light.json` (**placeholders**: dark `#8b9bf5`/`#c084fc`,
  light `#6366f1`/`#9333ea`) + `Wave::status_color` arms.
- **Two `--demo` cards** (`lens-app/src/main.rs::demo_cards`): `demo-awaiting-review`,
  `demo-scheduled` (static `now + 2m` wake).
- **`chrome.rs`**: two `wave_label` arms (compile necessity — this is the throwaway
  pill **B1 deletes**; no behavior work).
- **9 unit tests**, incl. the load-bearing `settles_to_awaiting_review_after_turn_ends`.

---

## Key decisions (the *why*, so you don't re-litigate)

1. **Both states are Lens-owned signals** — they ride Lens's own MCP server (which will
   also host board-control, messaging, KB tools). **omnigent has NO scheduling concept**
   — verified against `vendor/omnigent-0.5.1/openapi.json` (status enum is strictly
   `launching/idle/running/waiting/failed`; `waiting` = async-work drain, not "parked
   until T"). So neither state touches the omnigent contract.
2. **Scheduling = "A now, B as a seam."** A: Lens owns the schedule (a Lens MCP
   `schedule_wake` tool and/or a board action; Lens fires the wake by sending a message
   at T). B (future): forward a native harness schedule through omnigent → same field.
   C (native `/loop`, unforwarded) is **off the table** — no wire signal, can't light a
   wave. Chosen A because it's the only *uniform-across-harnesses* option and needs no
   omnigent change.
3. **`await_review` is NON-blocking** (a blocking MCP call would time out). It posts the
   review to Lens and **returns control to the agent, who ends its turn**. The human's
   Canvas comments return via **MessageCenter** ("You've got Mail" prompt — SessionStart
   hook or a second MCP tool), which **clears** `awaiting_review`. → it's *soft* async
   attention, so it sits **below** NeedsInput/Failed/Working but **above** Ready.
4. **The settle property is load-bearing (`AwaitingReview` > `Ready`).** When the agent
   ends its `Working` turn, the turn advance *also stamps Ready*. Because AwaitingReview
   (4) outranks Ready (6), the idle session **settles into AwaitingReview, not a Ready
   flash**. Tested.
5. **`Scheduled` > `Ready`, gated to Active+Idle+future wake, self-clears.** A
   self-scheduled session says "I'll resume myself," which must not masquerade as
   "ready for you." It renders only while the wake is in the future; once now passes it,
   the branch stops matching and it falls through — like Ready's decay.

---

## ⚠ Deferred — DO in the B1–B5 wave build (not separate)

The wave *behaviors* were **intentionally cut** from the composer slice so all 8 waves
get their treatment as **one system** in B1–B5 (building AwaitingReview's pulse in
isolation now would just mean reconciling with B's behavior work later):

1. **Behaviors** (code keyed off `Wave` — D2, `Colorize::opacity/mix`, not tokens):
   - `AwaitingReview` — attention-grade like NeedsInput but a **distinct hue** + a
     *gentle* pulse (soft, not hard-block urgency). Card affordance: deep-link to the
     Canvas artifact under review.
   - `Scheduled` — calm/**dimmed** tint (kin to slept-dim) + a **countdown affordance**
     ("wakes in 3m") in the activity slot. Static or very-slow breathe.
   - Plus the existing 6 waves' glow/pulse/radial (B-ledger).
2. **Poller repaint timer at `scheduled_wake_at`** — mirrors `READY_DECAY_MS`'s gpui
   executor wake (dual-clock in the poller) so the card stops showing `Scheduled` at T.
   *Firing* the wake is the deferred scheduler (#11); the wave only needs the repaint.
3. **Color tuning** — the 4 wave colors (ready/working/scheduled/awaiting_review) must
   be mutually distinguishable; folds into the **one end-of-build tuning pass** via the
   reload loop (`⌘⇧T`, `LENS_THEME_DIR=crates/lens-ui/src/theme`).

## SPEC-GAPS #11 — Lens-owned MCP producer layer (its own spec→plan→build)

The producers that *feed* these fields: Lens MCP server (`await_review`,
`schedule_wake`, board control, messaging, KB) + the wake-firing scheduler.
**⚠ OPEN RISK at the top of #11 — resolve FIRST:** a **remote** agent (managed host /
omnigent server) must reach an MCP server on the **user's local Mac**. If that
transport doesn't work, the whole producer layer needs a different shape. The
wave-side contract here does **not** depend on it (`--demo` sets the fields directly).

---

## Start-here for next session

- Sequencing (unchanged): ~~theming schema~~ → **waves (B1–B5, resume here)** → board
  packing (B6–B8) → light checkpoint → transcript → composer → panes/terminal/editor →
  shell polish → theming machinery.
- **B1 = icon-tile** replaces the throwaway pill (deletes `pill_text_color`/`wave_label`
  from `card/chrome.rs` — note that removes the two label arms this session added) and
  must render **all 8 waves** uniformly (44px status tile + card overlay per
  `board-home.html`). Then B2–B5 add the behaviors above.
- The wave build is `--demo`-driven (now 8 preset cards) and tunes colors via the
  reload loop.
- Start with `superpowers:brainstorming` if the B1–B5 behavior scope isn't crisp, else
  `superpowers:writing-plans` from the B1–B5 ledger
  (`docs/handoffs/2026-07-16-lens-ui-theming-and-card-audit.md`).
