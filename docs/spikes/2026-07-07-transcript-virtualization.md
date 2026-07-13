# Spike — transcript variable-height virtualization (framework §4.1c/d gate)

**Date:** 2026-07-07 (run 2026-07-08)
**Verdict:** **GO on native gpui `list()`.** All four transcript §16 contracts +
both structural checks + the stick-don't-yank UX demo pass on gpui's native
`list()` / `ListState`. The "need a custom variable-height virtualizer" residual
(framework §4.1c/d, transcript §19 note 3) is **retired** — gpui ships exactly
that primitive, purpose-built for chat logs.
**Mechanism:** native `list()` (Backend A), **not** gpui-component's
`v_virtual_list` (Backend B, which fails the scroll-anchoring/bottom family).

Design: `docs/specs/2026-07-07-transcript-virtualization-spike-design.md`.
Harness (throwaway): `spikes/transcript-virtual/` (+ `NOTES.md` = raw discovery log).

---

## What the spike asked

Framework §4.1c/d's go/no-go: can a variable-height virtualizer satisfy
transcript **§16's four contracts** on gpui — (1) scroll anchoring when an
off-screen item changes height, (2) windowing, (3) variable per-item heights,
(4) jump-to-bottom — plus stable identity across recycle, markdown-component
nesting, and a minimal stick-don't-yank demo? And **which mechanism**: gpui
native `list()` (A) or gpui-component's virtualized list (B)? Verdict method:
instrumented probe assertions **and** human eyeball ("Both").

## What was built

A disposable gpui binary (`spikes/transcript-virtual/`, outside the lint wall)
with **both** candidates behind one `RowSource` seam (env/CLI-selected), driven
by a synthetic fixture (N rows, mixed heights incl. tall code/image/tool-span
rows, a growing last item, a mutable off-screen item) plus a real-capture
replay path. Seven keybind-triggered probes, each with a **baked-in assertion**
and an on-screen PASS/FAIL readout. Row state is a retained `Entity`-per-item in
a store keyed by item id; the list renders a *handle* into it.

## The §5 gate (resolved in Phase 0)

Probe 1b needs a programmatic read of the logical scroll anchor
`(top-item-index, sub-offset)`. The introspection surface **splits by candidate**:

- **Native `list()`**: `ListState::logical_scroll_top() -> ListOffset {
  item_ix, offset_in_item }` — a first-class getter, exactly the anchor pair.
  Plus `ListAlignment::Bottom` ("like a chat log"), where the pinned state reads
  as `item_ix == count`. **1b is fully machine-checkable.**
- **gpui-component `v_virtual_list`**: pixel offset only (`ScrollHandle::offset()`);
  `top_item()` indexes *painted* children, not logical rows; no bottom-alignment.
  1b's anchor must be **harness-derived** from `offset().y` + a cumulative-height
  table.

## Results — the head-to-head

| # | Contract / check | A — native `list()` | B — gpui-component `v_virtual_list` |
|---|------------------|---------------------|-------------------------------------|
| 1 | Windowing | **PASS** `renders=21 ≪ N`, frame cost flat | **PASS** `renders=21 ≪ 200` |
| 3 | Variable heights | **PASS** `render_calls ≈ visible+overdraw`, not all N | **PASS** `render_calls=23, not all N` |
| 1a | Stick-to-bottom (append) | **PASS** stays pinned (`ListAlignment::Bottom`) | **FAIL** — no persistent bottom-alignment |
| 1b | **Off-screen anchor** (go/no-go) | **PASS** — anchor held under off-screen-above height mutation | **FAIL** — drifted `(100,16px)→(98,0px)`; scrollable div does not compensate `scroll_y` when above-content grows |
| 4 | Jump-to-bottom | **PASS** initial `logical_scroll_top=(200,0px)` | **FAIL** — opens at top (`at_bottom=false`) |
| 6 | Identity across recycle + nesting | **PASS** `EntityId(2v1)→(2v1)`, `inits ≤ 1`; markdown row full-height, no inner scroll | **PASS** `EntityId(2v1)→(2v1)` |
| 7 | Stick-don't-yank UX demo | **PASS** pause + "N new" + resume | **FAIL** — can't resume; offset-polling has no reliable "at bottom" |

**Backend A: 7/7.** **Backend B: passes pure virtualization + identity (windowing,
variable heights, recycle identity), fails the entire scroll-anchoring/bottom
family (1a, 1b, jump-to-bottom, UX resume).** This matched the Phase 0/Phase 2
predictions from the API surface — a cross-check that the probes measure the real
invariants.

### The 1b result in detail (the true go/no-go)

Scroll to a mid-list anchor, then grow an off-screen item *above* the viewport:
- **A** — `logical_scroll_top()` is unchanged; gpui's `list()` anchors by logical
  item + sub-offset, so above-viewport reflow does not shift what's visible. The
  concern flagged at build time (does gpui actually compensate pixel reflow into
  an unchanged logical anchor, or does the getter merely *exist*?) resolved in
  favor of real compensation.
- **B** — the derived anchor drifted two rows + sub-offset (`(100,16px)→(98,0px)`).
  `v_virtual_list` is a scrollable `div`; unchanged `scroll_y` + taller
  above-content ⇒ the same pixel window now covers different logical rows. No
  anchor preservation. (Machine FAIL, corroborated by eyeball shift — a valid
  verdict, not a false red.)

## Probe-validity corrections (the false-verdict guard earned its keep)

Two probe bugs were caught and fixed *before* they poisoned the verdict — the
exact risk the spec flagged (a mismeasuring probe → false PASS/FAIL):

1. **Keybindings dead** — actions were bound to `key_context("Harness")` but no
   `FocusHandle` was ever focused, so the context never activated. Fixed:
   `FocusHandle` on the root view, focused on construction, `.track_focus()`.
   (Also remapped `shift-6/2/3` → `b/9/0`; gpui keymatch is unreliable for
   shifted number keys.)
2. **False identity FAIL** — probe 6 asserted `markdown_inits_after ==
   markdown_inits_before`, but the baseline is captured *before* the off-screen
   target's first paint (`before=0`), so a normal `0→1` first-init read as a
   re-init. The entity id was **identical** (`2v1→2v1`) — identity held. Fixed to
   the real invariant: `inits_after <= 1` (a genuine re-init shows `1→2`; a
   recreated entity fails the entity check).

## Conclusion & recommendation

**Build the transcript scroll surface on gpui's native `list()` + `ListState` +
`ListAlignment::Bottom`,** with each row a retained id-keyed `Entity` (reusing
the §4.1 markdown component, `.scrollable(false)`, per row). This satisfies all
four §16 contracts natively — no custom virtualizer, no fork, no extra dep.

**Do not use gpui-component's list for the transcript.** It virtualizes and
preserves identity, but structurally lacks bottom-anchoring and logical-anchor
readout; reproducing them over a scrollable div is unreliable (and would be
strictly more work than `list()` for a worse result). This **divides the
dependency story cleanly**: gpui-component for markdown (§4.1) and the §4.3 form
inputs; **native `list()` for the transcript scroll surface.**

Framework §4.1c/d and transcript §19 note 3 update from "unproven spike / may
need a custom virtualizer" to **resolved: native `list()`.**

## Deferred / not covered

- Frame-cost **flatness sweep** across N (9→1→0→1) was not fully exercised on B
  (both samples N=200); windowing is nonetheless proven by `renders ≪ N` on both.
  Absolute debug-build frame times (~8ms) are not load-bearing — the ratio and
  the `renders ≪ N` count are.
- Real ViewBlock projection pipeline, disk-paint→reconcile flash-free merge
  (transcript §17), reasoning capped-region scroll (§7), the polished
  `↓ N new · jump to latest` pill — all implementation, post-verdict.
- A live server-driven transcript (this used the synthetic fixture + replay).
