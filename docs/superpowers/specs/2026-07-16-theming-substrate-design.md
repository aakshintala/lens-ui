# §18 Theming substrate — design

**Date:** 2026-07-16
**Branch:** `feat/lens-app-multi-session`
**Status:** Design (brainstorm decisions D1–D4 locked in
`docs/handoffs/2026-07-16-theming-brainstorm-decisions.md`; this doc specifies them for
`writing-plans`).
**Scope:** The *minimal* theming substrate only — the load-bearing token schema + one embedded dark
theme + the gpui-component bridge. Delivery machinery (importers, picker, registry, hot-reload,
external files, light *authoring*) is out of scope and deferred to §18-machinery (D4 step 9).

---

## 1. Problem & goal

Every colored surface in `lens-ui` currently bakes raw hex (`gpui::rgb(0x…)`) at the call site — the
card chrome alone has 12 (`crates/lens-ui/src/card/chrome.rs`), and the six wave colors there don't even
match the locked board palette. Before we build the wave card, the board, the transcript, the terminal,
etc., we need **one semantic token surface** so that:

1. Every call site reads a *named* token (`cx.lens_theme().status.working.fill`), never a hex literal.
2. Swapping the whole palette (dark→light, or a user import later) is a data change, not a code change.
3. The gpui-component widgets we already render on (buttons, inputs, scrollbars, markdown, tree-sitter
   syntax) pick up our base palette automatically — one source of truth, no per-widget theming.

The **only** load-bearing deliverable is the token *schema*: once call sites bake
`cx.lens_theme().status.working`, all later delivery machinery slots in behind the same accessor with
**zero call-site churn**. Everything else (files, registry, watcher, importers) is deferred precisely
because nothing depends on it yet and it themes surfaces that don't exist.

### Non-goals (explicitly deferred to §18-machinery)
External theme-file loading / a `themes/` dir · registry / multiple selectable themes · hot-reload
watcher · light-theme *authoring* (light must be *expressible*, not authored) · iTerm/Alacritty
importers · settings picker · `JsonSchema` derivation for user tooling.

---

## 2. Decisions carried in (from the brainstorm handoff)

- **D1 — Bridge, do not fork.** Own a `LensTheme` superset. Bridge into gpui-component by *writing our
  base tokens onto its public `Theme.colors`* at init. Rationale (airtight): gpui-component's `theme` is
  the crate **root** — 85 of its files `use crate::ActiveTheme` and read `cx.theme().field`. A
  crates.io-compiled component can never see an *extended* `ThemeColor`, so "extend their theme" means
  forking the entire 60-component crate forever — the whole-crate vendor `framework.md:218` rejected. The
  standing "vendor just the markdown module" decision works because markdown is a *leaf*; the theme is
  not. Their widgets never need to be `status.*`-aware (status drives our custom card, not their
  buttons), so the fork's only unique benefit is one we never use.
- **D2 — 4 token groups.** Base (maps 1:1 onto `ThemeColor`), status (ours), terminal (ours), diff
  (ours). Wave *behavior* (glow/pulse) is **not** a token — it stays code keyed by `Wave`.
- **D3 — Build base+status+dark now; design *room* for all 4 groups.** Terminal and diff shapes are
  specified here but not built until their consuming surface lands (D4 steps 5/7) — adding a struct field
  then is not a call-site change, so there is no churn cost to deferring.
- **D4 — Sequencing.** This substrate is step 1; it's the sole prerequisite for the wave build (step 2)
  which validates the schema immediately.

---

## 3. Architecture

New module `crates/lens-ui/src/theme/` (lens-ui is the right home: the theme needs `gpui::Hsla` + the
gpui-component bridge; `lens-core` is gpui-free domain types and must stay that way).

```
crates/lens-ui/src/theme/
  mod.rs            LensTheme, globals, cx.lens_theme() accessor, init(), the bridge fn
  tokens.rs         BaseTokens, StatusTokens, StatusColor (+ Wave→StatusColor), serde hex helper
  lens-dark-deep.json   embedded default theme (base + status), include_str!'d
```

### 3.1 Data model

`LensTheme` is a plain global holding decoded `Hsla` values (not hex strings — parse once at startup):

All token structs derive `serde::{Serialize, Deserialize}` (that is what `from_json` and a future
exporter use); `Hsla` fields carry `#[serde(with = "hex_hsla")]` (§4.1); `mode` needs no helper —
`ThemeMode` is natively `Deserialize` (snake_case → `"dark"`/`"light"`).

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LensTheme {
    pub name: SharedString,
    pub mode: ThemeMode,        // gpui_component::ThemeMode — Light | Dark
    pub base: BaseTokens,       // group 1 — bridged onto gpui-component Theme.colors
    pub status: StatusTokens,   // group 2 — ours (card, board, banners)
    // group 3 (terminal) + group 4 (diff): shapes specified in §5; fields added when their
    // consuming surface lands (D4 steps 5/7). Nothing references them today, so adding them
    // later is a struct change, not a call-site change — zero churn.
}
impl gpui::Global for LensTheme {}
```

`BaseTokens` is the **curated subset** of `ThemeColor` we own — field names mirror their `ThemeColor`
counterparts so the bridge is a trivial field-by-field copy. Everything else in `ThemeColor` rides
gpui-component's default.

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BaseTokens {
    // surfaces
    pub background: Hsla,
    pub foreground: Hsla,
    pub border: Hsla,
    pub muted: Hsla,
    pub muted_foreground: Hsla,
    pub popover: Hsla,
    pub popover_foreground: Hsla,
    pub accent: Hsla,
    pub accent_foreground: Hsla,
    // chrome
    pub sidebar: Hsla,
    pub sidebar_foreground: Hsla,
    pub sidebar_border: Hsla,
    pub title_bar: Hsla,
    pub title_bar_border: Hsla,
    pub tab: Hsla,
    pub tab_active: Hsla,
    pub tab_active_foreground: Hsla,
    pub tab_foreground: Hsla,
    // controls
    pub input: Hsla,
    pub ring: Hsla,
    pub selection: Hsla,
    pub scrollbar: Hsla,
    pub scrollbar_thumb: Hsla,
    pub list: Hsla,
    pub list_active: Hsla,
    pub list_hover: Hsla,
    pub progress_bar: Hsla,
    // generic component-state (gpui-component already has these; we author to match our palette)
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub info: Hsla,
    // overlay scrim (card disconnect overlay, dialogs)
    pub overlay: Hsla,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StatusColor { pub fill: Hsla, pub on_fill: Hsla }

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StatusTokens {
    pub ready: StatusColor,
    pub working: StatusColor,
    pub needs_input: StatusColor,
    pub failed: StatusColor,
    pub slept: StatusColor,
    pub neutral: StatusColor,
}

impl StatusTokens {
    /// Total map from Wave → its color. Exhaustive; adding a Wave variant is a compile error here.
    pub fn for_wave(&self, wave: Wave) -> StatusColor {
        match wave {
            Wave::Ready => self.ready,
            Wave::Working => self.working,
            Wave::NeedsInput => self.needs_input,
            Wave::Failed => self.failed,
            Wave::Slept => self.slept,
            Wave::Neutral => self.neutral,
        }
    }
}
```

`StatusColor` carries `on_fill` (the contrast text color for a filled pill/badge on that fill) — it
replaces the current ad-hoc `pill_text_color` (light on neutral/slept, dark on the bright waves).

### 3.2 Accessor

Extension trait mirroring gpui-component's own `ActiveTheme` pattern exactly, so call sites read the
same way (`cx.lens_theme()` alongside `cx.theme()`):

```rust
pub trait ActiveLensTheme { fn lens_theme(&self) -> &LensTheme; }
impl ActiveLensTheme for App {
    #[inline(always)]
    fn lens_theme(&self) -> &LensTheme { LensTheme::global(self) }
}
impl LensTheme {
    #[inline(always)]
    pub fn global(cx: &App) -> &LensTheme { cx.global::<LensTheme>() }
}
```

In render code `cx` is `&mut Context<Self>`, which derefs to `App`, so `cx.lens_theme()` resolves —
identical to how gpui-component's own components reach `cx.theme()`.

### 3.3 The bridge

```rust
/// Overwrite the base tokens we own onto gpui-component's global Theme, and align its mode so its
/// components render on our palette. Called once at init, after gpui_component::init.
fn bridge_into_gpui_component(lens: &LensTheme, cx: &mut App) {
    let theme = Theme::global_mut(cx);
    theme.mode = lens.mode;              // so components pick the right light/dark variants
    let c = &mut theme.colors;
    c.background = lens.base.background;
    c.foreground = lens.base.foreground;
    c.border = lens.base.border;
    c.muted = lens.base.muted;
    c.muted_foreground = lens.base.muted_foreground;
    c.popover = lens.base.popover;
    c.popover_foreground = lens.base.popover_foreground;
    c.accent = lens.base.accent;
    c.accent_foreground = lens.base.accent_foreground;
    c.sidebar = lens.base.sidebar;
    c.sidebar_foreground = lens.base.sidebar_foreground;
    c.sidebar_border = lens.base.sidebar_border;
    c.title_bar = lens.base.title_bar;
    c.title_bar_border = lens.base.title_bar_border;
    c.tab = lens.base.tab;
    c.tab_active = lens.base.tab_active;
    c.tab_active_foreground = lens.base.tab_active_foreground;
    c.tab_foreground = lens.base.tab_foreground;
    c.input = lens.base.input;
    c.ring = lens.base.ring;
    c.selection = lens.base.selection;
    c.scrollbar = lens.base.scrollbar;
    c.scrollbar_thumb = lens.base.scrollbar_thumb;
    c.list = lens.base.list;
    c.list_active = lens.base.list_active;
    c.list_hover = lens.base.list_hover;
    c.progress_bar = lens.base.progress_bar;
    c.success = lens.base.success;
    c.warning = lens.base.warning;
    c.danger = lens.base.danger;
    c.info = lens.base.info;
    c.overlay = lens.base.overlay;
}
```

We do **not** call gpui-component's `apply_config` (it's `pub(crate)` — unreachable). We don't need it:
their `init` already populated a complete default `Theme`; we just override the ~30 base fields we own on
top. Fields we don't touch (tables, sliders, tiles, red/green/blue/magenta/cyan/yellow, `bullish`/
`bearish`, `progress` internals, `highlight_theme`) keep gpui-component's sensible defaults. HighlightTheme
(tree-sitter syntax) rides their default for now; authoring it is deferred to the transcript surface
(D4 step 5).

### 3.4 init

```rust
/// Parse the embedded dark theme, install it as the LensTheme global, and bridge its base tokens
/// onto gpui-component's Theme. Call once, immediately after gpui_component::init(cx).
pub fn init(cx: &mut App) {
    const DARK: &str = include_str!("lens-dark-deep.json");
    let lens = LensTheme::from_json(DARK)
        .expect("embedded lens-dark-deep.json must parse — this is a build-time invariant");
    bridge_into_gpui_component(&lens, cx);
    cx.set_global(lens);
}
```

The embedded JSON is a compiled-in invariant; a parse failure is a developer error caught by the parse
unit test (§6) long before runtime, so `expect` at startup is correct (not a user-facing failure mode).

`main.rs` calls `lens_ui::theme::init(cx)` on the line after each `gpui_component::init(cx)` — **two
sites** (live run + `--demo`). Both must be updated (the demo is how we eyeball the wave palette).

---

## 4. Theme file format

`lens-dark-deep.json` — hex strings (the format importers and a future light theme reuse). Forward/
backward compatible by construction: the parser uses `#[serde(default)]` on optional groups and does
**not** `deny_unknown_fields`, so (a) today's file omitting terminal/diff parses against a future binary,
and (b) an early-authored terminal block parses against today's binary (ignored). `base` and `status`
are required.

```json
{
  "name": "Lens Dark Deep",
  "mode": "dark",
  "base": {
    "background": "#07080b",
    "foreground": "#eef2f7",
    "border": "#222936",
    "muted": "#151922",
    "muted_foreground": "#9aa4b3",
    "popover": "#1c2230",
    "popover_foreground": "#eef2f7",
    "accent": "#4c8dff",
    "accent_foreground": "#0b1220",
    "sidebar": "#07080b",
    "sidebar_foreground": "#9aa4b3",
    "sidebar_border": "#222936",
    "title_bar": "#07080b",
    "title_bar_border": "#222936",
    "tab": "#101319",
    "tab_active": "#1c2230",
    "tab_active_foreground": "#eef2f7",
    "tab_foreground": "#9aa4b3",
    "input": "#151922",
    "ring": "#4c8dff",
    "selection": "#4c8dff",
    "scrollbar": "#101319",
    "scrollbar_thumb": "#2c3442",
    "list": "#101319",
    "list_active": "#1c2230",
    "list_hover": "#151922",
    "progress_bar": "#4c8dff",
    "success": "#36c98a",
    "warning": "#ff8a3d",
    "danger": "#ff5d5d",
    "info": "#4c8dff",
    "overlay": "#000000"
  },
  "status": {
    "ready":       { "fill": "#4c8dff", "on_fill": "#0b1220" },
    "working":     { "fill": "#36c98a", "on_fill": "#0b1220" },
    "needs_input": { "fill": "#ff8a3d", "on_fill": "#0b1220" },
    "failed":      { "fill": "#ff5d5d", "on_fill": "#0b1220" },
    "slept":       { "fill": "#7a8493", "on_fill": "#e5e7eb" },
    "neutral":     { "fill": "#374151", "on_fill": "#e5e7eb" }
  }
}
```

Base hexes are lifted from the locked `board-home.html :root`
(`--bg #07080b`, `--bg1 #101319`, `--bg2 #151922`, `--bg3 #1c2230`, `--line #222936`, `--line2 #2c3442`,
`--tx #eef2f7`, `--tx2 #9aa4b3`, `--tx3 #5f6a7a`). Status fills are the D2-locked wave colors. `on_fill`
follows the current contrast rule (dark ink on bright fills, light ink on neutral/slept).

### 4.1 Deserialization

Fields are typed `Hsla` for clean call sites; a serde `with`-module converts hex↔`Hsla` at the field
level, reusing gpui-component's `Colorize::parse_hex`/`to_hex`:

```rust
mod hex_hsla {
    use gpui::Hsla;
    use gpui_component::Colorize;      // parse_hex + to_hex live here
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Hsla, D::Error> {
        let s = String::deserialize(d)?;
        Hsla::parse_hex(&s).map_err(|e| D::Error::custom(format!("bad hex {s:?}: {e}")))
    }
    pub fn serialize<S: Serializer>(c: &Hsla, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&c.to_hex())
    }
}
```

Each `Hsla` field carries `#[serde(with = "hex_hsla")]`. `mode` deserializes from `"dark"`/`"light"`
into `gpui_component::ThemeMode`. `LensTheme::from_json(&str) -> Result<LensTheme>` is
`serde_json::from_str`. (Deriving `Serialize` too is nearly free and lets a future exporter/importer
round-trip; it costs nothing now.)

---

## 5. Deferred group *shapes* (specified, not built)

Built when their consuming surface lands. Recorded here so the file format and importers have a target
and so adding them later is mechanical. **Provisional** — values are placeholders until authored against
the real surface (per `premature-layer-boundary-binding`: specify the shape, don't lock the values).

### 5.1 Terminal (group 3) — D4 step 7, with the terminal renderer
Feeds the libghostty_vt + ghostty_rs custom gpui renderer (in progress at `../lens-terminal-ws`; no
palette yet). ~20 tokens; target of §18-machinery's iTerm/Alacritty importer.

```rust
pub struct TerminalTokens {
    pub foreground: Hsla, pub background: Hsla, pub cursor: Hsla, pub selection: Hsla,
    pub normal:  AnsiSet,   // black,red,green,yellow,blue,magenta,cyan,white
    pub bright:  AnsiSet,
}
pub struct AnsiSet { pub black: Hsla, pub red: Hsla, pub green: Hsla, pub yellow: Hsla,
                     pub blue: Hsla, pub magenta: Hsla, pub cyan: Hsla, pub white: Hsla }
```
JSON key: `"terminal": { "foreground": …, "normal": { "black": …, … }, "bright": { … } }`.

### 5.2 Diff (group 4) — D4 step 5, with the transcript surface
gpui-component has `bullish`/`bearish` + red/green but no diff-semantic bg pairs. ~6 tokens.

```rust
pub struct DiffTokens {
    pub added_bg: Hsla, pub added_fg: Hsla,
    pub removed_bg: Hsla, pub removed_fg: Hsla,
    pub context_fg: Hsla,
    pub hunk_header: Hsla,
}
```
JSON key: `"diff": { "added_bg": …, … }`.

When built, each gets `#[serde(default)]` on `LensTheme` so files that omit it still parse.

---

## 6. Testing

Pure, no gpui window needed for the core:

1. **Embedded parse** — `LensTheme::from_json(include_str!("lens-dark-deep.json"))` is `Ok` and
   `name == "Lens Dark Deep"`, `mode == Dark`. (This is what makes the `expect` in `init` a build-time
   invariant.)
2. **Locked-palette guard** — assert the six `status.*.fill` values equal the D2-locked hexes
   (`ready #4c8dff`, `working #36c98a`, `needs_input #ff8a3d`, `failed #ff5d5d`, `slept #7a8493`,
   `neutral #374151`). Guards against silent drift from the `board-home.html` SSOT.
3. **`for_wave` totality** — all six `Wave` variants resolve; adding a variant fails to compile.
4. **hex round-trip** — `parse_hex → to_hex → parse_hex` is stable for a sample token.
5. **Bridge smoke** (gpui `test_app` if cheap; else skip) — after `theme::init`, `cx.theme().background`
   equals `cx.lens_theme().base.background`, confirming the bridge wrote through.

---

## 7. A2 — hex→token call-site migration (companion, runs right after the substrate)

Not part of the substrate build but the immediate next step it unblocks (D4). `chrome.rs` today has 12
hardcoded hexes; A2 both **tokenizes** them and **corrects** the wave colors (current code uses a
different, wrong palette). Migration map:

| current `chrome.rs`                         | becomes                                        |
|---------------------------------------------|------------------------------------------------|
| `wave_border_color` (6 rgb, wrong palette)  | `cx.lens_theme().status.for_wave(wave).fill`   |
| `pill_text_color` (0xe5e7eb / 0x0b1220)     | `…status.for_wave(wave).on_fill`               |
| kebab menu bg `0x1f2937`                     | `cx.lens_theme().base.popover`                 |
| muted text `0x9ca3af` (×3)                   | `cx.lens_theme().base.muted_foreground`        |
| overlay text `0xf3f4f6`                      | `cx.lens_theme().base.foreground`              |
| overlay scrim `hsla(0,0,0,0.55)`             | `cx.lens_theme().base.overlay.opacity(0.55)`   |

`wave_border_color`/`pill_text_color` change signature to take `&App` (or the resolved `LensTheme`)
since they now read the global. Existing chrome unit tests are color-agnostic (they assert repo-row text
formatting) and are unaffected.

---

## 8. Files touched

- **New:** `crates/lens-ui/src/theme/mod.rs`, `crates/lens-ui/src/theme/tokens.rs`,
  `crates/lens-ui/src/theme/lens-dark-deep.json`.
- **Edit:** `crates/lens-ui/src/lib.rs` (`pub mod theme;` + re-export `ActiveLensTheme`, `LensTheme`).
- **Edit:** `crates/lens-app/src/main.rs` (call `lens_ui::theme::init(cx)` after both
  `gpui_component::init(cx)` sites).
- **A2 (companion):** `crates/lens-ui/src/card/chrome.rs`.

No new dependencies — `gpui-component` (`Colorize`, `Theme`, `ThemeMode`), `serde`, `serde_json` are
already in `lens-ui`.

---

## 9. Verification (definition of done for the substrate)

- `cargo test -p lens-ui` green (the §6 tests).
- `cargo run -p lens-app -- --demo` shows the six cards in the **locked** wave colors (after A2), proving
  the schema drives the real surface.
- `xtask gate` clean (no warnings / dead code).
- Cross-family review of the diff (per project rules: ≥1 review from a non-author model family).
```