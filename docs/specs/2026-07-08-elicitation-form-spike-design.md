# Spike design — JSON-Schema elicitation form renderer harness

**Date:** 2026-07-08
**Status:** Design approved; ready for execution (no separate plan — spec phases drive it)
**Owner:** Lens design effort
**Type:** Throwaway verification spike (discarded after findings)

Closes (or re-opens) the framework §4.3 residual — **the last load-bearing
un-spiked framework item** (STATUS: "Only §4.3 remains as a load-bearing un-spiked
framework residual"). See `docs/design/framework.md` §4.3 and
`docs/design/permissions-and-elicitations.md` §2/§3. Third in the framework-spike
series after the 2026-07-07 markdown and 2026-07-08 virtualization spikes; reuses
their calibration.

**Process note (deliberate):** spec-only. No writing-plans artifact and no TDD —
this is disposable exploratory code, so the phases below *are* the plan, and probe
correctness is enforced by assertions baked into the probes (verification *as* the
experiment), not tests-first. Rigor is spent on probe validity, ground-truth
fidelity, and the findings conclusion — not on regression-proofing code that gets
deleted. Matches memories `iterative-skills` and `review-spend-policy`.

---

## 0. The reframe this spike is built on (ground truth)

The framework doc frames §4.3 as *"render an **arbitrary** `requested_schema` JSON
Schema as a gpui input form (string/number/enum/boolean/**nested-object** fields)."*
A read of omnigent 0.4.0 (sibling checkout, source-grounded) shows that framing is
**over-scoped in one direction and under-scoped in another**:

- **Over-scoped — "arbitrary / nested" is not real.** omnigent's own auto-fill
  helper `omnigent/tools/_elicitation_schema.py::build_accept_content_from_schema`
  never recurses: a property that is not a flat primitive returns `None` (decline).
  The MCP elicitation contract is a **flat object of primitive properties** —
  `type: string | number | integer | boolean`, `enum`, or `oneOf: [{const}]`, with
  optional `default`. `content` values are constrained to
  `str | int | float | bool | list[str] | null`. **No nesting, no arrays-of-objects.**
  The hard "recursive schema" work the doc implies does not exist.

- **Under-scoped — the real surface is a discriminated set, not one form.**
  omnigent's shipping web renderer `web/src/components/blocks/ApprovalCard.tsx` has
  **no general JSON-Schema form renderer at all.** It resolves in priority order:
  1. **URL mode** — link / inline approve (both of Lens's live captures were exactly
     this: `mode:url`, `requestedSchema:{}`, policy `approve_file_ops`).
  2. **ExitPlanMode** — plan markdown + approve / approve-in-auto / reject-with-feedback.
  3. **AskUserQuestion** — the *one genuine multi-field form*: a carousel of questions,
     radio/checkbox options, a "type something" custom row → flat
     `{questionKey: scalar | string[]}` (`AskUserQuestionForm.tsx`, `askUserQuestion.ts`).
  4. **Codex command approval** — structured command card.
  5. **Option buttons** — `{properties:{answer:{enum}}}` → buttons. **Dormant — no
     current producer** (kept for future MCP flows).
  6. **Binary approve/reject** — everything else, plus Claude `allowAllEdits` /
     `remember` extra buttons.

- **The genuine runtime schema→form case** only fires for **third-party MCP servers**
  attached to a session — omnigent *does* forward their `requestedSchema` to the client
  stream (`omnigent/runner/mcp_manager.py:279-285`) — and is bounded to the flat-primitive
  subset above.

**Verification status of this reframe:** derived from omnigent source + types, **not
byte-verified from a live form-mode capture** (both live captures were url-mode). Flagged
⚠ throughout, same discipline as the "ReasoningClosed NOT-byte-verified" flag in the
Plan 3b-1 normalization work.

The spike is scoped to this reality: prove the **generic flat-primitive
schema→inputs mapper** (the actual gpui unknown) *and* that it composes with the
**real discriminated surface** (AskUserQuestion as the concrete form instance;
binary / url / plan-review as thin cards).

---

## 1. Goal & pass/fail

Answer the framework §4.3 go/no-go. `gpui-component` 0.5.1 already ships the field
*inputs* (Input / Select / Checkbox / NumberInput / Switch); `gpui-form` 0.5.1 derives
forms from Rust *structs* at **compile time** — the wrong shape for a runtime-arbitrary
schema, but it confirms the input primitives. So the unproven, load-bearing part is the
**runtime** mapping:

> **The crux.** Can `gpui-component`'s Entity-backed input widgets be instantiated from
> a **runtime-parsed** schema into a **heterogeneous, runtime-sized** collection, hold
> their state app-side keyed by field, and read back on submit into a valid flat
> `ElicitationResult.content` (`{field: str | i64 | f64 | bool | list[str]}`) — with
> `required` gating submit, `default` prefill, and **never a panic** on malformed input?

This is the analog of the markdown spike's "no-remount on append" and the
virtualization spike's "1b off-screen anchor": the single thing that, if it fails,
sinks the native-gpui path.

**The probe contract** — each probe carries a baked-in assertion + on-screen PASS/FAIL:

| # | Probe | The load-bearing part |
|---|-------|-----------------------|
| 1 | **Runtime dynamic form** (the crux) | N inputs built from a parsed schema; state in a map keyed by field; read-back → typed content. N and the input *types* are known only at runtime. |
| 2 | **Type coverage** | `string→Input`, `number/integer→NumberInput`, `boolean→Switch`, `enum` / `oneOf:[{const}]→Select`. Each renders the right widget from the schema alone. |
| 3 | **Constraints** | `required` gates submit; `default` prefills the input; number-parse / required-empty errors render **inline**, never panic. |
| 4 | **Content round-trip** | read-back matches the MCP flat value types; assert by re-serializing `content` and comparing to an expected JSON (values are `str/i64/f64/bool/list[str]` only). |
| 5 | **AskUserQuestion instance** | the carousel (radio/checkbox, `multiSelect`, "type something" custom row) drives the **same** state machinery → flat `{questionKey: scalar \| string[]}`; every-question-answered gates submit. |
| 6 | **Composition + fallback** | a discriminator picks form / binary / url / plan-review from the params; a malformed or over-complex schema degrades to a **raw key/value editor** (framework §4.3's stated fallback), never a crash. |
| 7 | **UX eyeball** | the form fits a composer-dock-sized panel (permissions §3); submit / cancel legible; a multi-field schema is not visually overwhelming. |

**Outcomes:**

- **PASS / GO** — probes 1–6 hold and the UX eyeball (7) is acceptable. §4.3's
  "hand-rolled arbitrary JSON-Schema renderer" residual **retires**: the real build is
  a bounded flat-primitive mapper over `gpui-component` inputs + a handful of
  structured-payload cards.
- **PARTIAL** — the mapper works but a specific widget/behavior needs vendoring or a
  patch (record which, like the markdown spike's three localized fixes).
- **FAIL / NO-GO** — runtime dynamic instantiation or read-back is structurally
  blocked → escalate the framework §4.3 fallback ladder (url-mode approval page /
  raw key-value editor as the *primary* path, not the fallback).

---

## 2. What gets built

A disposable gpui binary at **`spikes/elicitation-form/`** (outside the workspace lint
wall, like `spikes/markdown-stream/` and `spikes/transcript-virtual/`), depending on
**`gpui = 0.2.2`** + **`gpui-component = 0.5.1`** — the pair the markdown spike already
proved builds against the §3 pin with no reconciliation.

Structure, behind one seam so the fixture is swappable:

- **`SchemaForm`** — the unit under test. Input: a parsed `requestedSchema` (flat
  object). Builds a `Vec<FieldState>` (one per property; each owns/holds the relevant
  `gpui-component` input Entity + its metadata: key, kind, required, default). Renders
  the inputs; exposes `submit() -> Result<Content, Vec<FieldError>>` that reads every
  input back into a flat `serde_json::Map` of MCP-valid values.
- **A tiny schema model + parser** — maps a `requestedSchema` property to a `FieldKind`
  (`String | Number | Integer | Bool | Enum(Vec<String>) | Unsupported`). `Unsupported`
  (nested object, array-of-object, unknown) routes the *whole form* to the raw
  key/value fallback (probe 6).
- **`ElicitationCard`** — the discriminator (probe 6): reads a fixture's
  `ElicitationRequestParams`-shaped input and picks `Form(SchemaForm)` / `Binary` /
  `Url` / `PlanReview` / `AskUserQuestion`. Thin cards for the non-form shapes (enough
  to prove composition, not production chrome).
- **`AskUserQuestionForm`** — the carousel instance (probe 5), a faithful port of the
  omnigent web form's behavior (radio/checkbox, multiSelect, custom "type something"
  row, all-answered gate) onto the same `FieldState` read-back path.
- **Probe harness** — keybind-triggered probes, each with a baked-in assertion and an
  on-screen PASS/FAIL readout (the prior spikes' pattern). A fixture picker cycles the
  corpus.

---

## 3. Fixtures (source-grounded, ⚠ derived-not-byte-verified)

Hand-built from omnigent source I have read, each flagged with its provenance:

- **`generic_full`** — a flat-primitive `requestedSchema` exercising every `FieldKind`
  + `required` + `default` + `enum` + `oneOf:[{const}]`. Grounded in the subset
  `build_accept_content_from_schema` accepts.
- **`ask_user_question`** — a real-shaped `questions[]` payload (single-select,
  multi-select, `header`, option `description`/`preview`, an `isOther` custom row),
  grounded in `askUserQuestion.ts`'s `ClaudeQuestion`/`ClaudeQuestionOption` types.
- **`exit_plan_mode`** — `{plan: "<markdown>"}` structured payload.
- **`binary`** — no schema / `{approve: boolean}` (the policy-ASK shape; matches the
  live captures modulo mode).
- **`url`** — `mode:url` relative `/approve/...` (verbatim shape from the live capture).
- **`malformed`** — a nested-object / unknown-type schema, to drive probe 6's
  raw key/value fallback.

**One live capture attempted, not gated on:** drive a real MCP server that emits a
form-mode `requestedSchema` (or a Claude `AskUserQuestion` turn) against the live pinned
0.4.0 server, byte-verify one real shape, fold it into the corpus + flip the relevant
⚠ flag if it lands. If the capture doesn't cooperate (no form-emitting MCP server handy,
subscription constraints — the recurring capture-box limitation), the spike proceeds on
synthetic fixtures and the verdict records the gap, exactly as prior spikes did.

---

## 4. Outputs

- **Verdict/findings doc** `docs/spikes/2026-07-08-elicitation-form.md` — the go/no-go,
  the probe result table, which `gpui-component` inputs were used (and any that needed
  vendoring/patching, PARTIAL-style), the live-capture outcome, and the residual for
  the eventual consumer build.
- **Doc reconciliation** (the reframe is durable, the harness is not):
  - `framework.md` §4.3 — replace "arbitrary … nested-object" with the discriminated-set
    reality + the MCP flat-primitive bound; record the verdict + mechanism
    (`gpui-component` inputs, native runtime mapper).
  - `permissions-and-elicitations.md` §3 — the widget's mode table currently lists only
    Binary / Form / URL; add the **structured-payload modes actually emitted**
    (AskUserQuestion, ExitPlanMode, Codex command) and the Claude `allowAllEdits` /
    `remember` extras, plus the ⚠ **AskUserQuestion caveat**: for the *native Claude*
    path the submitted answers are **cosmetic + an approval surface — they do not
    propagate back to the agent** (the PermissionRequest hook returns only allow/deny;
    the real answer flows through Claude's own TUI picker — `askUserQuestion.ts` header
    comment). Only a *genuine MCP elicitation*'s `content` propagates.
  - STATUS — retire "§4.3 is the only load-bearing un-spiked framework residual"; the
    framework spike series closes.
- **Memory** — a `elicitation-form-spike-2026-07` entry with the reframe + verdict, and
  the fixture-provenance/caveat gotchas for the consumer build.

---

## 5. Non-goals (YAGNI)

- **Not the production widget.** No lens-client wiring, no `POST /events` / `/resolve`
  reply paths, no `target_session_id` mirror routing, no composer-dock integration, no
  url-validation boundary — all consumer-build concerns (permissions §3/§4/§5). The
  spike proves feasibility of the *renderer + read-back*, nothing downstream.
- **Not nested/recursive schema.** Out of MCP scope (§0); `Unsupported` → raw
  key/value fallback is the whole answer.
- **Not the Codex command card or the dormant enum-option buttons** beyond the
  discriminator recognizing them — no producer stress on them this spike (Codex needs a
  codex sub; option-buttons has no producer). Recognized-and-routed is enough.
- **Not polished chrome.** Thin cards; the eyeball check (probe 7) is "fits and is
  legible," not final visual design.
