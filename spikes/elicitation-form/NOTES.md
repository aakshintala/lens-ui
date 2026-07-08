# elicitation-form spike — running notes

Throwaway harness for framework §4.3 go/no-go (runtime JSON-Schema → gpui-component
inputs + discriminated elicitation cards). Disposable code; this file is the durable
API discovery record.

## Build feasibility — PASS (2026-07-08)

- `cargo build -p elicitation-form` succeeds on gpui `0.2.2` + gpui-component `0.5.1`.
- `cargo run -p elicitation-form` opens a gpui window (headless agent: build + brief
  launch without panic is the bar; probes are keybind-driven on screen).

## gpui-component API facts (0.5.1, source-verified)

### App bootstrap

```rust
Application::new().run(|cx: &mut App| {
    gpui_component::init(cx);  // REQUIRED
    // NumberInput::init(cx) lives in private `input::number_input` — NOT re-exported.
    // NumberInput still renders; only up/down keybindings may be missing.
    cx.open_window(..., |window, cx| {
        let view = cx.new(|cx| HarnessView::new(window, cx)); // needs AppContext in scope
        cx.new(|cx| Root::new(view.into(), window, cx))
    });
});
```

### String fields — `Input` (not `TextInput`)

The public text field is **`gpui_component::input::Input`**, bound to `Entity<InputState>`:

```rust
let state = cx.new(|cx| InputState::new(window, cx));
// default / placeholder on InputState builder at creation:
cx.new(|cx| InputState::new(window, cx).placeholder("Type something"))
state.update(cx, |s, cx| s.set_value("text", window, cx)); // state.rs:599
let v: SharedString = state.read(cx).value();                 // state.rs:787
// render:
Input::new(&state)
```

`Input` has **no** `.placeholder()` builder — placeholder is on `InputState::placeholder` /
`set_placeholder`.

### Number / integer — `NumberInput`

Same `Entity<InputState>` as strings; render with `NumberInput::new(&state)`.
Parse `.value()` as `i64` / `f64` in app code; never panic on bad input.

### Boolean — `Switch`

Stateless render; app holds `bool`. Toggle in `Switch::new(id).checked(b).on_click(|&new, _, cx| ...)`.

### Enum — `Select` + `SearchableVec`

`SearchableVec<String>` implements `SelectDelegate` when items implement `SelectItem`
(String / SharedString work). Wiring:

```rust
let delegate = SearchableVec::new(vec![SharedString::from("a"), ...]);
let select = cx.new(|cx| SelectState::new(delegate, Some(IndexPath::default().row(ix)), window, cx));
Select::new(&select)
// read-back:
select.read(cx).selected_value() -> Option<&Value>
select.update(cx, |s, cx| s.set_selected_value(&SharedString::from("a"), window, cx));
```

**Workaround:** `SharedString::from(str_ref)` in closures requires **owned** `String`
(`SharedString::from(s.clone())`) — `&str` from outer stack does not satisfy `'static`
in `set_value` / `set_selected_value` closures.

**Radio-group substitute:** not needed — `SearchableVec` delegate worked without custom
delegate impl.

### Radio / Checkbox (AskUserQuestion)

Stateless; app holds selection. ElementId tuples: `("radio", usize)` / `("cb", usize)` —
**not** three-tuple `(&str, usize, usize)`. Use `ix * 100 + opt_ix` to encode pairs.

`Disableable` trait must be in scope for `Button::disabled()`.

### Entity patterns

- Create heterogeneous runtime form: `Vec` of per-field `Entity<InputState>` /
  `Entity<SelectState<SearchableVec<SharedString>>>` + app-held bools — **works**.
- Read-back on submit via `state.read(cx)` — no panic path for parse failures.
- `ent.update(cx, |form, cx| form.submit(cx))` **fails borrow check** when `submit` needs
  `cx` — use `ent.read(cx).submit(cx)` from button handlers instead.

## Probe harness

| Key | Probe |
|-----|-------|
| `←` / `→` / `[` / `]` | Cycle fixtures |
| `1` | Runtime dynamic form (crux) + seeds round-trip |
| `2` | Type coverage (FieldKind labels) |
| `3` | Constraints (required blocks, defaults, parse error inline) |
| `4` | Content JSON round-trip |
| `5` | AskUserQuestion flat map |
| `6` | Composition routing (6 fixtures → card kind) |

Fixtures are in `src/fixtures.rs` — all ⚠ derived-not-byte-verified.

## Initial crux read (pre-visual)

Probe `1` is designed to PASS when pressed with `generic_full`: 6 runtime fields built,
submit returns MCP-flat JSON matching expected map. Build compiles the full path; visual
confirmation is for the human driver.

## Open / PARTIAL items

- **NumberInput::init** not public — arrow-key increment may be absent (not probed).
- **Custom-row auto-select on type** (web parity): `on_custom_input_changed` stub exists
  but is not wired to `InputState` change events — probe 5 uses programmatic fill;
  interactive custom row needs typing + manual radio/checkbox click.
- **Live MCP form-mode capture** — not attempted; fixtures remain synthetic.
