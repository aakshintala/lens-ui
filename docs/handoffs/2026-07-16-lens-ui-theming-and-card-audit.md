# Handoff — lens-ui card/board audit + theming-first (next: §18 brainstorm)

**Date:** 2026-07-16
**Branch:** `feat/lens-app-multi-session` (off `main@726cbd2`; this session's commit `b367dbf`)
**Status:** Audit DONE + small fixes landed. **Next job = brainstorm §18 theming** (a design pass,
not an edit). Then the wave build. This doc is the brief for both.

**Retires** `docs/handoffs/2026-07-15-lens-ui-card-visuals.md` — its still-live content (gpui-0.2.2
wave feasibility + two open deferred items) is folded into the "Carried forward" section at the bottom
of this doc.

---

## What this session did

Audited `lens-app`/`lens-ui` against `docs/design/application-shell-and-layout.md` §4–§5 (deep) + a
thin whole-doc pass for spec clauses that are *wrong* (not merely unbuilt). Landed the cheap fixes;
teed up theming as the gating next effort.

**Committed (`b367dbf`, branch only — NOT merged; the wave build is still ahead):**
- **A1** — `chrome.rs::wave_border_color` had Ready=green / Working=blue, **backwards** vs §5.1.
  Fixed: Ready→blue `0x3b82f6`, Working→green `0x22c55e`. (Values still hardcoded — tokenization lands
  with theming, see below.)
- **C1** — §5.1 Needs-input **red→orange**, reconciled to `renders/board-home.html` as the pixel SSOT
  (orange keeps Needs-input distinct from Failed-red). Annotated in the spec.
- **C2** — §5.1 stale omnigent ref `0.3.0.dev0` → `0.5.1`.
- Gate green: `lens-ui` builds clean, `clippy` zero-warnings, 17 lib + 1 acceptance tests pass.

**Decisions locked this session (user):**
1. Needs-input wave = **orange** (not red).
2. The card status readout is the **44px icon-tile** (§5.1), **not** the filled text *pill* currently
   in `chrome.rs`. The pill is throwaway — the wave build replaces it with the tile.
3. **Invest in theming (§18) NOW**, before building more UI surface — avoids retrofitting every
   `gpui::rgb(...)` call site.

---

## The deviation ledger (the durable audit artifact)

Scope = §4 board + §5 card (the only live surface; the rest of the shell is known-skeleton stubs, not
deviations). Buckets: **A** = code bug vs locked decision · **B** = unbuilt-per-plan · **C** = spec was
the stale one.

### A — code bugs
- **A1 DONE** (colors swapped) — see above.
- **A2 OPEN** — hardcoded hex ≠ render palette (code Ready `0x3b82f6` vs render blue `#4c8dff`; every
  value differs) **and** violates §18 "semantic tokens, not raw hex at call sites". **Resolved by the
  theming effort** (do NOT hand-patch hex now — it all gets re-homed to `status.*` tokens).

### B — unbuilt anatomy (the wave build + board)
Card (§5.1 + render):
- **B1** — icon-tile (44px, status-colored, per-state glyph ↻☾✓🔔!). Replaces the pill (decision 2).
- **B2** — context-window progress bar (`.pbar`, status-colored fill). Code has ctx-% text only.
- **B3** — glow (`box-shadow:0 0 22px -12px`) + radial tint + **pulse animation** (the actual "wave";
  fast=Needs-input, calm shimmer=Working, steady/none=Ready/Failed/Slept). Feasibility in the
  2026-07-15 handoff.
- **B4** — layout order: render is tile-left + stacked STATUS/title/model; code stacks header-row then
  a model-row.
- **B5** — Slept dim + Resume button, Failed Retry button (code fakes Retry as activity text; Slept is
  just a gray border).

Board (§4):
- **B6** — adaptive count-aware packing (§4.3, decision ③). `render_board_grid` is plain `flex_wrap`.
- **B7** — ordinal slots (§4.1, decision ①). Cards sort by `session_id` string; no persisted order.
- **B8** — group "lanes" (`.gwrap`). Deferred (also deferred in the prior handoff).

### C — spec was stale (all resolved this session)
- **C1 DONE** (red→orange), **C2 DONE** (version bump).
- **C3 dropped** — §7.5 "(0.2.0 codex-native Plan)" is an *introduced-in* note, legitimately correct,
  not stale. Left as-is.
- **C4 no-edit** — §5.1 already mandates the icon-tile; the pill was a code shortcut. Resolution is a
  code-direction call (tile wins, decision 2), not a doc change.

---

## NEXT: §18 theming brainstorm (do this FIRST — it gates the wave build)

Not a straight edit — it's a load-bearing design surface everything docks into. Run
`superpowers:brainstorming` → design before any code.

**Grounding facts (verified this session):**
- `gpui-component = "0.5.1"` is **already a dependency** in `lens-ui` and `lens-app` (and all 3 spikes).
  Its theme provider (`ActiveTheme`/`Theme`, semantic tokens) is available with no vendoring.
- **Zero theming in-tree today** — no `Theme` struct anywhere; raw hex at every call site. Greenfield.

**THE load-bearing fork to resolve:**
- §18 as written says "a gpui `Theme` struct (semantic names, not raw hex)" — implies *our own* struct.
- gpui-component has its *own* theme system, and the components we plan to adopt (markdown renderer,
  inputs, forms — proven in the spikes) read colors from *its* theme context.
- So: **(a) adopt gpui-component's theme as the substrate + layer our `status.*` tokens on top**, vs
  **(b) build the §18 struct + bridge gpui-component to it.** This choice decides the token schema, the
  importer story (base16 → VS Code, §18), and how the wave's `status.*` colors are consumed.

**Must come out of the brainstorm:** the fork decision, the token schema (incl. `status.*` for the
wave — seed values from the `board-home.html` `:root` palette: green `#36c98a`, blue `#4c8dff`, orange
`#ff8a3d`, red `#ff5d5d`, gray `#7a8493`, purple `#b08cff`), and a design doc. Then A2 is a mechanical
call-site migration and the wave build (B1–B5) consumes real tokens from day one.

**After theming:** wave build (B1–B5) as its own planned unit (`--demo` is the iteration vehicle);
board packing (B6–B8) deferred, cards-first.

## How to run
```
cargo run -p lens-app -- --demo      # six wave states, no server (the iteration vehicle)
```

---

## Carried forward from the retired 2026-07-15 card-visuals handoff

**gpui 0.2.2 wave feasibility (verified against registry source — don't re-derive for B3):**
- **Gradient tint: YES** — `gpui::linear_gradient` (`color.rs:765`) + `.bg(impl Into<Background>)`
  (`styled.rs:372`). Only *linear* exists; approximate the render's radial tint with a linear gradient.
- **Pulse animation: YES** — `gpui::Animation` + `AnimationElement` (`elements/animation.rs`) +
  `Window::request_animation_frame`/`on_next_frame` (`window.rs:1644,1654`). Drive glow/opacity/tint on
  a sine; period per wave (fast=Needs-input, calm shimmer=Working, steady/none=Ready/Failed/Slept).
  **§4.4 constraint:** a card animation must NOT notify `FleetStore` (per-card only) — the
  render-isolation acceptance test must stay green.
- **Box-shadow glow: NO builder** in 0.2.2 (no `.shadow()` on `Styled`/`Div`; `BoxShadow` has no
  setter). Options: (a) set raw `StyleRefinement.box_shadow` via `Styled::style()`
  (`div().style().box_shadow = Some(smallvec![BoxShadow{..}])`) — verify it compiles first; or (b)
  approximate with a pulsing outer ring / gradient.

**Two open deferred items (fold into the stream-lifecycle follow-up, NOT the theming/wave work):**
- **App exit hangs 20–30s with N live streams.** `StreamBridge::drop` (`fleet/live.rs`) does `stop()` +
  `forwarder.join()`; `EventStream::stop()` is cooperative (`reader.rs:60`) and the SSE `body.read()`
  has no read timeout, so each join blocks until the next server heartbeat → ×N streams. **Do NOT**
  "fix" by detaching the join. Proper fix = a bounded read/poll on the SSE body so `stop` takes effect
  promptly, done so an idle-timeout isn't mistaken for a disconnect (would trip reconnect). Lives with
  the whole-branch review's **off-thread live-subscribe** follow-up (`spawn_live_session` blocks the fg
  thread at window-init).
- **This box has no working model provider** — goose-native turns fail instantly (→Failed/red);
  claude-sdk (`debby`) sends 503 (no warm runner). So live Working→Ready/Needs-input can't be driven
  here; **`--demo` is the workaround** for exercising wave states.
