# Handoff — lens-ui card visuals (the "wave") + dev affordances

**Date:** 2026-07-15
**Branch:** `feat/lens-app-multi-session` (off `main@726cbd2`, the merged lens-ui skeleton)
**Status:** WIP committed. The visual work (the animated "wave") is a **new-session job** — this
doc is its brief. The dev affordances (`--demo`, `--sessions`) are done and are the iteration vehicle.

---

## Why this branch exists

After the lens-ui skeleton merged, we launched `lens-app` to look at it. Two gaps surfaced:
1. The windowed app only wired a single `--session` → added multi-session windowed mode.
2. The card chrome renders the "wave" as a **flat border + a status pill** — a placeholder. The real
   design is an **animated glowing pulse** whose color + pulse-rate encode urgency. Not built.

The visual work is genuine UI/rendering polish, out of the skeleton's mechanism-proof scope, so it's
deferred to a fresh session. This branch carries the tooling to do it.

## What's on the branch (committed)

- **`crates/lens-app/src/main.rs`:**
  - `--session` is now **repeatable** + a `--sessions <csv>` flag → the windowed board shows N cards
    (`FleetStore::spawn_live_session` per id; the pub `cards` map). KEEPER.
  - **`--demo` mode** (`run_demo` + `demo_cards`): paints six cards, one per wave state, directly into
    the `cards` map — **no live agent needed**. THE iteration vehicle for the wave work. KEEPER.
- **`crates/lens-ui/src/card/chrome.rs`:** a placeholder visual bump (filled state pill + label +
  2px border). **Throwaway** — the new session replaces this with the real wave.

## The spec (SSOT) — DO NOT re-derive

- **`docs/design/application-shell-and-layout.md` §5.1 "Anatomy & the wave"** — the anatomy ASCII, the
  icon-tile/status/activity/rows/footer/progress-bar layout, and **the wave table** (line ~257):

  | Wave | Treatment | Urgency | Derived from |
  |---|---|---|---|
  | Needs-input | **fast pulse**, red | highest | `pending_elicitations` non-empty; sticky; overrides Slept dimming |
  | Ready | **steady**, blue | high | `idle` + unacknowledged turn completion |
  | Working | **calm shimmer**, green | medium | `running/launching/waiting`, no pending elicitation |
  | Failed | steady, red + Retry | ≈Ready | `failed` / `last_task_error` |
  | Slept | **dimmed card** + Resume, no pulse | lowest | Lens lifecycle Slept |

- **Reference render: `docs/design/renders/board-home.html`** — the exact target look. Card recipe
  (CSS): `border:1.5px solid var(--c)` + **colored glow** `box-shadow:0 0 22px -12px var(--c)` +
  **radial-gradient tint** bg + a **44px colored icon-tile** (glyph per state) + colored uppercase
  `.stat` + **state-colored progress bar** `.pbar`. Palette (`:root`): green `#36c98a`, blue `#4c8dff`,
  orange `#ff8a3d`, red `#ff5d5d`, gray `#7a8493`. (Render uses orange for Needs-input; the §5.1 table
  says red — reconcile, render likely wins.)
- The render also shows **group "lanes"** (`.gwrap`: colored border + glow grouping cards) — a bigger
  board-layout change; do the cards first, lanes as a follow-up.

## ⚠ Bugs/deltas to fix in the new session

1. **Wave COLORS are swapped in the merged skeleton.** `chrome.rs::wave_border_color` has
   Ready=green `0x22c55e`, Working=blue `0x3b82f6` — **backwards** vs §5.1 (Ready=blue, Working=green).
   The wave *logic* (`derive_wave`) is correct; only the color values are wrong. Fix against §5.1.
2. **No wave animation / glow / gradient / icon-tile** — the whole visual is a placeholder.

## gpui 0.2.2 feasibility (verified against the registry source — don't re-derive)

- **Gradient tint: YES** — `gpui::linear_gradient` (`color.rs:765`) + `.bg(impl Into<Background>)`
  (`styled.rs:372`). Only *linear* found; approximate the render's radial tint with a linear gradient.
- **Animation (the pulse): YES** — `gpui::Animation` + `AnimationElement` (`elements/animation.rs`),
  and `Window::request_animation_frame` / `on_next_frame` (`window.rs:1644,1654`) for a frame loop.
  Drive the glow/opacity/tint on a sine and set the **period per wave** (fast for Needs-input, calm
  shimmer for Working, steady/none for Ready/Failed/Slept). Watch §4.4: animating a card must NOT
  notify `FleetStore` (per-card only), and the isolation acceptance test must stay green.
- **Box-shadow GLOW: NO builder.** No `.shadow()` on `Styled`/`Div` in 0.2.2, and `BoxShadow`
  (`style.rs:308`) has no setter method. Options: (a) set the raw `StyleRefinement.box_shadow` field
  via the `Styled::style()` accessor (`div().style().box_shadow = Some(smallvec![BoxShadow{..}])`),
  or (b) approximate the glow with a pulsing outer ring / gradient. Verify (a) compiles first.

## Other deferred items surfaced this session (fold into the stream-lifecycle follow-up)

- **App exit takes 20-30s with N live streams.** Root cause: `StreamBridge::drop` (`fleet/live.rs`)
  does `stop()` + **`forwarder.join()`**; `EventStream::stop()` is cooperative (`reader.rs:60`) and the
  SSE `body.read()` has **no read timeout** (REST_TIMEOUT is REST-only; the read-idle backstop is
  deferred), so each join blocks until the next server heartbeat. ×N streams → 20-30s. **Do NOT** fix
  by detaching the join (throwaway — the proper fix makes join fast). Proper fix = a bounded read/poll
  on the SSE body so `stop` takes effect promptly, done carefully so an idle-timeout isn't mistaken for
  a disconnect (would trip the reconnect state machine). Lives with the whole-branch review's flagged
  **off-thread live-subscribe** follow-up (`spawn_live_session` blocks the fg thread at window-init).
- **This box has no working model provider** — goose-native turns fail instantly (→ Failed/red);
  claude-sdk (`debby`) sends 503 (no warm runner). So live Working→Ready/Needs-input can't be driven
  here without provider/runner config; `--demo` is the workaround for seeing states.

## How to run

```
cargo run -p lens-app -- --demo                                   # six states, no server
cargo run -p lens-app -- --sessions <id1,id2,…> --base-url <url>  # N live cards
```
Message send shape (for driving live turns): `POST /v1/sessions/{id}/events`
`{"type":"message","data":{"role":"user","content":[{"type":"input_text","text":"…"}]}}`.
Omnigent 0.5.1 is installed (`08285468`); `omnigent server start` + `omnigent host "" --non-interactive`.
