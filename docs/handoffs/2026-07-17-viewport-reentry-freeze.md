# Handoff — Viewport re-entry animation freeze (focus↔board)

> **✅ RESOLVED 2026-07-17** (same day, dedicated session). Fixed on `main` in `lens-ui`
> (`board/mod.rs`, `card/view.rs`) with 3 regression tests in `tests/acceptance_shell.rs`; gate green;
> codex cross-family review addressed (Findings 1+2 fixed, 3 deferred). **Fix ≠ candidate #1 verbatim:**
> resetting `last_bounds` from `BoardView::render` SUPPRESSED the cards' re-renders (reading sibling
> entities inside `detect_accessed_entities` perturbs `.cached()` dirty-tracking); the working fix does
> it in the **fleet-observe EFFECT** via `view.update(|v,cx| { v.invalidate_viewport_gate(); cx.notify() })`.
> **Also: the handoff was wrong that `#[gpui::test]` can't assert this** — canvas paint bounds ARE real
> under `add_window_view` (only glyph shaping is faked), so the regression tests are ordinary
> `#[gpui::test]`s, no `harness = false` binary needed. Full write-up: memory `viewport-reentry-freeze`.
> Remaining optional step: on-device visual confirmation of the demo repro (automated real-window test
> already drives board→focus→board and asserts render resumes). The rest of this doc is the original
> (pre-fix) analysis, kept for provenance.

**Written:** 2026-07-17 · **Branch:** `main` · **HEAD:** `ad7a6d5` (unpushed) · **For:** a dedicated next session
**Use `superpowers:diagnosing-bugs` (perf/UI regression loop).** This is a real bug with a known mechanism.

## Symptom (user-reported, on-device)

Enter **focused mode** on a session, then switch **back to board view**: the Working card's **spinner**
and the activity-line **pulse dot** are **frozen** — they no longer spin/pulse. They stay frozen until
some other notify fires (e.g. a live SSE fold, a theme reload, a resize).

## Root cause (high-confidence — this is handoff Follow-up 3, mis-scoped)

The per-card animation driver is viewport-gated in `crates/lens-ui/src/card/view.rs`:

- **`view.rs:84`** — `let visible = match self.last_bounds.get() { Some(b) => b.intersects(&viewport), None => true (first-frame) }`.
- **`view.rs:91-95`** — `desired = anim_tick_for(wave).filter(|_| visible)`; if `desired != anim_interval`, drop + respawn the timer `Task`. `visible=false → desired=None → anim_task dropped → no re-render`.
- **`view.rs:160`** — the canvas **paint closure** does `last_bounds.set(Some(bounds))` — i.e. `last_bounds` is
  updated **at paint time, AFTER render, and WITHOUT `cx.notify()`**.

The freeze: while the card is off-screen (focused mode), it isn't painted, so `last_bounds` holds a stale
value. On return to board, the card re-renders **once** (board re-render) and reads the **stale** bounds at
`view.rs:84` → `visible=false` → `desired=None` → the timer is never respawned. The paint closure *does* then
write the correct on-screen bounds (`:160`), but that carries **no notify**, so nothing re-runs the gate. The
card sits with correct bounds, no timer, and no scheduled re-render → **frozen**.

Both the spinner and the new activity pulse freeze together because they share the one driver (the pulse reads
`now_ms` per render via `motion.rs pulse_alpha` — correct; it just needs the card to keep re-rendering).

## The correction to the prior handoff

`docs/handoffs/2026-07-17-wave-build-b1-b5-followups.md` **Follow-up 3** claimed this was **unreachable** in the
non-scrolling build ("rides with B6's scroll container"). **That was wrong.** The **focused-mode ↔ board**
toggle takes cards off-screen and back — the exact off→on transition — so it's live today, independent of B6.

## Repro (manual / HITL — no auto harness yet)

```bash
LENS_THEME=dark cargo run -p lens-app --release --features demo -- --demo
# 1. note the Working card ("Refactor the session poller") spinner + green pulse dot animating
# 2. click a card to enter focused mode (Working card now off-screen)
# 3. return to board view
# 4. BUG: Working spinner + pulse frozen
```
Under gpui `TestAppContext` the text/paint system is a `NoopTextSystem` (memory `gpui-test-noop-text-system`),
so a real freeze can't be asserted in a `#[gpui::test]` — this needs the real-app harness or on-device. First
job in the next session per `diagnosing-bugs`: build the tightest feedback loop you can (candidate: the
`xtask`-run real-`Application` harness that Task 9 used, driving a board→focus→board transition and sampling
each card's `render_count` delta).

## Fix constraint + candidate directions (do the diagnosis loop first)

**HARD CONSTRAINT:** do **NOT** fix by calling `notify()` from the paint closure — it breaks gpui's
render/paint separation (this was the explicit warning from the original codex review).

Candidate approaches to evaluate (not yet chosen):
1. **Invalidate `last_bounds` on re-show.** When the board transitions back from focused mode, reset each card
   view's `last_bounds` to `None` → the `view.rs:84` first-frame path treats it as visible → respawns the timer
   → paints → writes correct bounds. Requires the board/focus switch (`crates/lens-ui/src/board/mod.rs`) to know
   when it's re-shown and reach the card views. Likely the cleanest.
2. **Render-time visibility** instead of paint-time bounds — but gpui doesn't expose element bounds during
   render (that's *why* the code reads last-painted bounds), so this needs a different visibility signal.
3. **Paint-safe transition nudge** — detect an off→on transition in the paint closure and schedule a re-render
   via a render/paint-safe deferral (e.g. an on-next-frame hook), never `notify()`.

## Verification when fixed

- Re-run the manual repro: board → focus → board, confirm spinner + pulse resume.
- Add a **Scheduled↔offscreen↔onscreen transition test** at the real-app harness layer (the §4.4 isolation test
  `animating_card_does_not_render_a_static_sibling` is the sibling pattern to extend).
- `cargo run -p xtask -- gate` green.

## Where the visual work stands (all landed, don't reopen)

Wave-build follow-ups are DONE + committed on `main` (unpushed): perf 30→20fps (`4e27830`), card wash
(`a172887`), header hierarchy + host pill + per-wave activity line (`ad7a6d5`). Spec `§11` + STATUS current.
Activity-line render ref: `docs/design/renders/wave-card-activity-line.html`. **Push is the user's call** (not
done). Memories: `wave-perf-fps-attribution`, `wave-card-body-wash`, `wave-card-activity-line`.
