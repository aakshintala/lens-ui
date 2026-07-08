# Spike design — transcript variable-height virtualization harness

**Date:** 2026-07-07
**Status:** Design approved; ready for execution (no separate plan — spec phases drive it)
**Owner:** Lens design effort
**Type:** Throwaway verification spike (discarded after findings)

Closes (or re-opens) the framework §4.1c/d virtualization residual — the second
half of the transcript's gpui feasibility, deferred out of the 2026-07-07
markdown spike. See `docs/design/framework.md` §4.1 and
`docs/design/conversation-transcript.md` §16 (four contracts) / §19 note 3.

**Process note (deliberate):** spec-only. No writing-plans artifact and no TDD —
this is disposable exploratory code, so the phases below *are* the plan, and
probe correctness is enforced by assertions baked into the probes (verification
*as* the experiment), not tests-first. Rigor is spent on probe validity and the
findings conclusion, not on regression-proofing code that gets deleted. Matches
memories `iterative-skills` and `review-spend-policy`.

---

## 1. Goal & pass/fail

Answer the framework §4.1c/d go/no-go: **can a variable-height virtualizer
satisfy transcript §16's four contracts on gpui, and which mechanism do we
adopt?** The transcript is variable-height (expanded tool spans, code, images),
so gpui's `uniform_list` (uniform-height only) does not apply (transcript §19
note 3).

The four §16 contracts, with the hard part of each:

| # | Contract | The load-bearing part |
|---|----------|-----------------------|
| 1 | **Scroll anchoring** | viewport holds a stable logical anchor when (a) the *streaming last item grows* and (b) an *off-screen item above the viewport changes height*. **(1b) is the true go/no-go.** |
| 2 | **Windowing** | painted/measured items ≪ N at scale; per-frame cost ∝ visible, not N |
| 3 | **Variable heights** | mixed row heights; measure-and-cache (measure once until height changes, not every frame) |
| 4 | **Jump-to-bottom** | opening lands at the bottom (latest) |

Plus two structural checks that decide whether the mechanism composes with the
rest of the transcript design, run on both candidates:

- **Stable identity across recycle** — each row's state is a retained `Entity`
  held by the harness model, **keyed by item id** (the list renders a *handle*,
  it does not own row state). A row scrolled off and back must **not** remount:
  inner state (e.g. a markdown selection) survives. This is how transcript §5
  stable-identity composes with virtualization.
- **Nesting** — a real §4.1 markdown `TextView` (`.scrollable(false)`,
  full-height) placed in some rows. Confirms (i) the outer list hosts arbitrary
  child Entities and (ii) a markdown-component-that-is-itself-an-inner-list
  measures correctly full-height inside the outer virtualizer (no nested-scroll
  capture, no clipping).

And the minimal UX demo (not the polished pill): **stick-to-bottom auto-follow;
pause when the user scrolls up; a bare "N new" counter.** Just enough to prove
the scroll handle exposes the hooks — the polished `↓ N new · jump to latest`
pill is deferred (transcript §16).

**Outcomes (per candidate):**

- **PASS** — all four contracts + both structural checks + the UX demo hold.
- **PARTIAL** — holds but needs vendoring/patching (as §4.1 markdown did), or
  the 1b anchor holds only by eyeball (see §5 risk) — recorded with the exact
  residual.
- **FAIL** — full relayout of all N on append, no windowing, or 1b anchoring
  cannot hold → drop to the fallback ladder (custom measure-and-cache
  virtualizer, framework §4.1 / transcript §19), which becomes a new costed
  spike.

The deliverable is a findings verdict + evidence, not shippable code.

---

## 2. Architecture

Throwaway binary crate at `spikes/transcript-virtual/`. Automatic workspace
member (`members = ["crates/*", "spikes/*"]`) that deliberately does **not** opt into
`lints.workspace` — the throwaway/production wall (root `Cargo.toml`), same as
`spikes/markdown-stream/`.

**Dependencies:** `gpui-component` 0.5.1 + the gpui it pins (0.2.2 = the §3 pin;
confirmed compatible by the §4.1 spike, no reconciliation). No mdstitch (not
needed here). The §4.1 markdown component is reused for the nesting probe.

**A + B head-to-head behind one interface.** Both candidates render through a
single `RowSource` seam so the probes are byte-identical across them; an env/CLI
flag selects the backend:

- **Backend A** — gpui **native `list()`** + `ListState` + `ListAlignment::Bottom`.
- **Backend B** — `gpui-component`'s virtualized **`List`**.
- Backend C (custom) is **not built** — it is the documented fallback ladder.

**Modules** — each single-purpose, understandable and testable in isolation:

- `fixture` — the synthetic generator (§4) and the `.sse`-capture→rows projector.
- `rowsource` — the `RowSource` interface + the row model: a retained
  `Entity`-per-item store keyed by item id, plus the height-mutation and
  append-to-last hooks the probes drive.
- `backend_a` / `backend_b` — the two virtualizer bindings behind `RowSource`.
- `probe` — instrumentation: measure/render counters, frame-cost timer,
  `(top-item-index, sub-offset)` anchor recorder, auto-follow state log, and the
  baked-in assertions that turn a probe into a pass/fail signal.
- `app` — the gpui window wiring, keybindings that trigger each probe scenario,
  and an on-screen readout of the live probe numbers.

---

## 3. Fixture (§4 of the design)

One parametrized synthetic generator plus a real-capture replay:

- **Synthetic-at-scale (primary):** `N` rows (sweep, e.g. **200** and **2 000**);
  a **height distribution** — mostly one-liners with periodic tall rows (a code
  block, an image placeholder, an expanded tool-span block); **one growing
  streaming last item** (appends on a timer/keypress); and **one designated
  off-screen item above the viewport whose height mutates on a keypress** — this
  is the contract-1b trigger.
- **Real replay (fidelity):** one captured `.stream.sse`
  (`docs/spikes/captures/…`) projected to rows through a trivial item→row map.
  Our captures are short, so this validates realism, not scale — scale comes
  from the synthetic sweep.

---

## 4. Probes (instrument **and** eyeball — "Both", per §4.1)

Each probe carries a baked-in assertion so a green run is a machine-checked
signal, not a vibe. Eyeball confirms the assertion measures the real thing.

| Probe | Contract | Instrument | Assertion |
|-------|----------|-----------|-----------|
| Windowing | 2 | per-item paint/measure counter; frame-cost timer swept over N | painted ≈ viewport+buffer **≪ N**; frame cost **flat across N** (∝ visible) |
| Variable heights | 3 | per-item measure-call counter | measured **once** until its height changes, not per frame; eyeball: no clip/overlap |
| Anchoring 1a | 1 | scroll offset at bottom while appending to last item | stays **pinned to bottom**; eyeball: no jump |
| Anchoring 1b | 1 | `(top-item, sub-offset)` recorded before/after mutating the off-screen-above item's height | recorded anchor **unchanged**; eyeball: no visible shift |
| Jump-to-bottom | 4 | initial scroll offset on open | equals **bottom** |
| Identity/nesting | struct | select text in an inner markdown row, scroll off + back | selection **survives** (no remount); eyeball: full-height render, no nested scroll |
| UX demo | +  | auto-follow ↔ paused state-transition log; "N new" counter | pauses on scroll-up, resumes on scroll-to-bottom, counter increments while paused |

**Method note:** the frame-cost + windowing pair is the same instrumented
approach that produced the §4.1 ~25µs/frame number; a non-virtualizing renderer
shows ms-scale cost rising with N.

---

## 5. The one build risk (confirm in Phase 0)

Probe **1b** assumes `ListState` (or `gpui-component`'s equivalent) exposes the
**logical anchor** (top index + sub-offset) programmatically. If it exposes only
a pixel offset, the 1b probe degrades to **eyeball-only** — still a verdict, but
recorded as such (a PARTIAL qualifier, not a silent gap). **Phase 0 confirms the
`ListState` introspection surface first**, before the anchor probe is built on
an assumption.

---

## 6. Build sequence (the phases *are* the plan)

- **Phase 0** — scaffold `spikes/transcript-virtual/`; build `fixture`,
  `rowsource` (the `RowSource` seam + retained id-keyed store), and `probe`
  (backend-agnostic). **Confirm the `ListState` anchor-introspection surface
  (§5) before building the 1b probe.**
- **Phase 1** — Backend A (native `list()`) behind `RowSource`; wire all probes.
- **Phase 2** — Backend B (`gpui-component` `List`) behind the same interface.
- **Phase 3** — run the probe matrix: both backends × {4 contracts, identity,
  nesting, UX demo}; collect numbers + eyeball.
- **Phase 4** — verdict: findings doc
  `docs/spikes/2026-07-07-transcript-virtualization.md`; update framework §4.1c/d
  + transcript §19; write memory. Park/delete the harness.

**Delegation:** composer-2.5 (`cursor-delegate`) builds each phase against this
spec; Opus owns the `probe` interface design and result interpretation (the one
real failure mode is a mismeasuring probe → false verdict). Review is scoped to
**probe validity** (not a full cross-family pass on throwaway code) plus a
careful read of the findings conclusion.

---

## 7. Deferred / not covered

- The polished `↓ N new · jump to latest` pill + exact "N new" semantics
  (transcript §16) — implementation, post-verdict.
- Real ViewBlock projection pipeline (transcript §3/§4); disk-paint→reconcile
  flash-free merge (§17); reasoning capped-region scroll (§7).
- The production transcript widget itself.
- `gpui-component` version-pin reconciliation beyond "0.5.1 → gpui 0.2.2 matches
  the §3 pin" (already confirmed by the §4.1 spike).
