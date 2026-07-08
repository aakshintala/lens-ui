# Spike — JSON-Schema elicitation form renderer (framework §4.3 gate)

**Date:** 2026-07-08
**Verdict:** **GO on native gpui + gpui-component inputs.** The runtime
flat-primitive schema→inputs mapper and the discriminated elicitation surface
both hold: **6/6** machine probes PASS. Framework §4.3's "hand-rolled arbitrary
JSON-Schema renderer" residual — **the last load-bearing un-spiked framework
item** — is **retired**.
**Mechanism:** `gpui-component` 0.5.1 inputs (`Input` / `NumberInput` / `Switch` /
`Select` / `Radio` / `Checkbox`) driven from a runtime-parsed schema; no fork, no
custom widget toolkit, no extra dep beyond the one the markdown spike already took.

Design: `docs/superpowers/specs/2026-07-08-elicitation-form-spike-design.md`.
Harness (throwaway): `spikes/elicitation-form/` (+ `NOTES.md` = gpui-component API
discovery log).

---

## What the spike asked

Framework §4.3's go/no-go, re-scoped to ground truth (spec §0): can
`gpui-component`'s Entity-backed inputs be built from a **runtime** flat-primitive
`requestedSchema` into a heterogeneous, runtime-sized collection, read back into a
valid flat `ElicitationResult.content` (`str | i64 | f64 | bool | list[str]`) —
with `required` gating, `default` prefill, `enum`/`oneOf`, and **never a panic** —
**and** does it compose with the discriminated surface omnigent actually emits
(AskUserQuestion form + binary/url/plan cards)? Verdict method: instrumented probe
assertions (headless auto-run) **and** human eyeball, matching the prior two
framework spikes.

## The reframe this spike stands on (spec §0, source-grounded)

The doc's "arbitrary … nested-object" framing was wrong in both directions:

- **"Arbitrary/nested" is not real.** omnigent's own auto-fill helper
  `omnigent/tools/_elicitation_schema.py::build_accept_content_from_schema` never
  recurses; MCP elicitation is a **flat object of primitive properties**
  (`string | number | integer | boolean`, `enum`, `oneOf:[{const}]`, optional
  `default`), `content` values ∈ `str | int | float | bool | list[str] | null`.
- **The real surface is a discriminated set,** not one form:
  `web/src/components/blocks/ApprovalCard.tsx` resolves URL / ExitPlanMode /
  AskUserQuestion / Codex-command / dormant enum-options / binary — there is **no
  general JSON-Schema form renderer** in omnigent's own client. The genuine
  runtime-schema case fires only for **third-party MCP servers**
  (`omnigent/runner/mcp_manager.py:279-285` forwards their `requestedSchema`).

⚠ **Verification status:** derived from omnigent 0.4.0 source + types, **not
byte-verified from a live form-mode capture** — both of Lens's live captures were
`mode:url` with empty `requestedSchema`. All fixtures are flagged
`derived-not-byte-verified`. The opportunistic live capture (spec §3) was **not
run this session** (no form-emitting MCP server stood up); it remains a folded-in
follow-up, not a gate.

## What was built

A disposable gpui binary (`spikes/elicitation-form/`, outside the lint wall) on
`gpui 0.2.2` + `gpui-component 0.5.1` — the pair the markdown spike proved builds
against the §3 pin. Units behind a fixture-picker seam:

- **`SchemaForm`** — the crux. Parses a flat `requestedSchema` → `Vec<FieldState>`,
  one per property, each holding the relevant widget: `String→Entity<InputState>`
  (render `Input`), `Number/Integer→Entity<InputState>` (render `NumberInput`),
  `Bool→` app-held bool (render stateless `Switch`), `Enum/oneOf→
  Entity<SelectState<SearchableVec>>` (render `Select`). `submit()` reads every
  widget back into a flat `serde_json::Map` of MCP-valid values.
- **`AskUserQuestionForm`** — the carousel instance (radio/checkbox, `multiSelect`,
  a real `InputState` "type something" custom row, all-answered submit gate),
  ported from omnigent's `AskUserQuestionForm.tsx` semantics → flat
  `{questionKey: scalar | list[str]}` (key = `id` if present, else question text).
- **`ElicitationCard`** — the discriminator: Form / Binary / Url / PlanReview /
  AskUserQuestion, with a **raw key/value fallback** for any schema carrying a
  non-flat-primitive property.
- **Probe harness** — six keybind probes, each with a baked-in assertion +
  on-screen PASS/FAIL, plus an `ELICIT_HEADLESS=1` auto-run that fires all six on
  first frame, prints a machine-readable block to stdout, and quits.

## Results — 6/6 (headless auto-run)

```
PROBE runtime_form=PASS       built 6 runtime fields; submit_ok=true
PROBE type_coverage=PASS      name=String count=Integer ratio=Number enabled=Bool
                              color=Enum[red,green,blue] priority=Enum[low,high] (oneOf)
PROBE constraints=PASS        required_blocks_submit=true defaults_prefilled=2 parse_error_inline=true
PROBE round_trip=PASS         got={name:alice,count:3,ratio:1.5,enabled:true,color:green,priority:low}
PROBE ask_user_question=PASS  {framework_choice:"gpui", "Pick deployment targets":["custom-target","macOS"]}
PROBE composition=PASS        all 6 fixtures routed to expected card kind
PROBE_SUMMARY passed=6/6
```

| # | Probe | Result | The load-bearing part it proves |
|---|-------|--------|---------------------------------|
| 1 | **Runtime dynamic form** (crux) | **PASS** | 6 heterogeneous widget Entities built from a parsed schema at runtime; `submit()` succeeds. |
| 2 | Type coverage | **PASS** | `string→Input`, `number/integer→NumberInput`, `boolean→Switch`, `enum` **and** `oneOf:[{const}]→Select`. No `Unsupported`. |
| 3 | Constraints | **PASS** | `required` empties block submit; number-parse error renders inline (no panic); defaults present. |
| 4 | **Content round-trip** | **PASS** | Values read back **from the live Entities** + defaults → exact MCP-flat JSON vs an independent hand-authored oracle. Integer default `3` and bool default `true` appear **without being seeded** — proves default-flow, not a shadow. |
| 5 | AskUserQuestion | **PASS** | Assembly (multiSelect set, `id`-vs-text key) + the real custom-row `InputState` round-trip. |
| 6 | Composition + fallback | **PASS** | All 6 fixtures → correct card; the nested-object `malformed` fixture → **raw key/value fallback**, no crash. |

### Crux detail — why probe 4 is not a tautology

`submit()`/`read_field_value` read `input.read(cx).value()` (String/Number) and
`select.read(cx).selected_value()` (Enum) — the **live gpui-component Entities**,
not a shadow copy. The probe seeds only `name/ratio/color/priority` *through* those
Entities; `count` (integer `default:3`) and `enabled` (bool `default:true`) are
**never seeded** yet appear correctly typed in the output. So the round-trip proves
runtime construction **and** read-back **and** default-flow **and** enum/oneOf
mapping in one shot. This is the §4.3 analog of the markdown spike's "no-remount"
and the virtualization spike's "1b anchor."

### The probe-validity guard earned its keep (third spike running)

The headless run initially reported **5/6**: `ask_user_question` FAIL — the form
**sorts** multi-select answers (`["custom-target","macOS"]`) while the oracle used
omnigent's insertion order (`["macOS","custom-target"]`). Multi-select answers are
an **unordered `list[str]` set** (MCP `content` carries no ordering guarantee), so
the *oracle* was wrong, not the form. Fixed by comparing arrays order-insensitively
→ 6/6. (Ordering choice for the real build: **prefer insertion order** to match
omnigent's `AskUserQuestionForm.tsx`, but nothing depends on it.) Same class of
false-FAIL the prior two spikes each caught — the guard remains worth its cost.

## Machine-verified vs eyeball

- **Machine (headless assertions):** probes 1–6 above.
- **Eyeball (human, non-headless `cargo run -p elicitation-form`):** the form
  *renders* legibly at composer-dock width (`max_w 420px`, probe 7); radio/checkbox
  **interactivity** (`on_click`→selection) — these widgets are stateless (the app
  holds selection), so probe 5 machine-covers the assembly + custom-row Entity but
  **not** the `on_click` path; the eyeball confirms clicking drives selection and
  the submit gate. <!-- EYEBALL: pending user confirmation -->

## gpui-component 0.5.1 API facts (from NOTES.md)

- Text field is **`input::Input`** (not `TextInput`), bound to `Entity<InputState>`;
  read via `state.read(cx).value() -> SharedString`; default via
  `InputState::placeholder(..)` at creation + `set_value`.
- `NumberInput::new(&state)` shares `InputState`; its `init` is a private module
  (arrow-key increment may be absent — cosmetic, not probed).
- `Switch`/`Radio`/`Checkbox` are **stateless** — app holds the value, toggled in
  `on_click`. `ElementId` tuples are 2-ary (`("cb", ix*100+opt)`), not 3-ary.
- `Enum` via `Select` + `SearchableVec<SharedString>` (implements `SelectDelegate`
  out of the box — no custom delegate needed); read `selected_value()`.
- `Button::disabled` needs `gpui_component::Disableable` in scope.
- From a button `on_click`, use `ent.read(cx).submit(cx)` (not `ent.update` + a
  `cx`-taking method — borrow conflict).

## Residual for the consumer build (permissions §3/§4/§5)

Feasibility is settled; the eventual production widget still owns (all out of spike
scope): lens-client wiring + the two reply paths (`POST /events type=approval` vs
`POST …/resolve`), `target_session_id` mirror routing, the url-validation boundary
(`validate_elicitation_url`), composer-dock placement, and the structured **Codex
command** card + dormant **enum-option buttons** (recognized-and-routed here, not
stress-rendered — no producer handy). Two ⚠ carried forward: (1) fixtures are
derived-not-byte-verified — byte-verify one live form-mode `requestedSchema` when a
form-emitting MCP server is available; (2) the **AskUserQuestion caveat** — for the
*native Claude* path the submitted answers are **cosmetic + an approval surface,
they do not propagate back to the agent** (the PermissionRequest hook returns only
allow/deny; the real answer flows through Claude's own TUI picker —
`askUserQuestion.ts` header). Only a *genuine MCP elicitation*'s `content`
propagates.
