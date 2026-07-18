# Design — two new wave states: `Scheduled` + `AwaitingReview`

**Date:** 2026-07-16
**Status:** design approved, pending user review of this doc
**Scope:** **wave-side contract only.** This locks the presentation layer (the two
new `Wave` variants, the card fields they key off, ladder placement, colors,
behaviors) so the imminent wave build (B1–B5) reserves room and can `--demo` them.
The *producers* that feed these fields — the Lens-owned MCP server (`await_review`,
`schedule_wake`), the wake-firing scheduler, MessageCenter return path — are a
**separate, not-yet-built workstream**, filed as `docs/SPEC-GAPS.md` #11.

Supersedes nothing; extends `card/wave.rs` (the 6-wave ladder shipped with the
§18 theming substrate, handoff `2026-07-16-theming-substrate-shipped.md`).

**Build boundary (2026-07-16):** the *structural contract* — §2.1 fields, §2.2
ladder, §2.3 **tokens only** (two `StatusTokens` fields + both JSONs +
`status_color` arms, placeholder colors), §2.4 demo cards, §3 tests — is a single
composer slice that lands **first**, so B1's icon-tile renders all 8 waves
uniformly. **Deferred out of that slice:** (a) the §2.3 **wave *behaviors*** (pulse
/ dim / countdown affordance / Canvas deep-link) fold into the **B1–B5** wave build
where all 8 waves get their glow/pulse/radial treatment as one system; (b) the §2.2
**poller repaint timer** at `scheduled_wake_at` ships with the scheduler (#11) or a
tiny follow-on — the demo uses a static future timestamp and needs no timer to
render `Scheduled`.

---

## 1. Motivation & feasibility verdict

We want two more card states beyond the current six (`NeedsInput`, `Ready`,
`Working`, `Failed`, `Slept`, `Neutral`):

1. **`Scheduled`** — the session is parked and will self-resume at a known time
   (agent scheduled a wake, or the user set a loop/wake).
2. **`AwaitingReview`** — the agent wrote something to the Canvas and asked for a
   human review (soft, async — *not* a hard block).

**Feasibility (verified against `vendor/omnigent-0.5.1/openapi.json`):** omnigent's
status enum is strictly `launching / idle / running / waiting / failed`, with **no
schedule/wake/defer/snooze/cron concept** anywhere; `waiting` means "parent turn
parked on the async-work drain" (sub-agents/background tools), *not* "parked until
time T." So a wave for either state is supportable **only if Lens owns the
originating signal** — which it does: both ride Lens's own MCP server. Neither
touches the omnigent contract.

### Provenance decisions (locked)

- **`AwaitingReview` source** — Lens-owned MCP server. The agent calls a Lens
  `await_review` tool; Lens sees the call directly. (Same server will host board
  control, messaging, knowledge-base tools.)
- **`Scheduled` source** — **A now, B as a seam.** *A:* Lens owns scheduling (a
  Lens MCP `schedule_wake` tool and/or a board action); Lens holds the wake-time
  and fires the wake by sending a message at T. *B (future seam):* if a harness's
  native `/loop`/`ScheduleWakeup` is ever forwarded through omnigent as a
  `scheduled_until` signal, it populates the **same** card field with no
  `derive_wave` change. *C (native, unforwarded) is off the table* — it produces
  no wire signal and cannot light a wave.
- **Why Lens-owned over harness-native:** it's the only option that gives a
  **uniform board across harnesses** (native scheduling is per-harness and mostly
  invisible), needs **no omnigent contract change** (the recurring blocker in this
  project), and is **symmetric with `AwaitingReview`** — both become one coherent
  "Lens MCP capability layer."

---

## 2. Wave-side contract (what this design changes)

`Wave` grows 6 → 8. Everything below lives in `crates/lens-ui` (+ two theme JSONs);
**no `lens-core` change** in this slice (the signals are Lens-owned, not omnigent
folds; the forwarded-`Scheduled` seam would add a `lens-core` field later).

### 2.1 New `Wave` variants + card fields

```
enum Wave { NeedsInput, Ready, Working, Failed, Slept, Neutral,
            AwaitingReview, Scheduled }   // + 2
```

| Wave | New `SessionCard` field | Type | Default | Set / cleared by (deferred producer) |
|---|---|---|---|---|
| `AwaitingReview` | `awaiting_review` | `bool` | `false` | **Set** when Lens MCP `await_review` fires; **cleared** when MessageCenter delivers the human's Canvas comments back to the session. |
| `Scheduled` | `scheduled_wake_at` | `Option<i64>` (epoch ms) | `None` | Lens scheduler on `schedule_wake`; cleared/updated when the wake fires or is cancelled. Source-agnostic (see seam). |

- Both default in `SessionCard::new`. Real population is the deferred MCP
  workstream; `--demo` cards set them directly to exercise the waves now.
- **`scheduled_wake_at` is deliberately source-agnostic — that *is* the B-seam.**
  The Lens scheduler writes it today; a future forwarded omnigent `scheduled_until`
  fold writes the same field. No `ScheduleSource` enum yet (YAGNI); add one only if
  display ever needs to distinguish Lens-set vs harness-forwarded.

### 2.2 `derive_wave` ladder (priority order — first match wins)

```
1. NeedsInput      needs_attention && status != Failed && last_task_error.is_none()
2. Failed          status == Failed || last_task_error.is_some()
3. Working         status in {Running, Launching, Waiting}
4. AwaitingReview  awaiting_review
5. Scheduled       lifecycle == Active && status == Idle && scheduled_wake_at is in the FUTURE
6. Ready           status == Idle && recent completion within READY_DECAY_MS && !is_focused
7. Slept           lifecycle == Slept
8. Neutral         (else)
```

Rationale for each placement:

- **`AwaitingReview` below `NeedsInput`, below `Failed`, below `Working`.** It is
  *soft* async attention, not a hard block: `await_review` is fire-and-return, so
  the agent ends its turn — the session usually goes **idle** while the flag stays
  set. (1) A hard `NeedsInput` (e.g. the agent also fires an `AskUserQuestion`) must
  win — hard block over soft nudge. (2) `Failed` outranks it — a broken session's
  pending review is moot. (3) `Working` outranks it — the only overlap is the edge
  where the agent got re-engaged and is actively running while a review is still
  pending; the live "working" signal is more honest than a calm "awaiting review"
  on a session that's actually grinding. The pending review still surfaces as a
  **card badge / Canvas deep-link** (B1 affordance) so it isn't lost.
- **The settle property (load-bearing — why `AwaitingReview` > `Ready`).** When
  the agent ends its `Working` turn, `Working` stops matching *and* the turn advance
  stamps `Ready` (`last_completed_at`). So the idle session is simultaneously
  `Ready`-eligible **and** `awaiting_review`. Because **`AwaitingReview` (4) >
  `Ready` (6)**, it **settles into `AwaitingReview`** — not a misleading "Ready"
  flash — and holds there until MessageCenter clears the flag. This is the intended
  behavior and gets an explicit test.
- **`Scheduled` > `Ready`** (confirmed). A self-scheduled idle session says "I'll
  resume myself, back at T" — showing `Ready` there would falsely imply it's parked
  on *you*. Plain scheduling = calm self-resume; if the agent actually wants your
  eyes, that's `await_review`/`AwaitingReview`.
- **`Scheduled` gated to `lifecycle == Active && status == Idle`.** Disjoint from
  `Working` (active now → `Working` wins; the latent schedule is background) and
  from `Slept` (a slept session shouldn't self-wake). Only overlaps `Ready`, which
  it outranks.
- **`Scheduled` self-clears.** It renders *only while `scheduled_wake_at` is in the
  future*. Once the clock passes it, the field goes stale, the branch stops
  matching, and the wave falls through to `Ready`/`Neutral` — exactly like `Ready`'s
  decay. **Consequence:** the poller needs a **repaint timer at `scheduled_wake_at`**
  (mirrors `READY_DECAY_MS`'s gpui executor wake in the poller's dual-clock). *Firing*
  the wake (send-message-at-T) is the deferred scheduler; the wave only needs the
  repaint so the card stops showing `Scheduled` at T.

### 2.3 Colors + behaviors

- **Tokens:** `StatusTokens` gains two hex fields — `scheduled`, `awaiting_review`
  (`crates/lens-ui/src/theme/tokens.rs`) — plus entries in both
  `lens-dark.json` / `lens-light.json`. Colors are **placeholders**, tuned in the
  one end-of-build pass (per the deferred systematic-tuning decision). `Wave::status_color`
  gets two arms; the exhaustiveness test (`status_color_total_over_all_waves`) grows
  by two assertions.
- **Behaviors** are code keyed off `Wave` (D2 — `Colorize::opacity/mix`, not tokens):
  - **`AwaitingReview`** — attention-grade like `NeedsInput` but a **distinct hue**
    + a gentle pulse (soft, not the hard-block urgency of `NeedsInput`). Card
    affordance: deep-link to the Canvas artifact under review (B1 polish, noted not
    built).
  - **`Scheduled`** — calm/dimmed tint (kin to the slept-dim) in its own hue, with
    a **countdown affordance** ("wakes in 3m") in the activity slot. Static or
    very-slow breathe — deliberately *not* attention-grabbing.

### 2.4 `--demo` coverage

`lens-app/src/main.rs::demo_cards` gains preset cards for both new waves (one
`AwaitingReview`, one `Scheduled` with a future `scheduled_wake_at`), so the wave
build tunes their colors via the reload loop (⌘⇧T with
`LENS_THEME_DIR=crates/lens-ui/src/theme`) alongside the existing six.

---

## 3. Tests (wave-side)

Extend `card/wave.rs` unit tests:

1. `awaiting_review_below_needs_input` — `awaiting_review && needs_attention` ⇒
   `NeedsInput` (hard block wins).
2. `awaiting_review_below_failed` — `awaiting_review && status==Failed` ⇒ `Failed`.
3. `working_beats_awaiting_review` — `awaiting_review && status==Running` ⇒
   `Working`.
4. **`settles_to_awaiting_review_after_turn_ends`** — set `awaiting_review`, run a
   turn to idle so `Ready` is *also* eligible (recent `last_completed_at`,
   `!is_focused`) ⇒ `AwaitingReview`, **not** `Ready`. (The load-bearing settle.)
5. `scheduled_requires_future_wake` — `scheduled_wake_at` in the past ⇒ falls
   through (not `Scheduled`); in the future (idle, Active) ⇒ `Scheduled`.
6. `scheduled_beats_ready` — future wake + recent completion + `!is_focused` ⇒
   `Scheduled`.
7. `scheduled_below_working` — future wake + `status==Running` ⇒ `Working`.
8. `scheduled_requires_active_not_slept` — future wake + `lifecycle==Slept` ⇒
   `Slept`.
9. `status_color_total_over_all_waves` — extended with `scheduled` +
   `awaiting_review` arms (compile-time exhaustiveness is the real guard).

---

## 4. Out of scope / deferred (tracked)

Filed as **`docs/SPEC-GAPS.md` #11 — Lens-owned MCP producer layer**, its own
spec → plan → implementation cycle:

- The Lens MCP server surface: `await_review`, `schedule_wake`, board control,
  messaging, knowledge-base tools; and the wake-firing **scheduler** (send-message
  at T) + the `Ready`-style repaint-timer wiring in the poller.
- **`await_review` mechanics** (captured so they're not re-derived): it is
  **non-blocking** (a blocking MCP call would time out) — the tool sends the review
  request to Lens and **returns control to the agent, who ends its turn**. The human
  later reviews the Canvas and submits comments; those flow back as a prompt message
  via **MessageCenter** (a SessionStart hook *or* a second MCP tool — either way Lens
  posts a "You've got Mail" message), which is what **clears** `awaiting_review`.
- **OPEN RISK (load-bearing for both producers):** a **remote** agent (managed
  host / omnigent server) must reach an MCP server that lives on the **user's local
  Mac**. If that transport doesn't pan out, the producers need a different shape.
  The wave-side contract here does **not** depend on it (the `--demo` path sets the
  fields directly), so this slice proceeds regardless — but the risk gates the
  producer workstream and is recorded at the top of the SPEC-GAPS entry.
- The **B-seam** for forwarded harness scheduling: if pursued, an omnigent
  contract request for a `scheduled_until` signal (sibling to the `client-message-id`
  ask), folded into the same `scheduled_wake_at` field via `lens-core`.
