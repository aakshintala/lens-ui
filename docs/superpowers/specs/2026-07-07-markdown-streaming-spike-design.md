# Spike design — streaming-markdown stable-identity harness

**Date:** 2026-07-07
**Status:** Design approved; ready for implementation plan
**Owner:** Lens design effort
**Type:** Throwaway verification spike (discarded after findings)

Closes (or re-opens) the framework §4.1 gpui lock gate. See
`docs/design/framework.md` §4.1 and `docs/design/conversation-transcript.md`
§5/§19.

---

## 1. Goal & pass/fail

Answer the framework §4.1 go/no-go: **can we lift `gpui-component`'s markdown
component and stream into it with stable element identity?** Concretely verify
three of the four §4.1 residuals (virtualization is a separate spike, §7):

- **(a) Stable identity** — appending deltas and doing the
  `StreamingMessage`→`Message` finalize swap does **not** remount: scroll
  position holds, a live text selection survives, no flicker, and the append
  path does **not** re-parse / re-build the whole element tree every frame
  (transcript §5 "stable widget identity", §19 note 1).
- **(d) Safe-prefix streaming** — closed markdown constructs render immediately;
  the open trailing construct is held pending (via `mdstitch`) until it closes;
  no incomplete-syntax flicker (transcript §5).
- **(b) Sanitization boundary** — the Lens link/image policy (block
  `javascript:` / `data:` schemes; external images not inlined, per transcript
  "artifact-only inline images") can be enforced **without forking**
  gpui-component (framework §2.5, §4.1).

**Outcomes:**

- **PASS** — all three hold with `gpui-component` as a plain cargo dep.
- **PARTIAL** — works but requires vendoring / patching the markdown module
  (framework §4.1 permits vendoring just that module under Apache-2.0).
- **FAIL** — full element rebuild on append, or sanitization can't be injected →
  drop to the §4.1 fallback ladder (rung 1: hand-rolled `pulldown-cmark`→gpui
  element renderer).

The deliverable is a findings verdict + evidence, not shippable code.

---

## 2. Architecture

Throwaway binary crate at `spikes/markdown-stream/`. It is an automatic
workspace member (`members = ["crates/*", "spikes/*"]`) and deliberately does
**not** opt into `lints.workspace` — that is the throwaway/production wall
(root `Cargo.toml`).

**Dependencies:** `gpui-component` 0.5.1 + the gpui version *it* pins (we do
**not** reconcile with the §3 `0.2.2` pin — pin reconciliation is a real-build
concern, §7). Plus `mdstitch` (streaming safe-prefix) and `pulldown-cmark`
(sanitization pre-pass).

**Modules** — each single-purpose, understandable and testable in isolation:

- `replay` — reads a `.md` fixture or a `.sse` capture and emits deltas on a
  ~60fps timer, coalescing to a frame tick (transcript §5: never re-render per
  token). Interface: `next_frame() -> Option<Delta>` / accumulated text.
- `sanitize` — a `pulldown-cmark` pass over the accumulated text that
  neutralizes disallowed link schemes and strips non-artifact image inlines,
  **before** handoff to gpui-component. Interface: `sanitize(&str) -> String`.
- `render` — the gpui view. Feeds `mdstitch(sanitize(accumulated))` into **one
  retained** gpui-component markdown entity keyed by a fixed item id.
- `probe` — instrumentation + the adversarial-scenario controls (§4): counts
  parses / element-builds per frame, logs frame time, drives held-scroll /
  held-selection / finalize-swap.

**Data flow (per frame tick):**
`replay` → accumulated text → `sanitize` → `mdstitch` safe-prefix →
retained gpui-component markdown entity → `probe` records build count + timing.

---

## 3. Input corpus

The real captures are thin on markdown — the big live stream
(`docs/spikes/captures/live/20260627-234800-claude.stream.sse`) has only ~40
`output_text.delta` events and claude-sdk folds reasoning into short plain
deltas, so it does not stress GFM constructs or safe-prefix edges. Therefore the
primary input is **synthesized**:

- **Primary (stress):** a GFM-heavy fixture derived from a real design doc
  (e.g. `docs/design/framework.md` — tables, fenced code, nested lists, links)
  chunked into deltas. Exercises every construct and lands deltas mid-construct
  constantly.
- **Adversarial fixture:** a small hand-authored `.md` — `javascript:` and
  `data:` links, an external image, an autolinkable file path, and constructs
  left unterminated at EOF (open `**`, open code fence, half-written table row).
  Stresses (b) sanitization and worst-case (d) safe-prefix.
- **Real-capture smoke:** replay the `output_text.delta` sequence from a live
  `.sse` for realism.

---

## 4. Evidence — adversarial scenario + instrumentation

"No remount" is made a **logged fact**, not a visual vibe. While deltas append
at the bottom, the harness deliberately stages the remount failure modes:

- **holds a scroll offset** scrolled up (a remount resets it),
- **holds a live text selection** (a remount drops it),
- then triggers the **`StreamingMessage`→`Message` finalize swap**.

`probe` asserts:

- element-build / parse count per frame stays ~O(changed blocks), **not**
  O(whole doc) every frame;
- scroll offset and selection are unchanged across the finalize swap;
- frame time stays within a smooth-streaming budget.

Corroborated by eyeballing and an optional recorded GIF.

---

## 5. Sanitization approach

Enforce the Lens policy as a **pre-parse text transform**: a `pulldown-cmark`
pass rewrites/neutralizes dangerous URLs and non-artifact image inlines in the
markdown source before gpui-component ever sees them. This is
framework-agnostic and works even treating the component as a black box. If
gpui-component exposes a link/image hook, note it as a cleaner path. **Whether
the pre-transform is sufficient (vs. needing a component hook / fork) is itself
a finding.**

Policy for the spike (from transcript §6.1 / framework §2.5):

- Links: allow `http`, `https`, `mailto`, `file`; neutralize `javascript:`,
  `data:`, and unknown schemes (render as inert text).
- Images: artifact-scheme images may inline; external image URLs are **not**
  inlined (render as a link or placeholder).

---

## 6. Approaches considered

- **A (chosen):** `gpui-component` markdown as-is, instrumented for rebuild.
  Directly answers the spike question; a failure routes cleanly to the fallback
  ladder.
- **B (rejected):** hand-roll `pulldown-cmark`→elements from the start. That
  tests *our* renderer, not whether the lift works — it is the fallback, not the
  question.
- **C (rejected/premature):** build both side-by-side and compare. Only worth it
  if A fails.

---

## 7. Explicit non-goals

- **Variable-height virtualization (§4.1c / §4.1d)** — separate follow-up spike;
  `uniform_list` is uniform-height, the transcript is variable-height, but that
  is a list-mechanics concern separable from "does markdown stream."
- **Real-build vendoring** of just the markdown module, and **gpui-version-pin
  reconciliation** with the §3 pin — deferred to first build.
- **Performance budgets** beyond a smooth-streaming sanity check.
- The harness is **discarded** after the findings doc; no production quality bar.

---

## 8. Deliverable

A findings doc at `docs/spikes/2026-07-07-markdown-streaming.md`:
PASS / PARTIAL / FAIL, the evidence (build counts, held scroll/selection across
the swap, safe-prefix behavior, sanitization sufficiency), and — if not PASS —
which §4.1 fallback-ladder rung applies. Mirrors the recording style of
`docs/spikes/2026-06-25-transport-stability.md`. Update framework §4.1 and
`docs/STATUS.md` with the verdict.
