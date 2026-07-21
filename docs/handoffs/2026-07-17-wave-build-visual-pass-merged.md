# Handoff — Wave Build B1–B5 visual pass DONE, merged + pushed

**Written:** 2026-07-17 · **Branch:** merged to `main` · **HEAD:** `7396afc` (== `origin/main`, pushed)
**Prior handoff:** `docs/handoffs/2026-07-17-wave-build-b1-b5-followups.md` (the pre-visual-pass state)
**Spec SSOT:** `docs/specs/2026-07-17-wave-behaviors-design.md` (see **§11** — as-built amendments)

## TL;DR

The B1–B5 wave build's **on-device visual acceptance pass is complete**. All visuals were
confirmed on-device (dark mode), fixes folded in, cross-family reviewed (codex, clean), gate
green, and the **whole branch is merged to `main` and pushed** (`7396afc`). The wave build is
done. Two non-blocking follow-ups remain (perf triage + B6 carry-forward, below).

**Verify green:**
```bash
cargo run -p xtask -- gate          # fmt + clippy (default+demo) + tests + drift. exit 0.
LENS_THEME=dark cargo run -p lens-app --release --features demo -- --demo   # the demo
```

## What the visual pass changed (commit `7396afc`)

RISK #1 (glyph tint) **passed** — the Lucide `stroke="currentColor"` SVGs tint correctly via
`svg().text_color()`; no `fill` change needed. Everything else confirmed on-device. Fixes/polish:

- **Line-height clipping (real bug).** gpui's `text_*` helpers set font-size but not line-height,
  so `overflow_hidden` (needed for ellipsis) shaved ascenders on single-line rows. Fix:
  `ellipsize_line` now sets `line_height(1.4)`; activity row uses `min_h(16)` + `text_xs`.
- **Card height 148 → 160** (`CARD_HEIGHT_PX`) — breathing room once line-heights were corrected.
- **Context bar recolored** — now **utilization threshold** (green ≤50% / amber ≤75% / red above,
  `t.base.success/warning/danger`), a budget signal independent of the card's wave color. This is a
  deliberate deviation from the mockup (bar = card color); recorded in spec §11 + §6 B2.
- **Repo row → Lucide SVG icons.** `folder` + `git-branch` glyphs (bundled ISC, `assets/icons/`)
  replace the `📁`/`⑂` emoji/fork. Rendered as a flex row (`repo_entry`/`render_repos_row`), not a
  formatted string. Overflow tooltip (`·+N`) uses the same glyphs, is **gated to `repos.len() > 1`**,
  and draws in gpui-component's themed `Tooltip` box.
- **Harness·model into the header meta** — aligned under the title (past the tile); the 44px tile is
  **vertically centered** against the resulting 3-line stack. The tile+stack wrapper has **NO
  `overflow_hidden`** on purpose — the Scheduled countdown ring's `inset(-4)` canvas must not clip
  (this bit us once; don't re-add it).
- **Title tooltip** — full title on hover (gated to a real title).
- **Board grid** (`board/mod.rs`) — `28px` gap (≥ 2× the expanding-ring's 12px reach, so neighbors'
  breathe animations don't bleed) + `28px` padding + `justify_center`/`content_start`.
- **Demo data/window** (demo-only, `main.rs`) — populated `repos` (incl. one 3-repo card for the
  overflow tooltip), varied tokens (amber/red bars), pushed the Scheduled wake to +45m so it holds
  through a viewing, and sized the window for a centered **4×2** grid.

**Files:** `crates/lens-ui/src/card/chrome.rs` (bulk), `board/mod.rs`, `card/model.rs`, `assets.rs`
(+ `folder.svg`/`git-branch.svg`), `card/mod.rs`, `crates/lens-app/src/main.rs`, spec §6/§11.

## Review

codex (`codex exec -s read-only`, gpt-5.x, cross-family) reviewed the full working-tree diff:
**one Low finding** — render-level test coverage for `render_repos_row` (empty/1/3 repos) was
dropped when the string-formatter tests were replaced by `repos_overflow_badge` badge tests.
**Declined** with rationale: element-tree asserts are false-green under gpui `NoopTextSystem`
(memory `gpui-test-noop-text-system`); the pure badge logic is covered and the empty/single/multi
cases were validated on-device. No correctness/layout/lifetime bugs found.

## Still open (non-blocking — do NOT re-open the visual pass)

1. **Perf/sweep triage (Follow-up 2 from the prior handoff — UNTOUCHED).** On-device release: ~12%
   CPU for 5 fast cards vs the spike's 8.8% budget (~2.3%/card vs 1.7%, ~30–35% over). Cadence is
   perfect (30fps fast / 1Hz Scheduled / 0 static, no drops). The overage is the new-since-spike
   per-frame canvas sweep (`render_sweep_overlay`, 2× `paint_path`) + SVG spinner re-transform.
   **Decision still open:** accept (subtle-shimmer quality bought it, scales sub-linearly, no drops)
   OR profile + reduce with Instruments. Also bundled: sweep feather not uniform across the slant
   (gpui 2-stop AABB-gradient limit) + `SWEEP_SKEW_DEG` sign vs the mockup. Rig + detail in
   `docs/spikes/2026-07-17-wave-build-perf.md` and the prior handoff's Follow-up 2.
2. **B6 viewport re-entry stuck-card (carry-forward).** Unreachable in this non-scrolling build;
   rides with B6's scroll container. Mechanism + the fix constraint (scroll container must
   re-evaluate the anim gate at the viewport edge; do NOT notify from the paint closure) in the
   prior handoff's Follow-up 3. Add a Scheduled↔offscreen↔onscreen transition test when building B6.

## By-design / not-bugs (unchanged from prior handoff)

- **B5 Wake/Retry → no-op `FleetStore` seams** — real behavior lands with state-model wake=respawn.
- **pbar track = `white 0.06`** — intentional mockup neutral.
- **All demo context bars were 24%** before this pass — uniform demo data, not a bug; now varied so
  amber/red are visible.

## Delegation notes

This pass was Opus-driven inline (fast iterative on-device loop with the user judging pixels — no
composer, since it was tightly-coupled UI tuning, not decomposable build slices). Review = codex
cross-family on the whole diff. Merge = ff-only to `main` + push (solo-project convention, memories
`integration-workflow` / `commit-when-finished`).
