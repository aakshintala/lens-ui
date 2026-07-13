# Spike — streaming-markdown stable identity (framework §4.1 gate)

**Date:** 2026-07-07
**Verdict:** **PARTIAL** — architecture is sound and liftable; three hardcoded
module behaviors make the *unmodified* dep streaming-hostile, all fixable by
**vendoring just the markdown module** (the path framework §4.1 already favored).
**Gate outcome:** the gpui lock **holds** — better-supported than before, with a
precisely-scoped residual (patch three spots in a vendored module).

Design: `docs/specs/2026-07-07-markdown-streaming-spike-design.md`.
Plan: `docs/plans/2026-07-07-markdown-streaming-spike.md`.
Harness (throwaway): `spikes/markdown-stream/` (+ `NOTES.md` = raw discovery log).

---

## What the spike asked

Framework §4.1's go/no-go: can we lift `gpui-component`'s markdown component and
stream into it with **stable element identity** (no remount / flicker /
scroll-jump), enforce the Lens link/image **sanitization boundary**, and — if
needed — **safe-prefix** streaming? Verdict method: instrumented probe **and**
human eyeball ("Both").

## What was built

A disposable gpui binary (`spikes/markdown-stream/`, outside the lint wall) that
replays a chunked delta stream into one retained `TextView::markdown` keyed by a
stable `ElementId`, sanitizes each accumulation (pulldown-cmark pre-parse
transform), and probes per-frame build cost. Sources: a GFM stress fixture, the
real 17 KB `framework.md`, and an adversarial fixture.

## Findings

### Feasibility — PASS
- `gpui-component 0.5.1` builds clean on this box (436 crates, ~1 m cold) and
  pulls **gpui 0.2.2 — the exact framework §3 pin**. The "pins its own gpui"
  caveat evaporates; no reconciliation needed.
- Public API: `TextView::markdown(id, text, window, cx)` + `.selectable(bool)` +
  `.scrollable(bool)`. Requires `gpui_component::init(cx)` and a `Root` wrapper.

### Stable identity (the core question) — PASS at the architecture level
- State (parsed content, selection, scroll) is a retained `Entity<TextViewState>`
  keyed by `use_keyed_state("{id}/state")`. Same id every frame ⇒ **no remount**.
- Re-parse runs async through a debounced `UpdateFuture`, **only on actual text
  change**, off the render path. Probe: build/tick mean **25 µs** over the 17 KB
  doc (4370 ticks), with build-time↔bytes correlation **−0.39** — flat/negative
  across 8× the bytes ⇒ **no O(n) per-frame reparse**. A synchronous
  full-reparse-per-frame renderer would show ms-scale, strongly-positive scaling.
- Finalize swap (full text, same id) is a no-op by construction.

### Three hardcoded behaviors break *naive* streaming — all in the vendorable module
| # | Behavior | Location | Observed effect |
|---|----------|----------|-----------------|
| 1 | 200 ms **trailing** debounce, hardcoded, resets on every update | `text_view.rs:628` (delay), `:168` (reset) | Updates faster than 200 ms perpetually reset the timer → **nothing renders until the stream pauses**, then whole sections appear at once. Confirmed live; re-running at 220 ms cadence (> debounce) restores progressive render. |
| 2 | `clear_selection()` on every reparse | `text_view.rs:610` | A text selection does **not** survive a streamed update. |
| 3 | `list_state.reset(children.len())` on every content change | `node.rs:1123` | gpui `ListState::reset()` re-inits scroll to top → **scroll jumps to the top on each render.** Confirmed live. Directly violates transcript §5 (in-place diff, no scroll-jump). |

### Sanitization boundary — feasible as a pre-parse transform
- `sanitize()` (pulldown-cmark → rewrite link/image events → `pulldown-cmark-to-cmark`
  reserialize) neutralizes `javascript:`/`data:`/unknown schemes to `about:blank`
  and converts external images to inert `[image: …]` text, before gpui-component
  sees the text. Unit-tested (3 tests). It treats the component as a black box, so
  it does not require touching the module. (Round-trip fidelity of to-cmark on
  exotic constructs is a known minor caveat, not load-bearing.)

### Toolchain finding — mdstitch needs Rust 1.95
- `mdstitch 0.1` (the survey's safe-prefix dep) **requires rustc ≥ 1.95**; the
  repo pins **1.91.1** (`rust-toolchain.toml`, deliberate). Deferred: given the
  debounce, intermediate mid-construct states rarely render anyway, so safe-prefix
  is **lower priority** than the scroll/selection fixes. Adopting mdstitch later
  is a deliberate toolchain-floor bump.

## Conclusion & recommendation

**Vendor just the markdown module** (Apache-2.0 permits it) and patch three
localized spots:
1. debounce policy — leading/throttle or configurable delay (or coalesce our own
   `Update::Text` sends to ≥ 200 ms as an interim, no-vendor mitigation);
2. drop `clear_selection()` on reparse;
3. replace `list_state.reset(len)` with a scroll-preserving splice / retained
   anchor.

This is neither the unmodified dep (streaming-hostile) nor a from-scratch renderer
(parser + tree-sitter highlight + element view all work). The gpui framework lock
(framework §1/§4.1) **holds**, with the residual reduced to a bounded vendor-and-patch.

## Deferred / not covered
- Variable-height virtualization (§4.1c/d) — separate spike (note: the module uses
  gpui `ListState`, a relevant building block to evaluate there).
- Real-build vendoring mechanics + gpui-pin compat beyond "0.5.1 → 0.2.2 matches".
- mdstitch adoption (toolchain bump) + safe-prefix at the fixed debounce.
- to-cmark round-trip fidelity audit.
