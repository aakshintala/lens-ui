# §18 Theming substrate — design

**Date:** 2026-07-16
**Branch:** `feat/lens-app-multi-session`
**Status:** Design (brainstorm decisions D1–D4 locked in
`docs/handoffs/2026-07-16-theming-brainstorm-decisions.md`; this doc specifies them for
`writing-plans`).
**Scope:** The theming substrate — the load-bearing token schema + **two** themes (dark + light,
base+status each) + the gpui-component bridge + startup selection + **external-file loading and a manual
reload command** (the tuning loop). The heavier control surface — settings pane, live OS-appearance
toggle, auto file-watcher, theme registry/picker, importers — is the **next** workstream (§18-machinery),
not this one.

> **Sequencing (decided with user, amends the brainstorm handoff D4):**
> 1. **This workstream** — schema + dark + light + bridge + external files + manual reload.
> 2. **Wave build (B1–B5) then board (B6–B8)** — the wedge; tunes its colors via this workstream's manual
>    reload.
> 3. **§18-machinery + settings pane** — live OS toggle, auto-watcher, registry, picker, and the settings
>    pane that houses them. Lands *before* transcript/composer/side-pane/editor so settings has a home
>    before those surfaces need to dock into it. (Ordering (b): wedge first, settings pane right after —
>    the board's toggles then have somewhere to live and the pane isn't empty scaffolding.)
> 4. Importers stay paired with their **terminal** surface (D4 step 7) — they theme tokens for a renderer
>    that doesn't exist yet, so they're not part of §18-machinery.
>
> Light-theme *authoring* is pulled forward from D4-step-4 into step 1: authoring light now is the
> forcing function that proves the schema is genuinely semantic (no dark-baked field values) *before*
> surfaces multiply.

---

## 1. Problem & goal

Every colored surface in `lens-ui` currently bakes raw hex (`gpui::rgb(0x…)`) at the call site — the
card chrome alone has 12 (`crates/lens-ui/src/card/chrome.rs`), and the six wave colors there don't even
match the locked board palette. Before we build the wave card, the board, the transcript, the terminal,
etc., we need **one semantic token surface** so that:

1. Every call site reads a *named* token (`cx.lens_theme().status.for_wave(w)`), never a hex literal.
2. Swapping the whole palette (dark↔light, or a user import later) is a data change, not a code change.
3. The gpui-component widgets we already render on (buttons, inputs, scrollbars, markdown, tree-sitter
   syntax) pick up our base palette automatically — one source of truth, no per-widget theming.
4. Tuning a color is *edit JSON → reload → see it*, not edit → recompile → relaunch.

The **only** load-bearing deliverable is the token *schema*: once call sites bake
`cx.lens_theme().status…`, all later delivery machinery (registry, picker, watcher, importers) slots in
behind the same accessor with **zero call-site churn**. The reload loop is in scope here not because
anything depends on it, but because we're about to tune every color against real pixels and restart-to-see
is slow.

### Non-goals (deferred to the §18-machinery + settings-pane workstream)

* config-dir convention / a `themes/` registry / **more than the two built-in themes** / a settings picker
* **auto** file-watcher (this workstream ships *manual* reload only)
* **live OS-appearance toggle** (re-bridging when the OS flips while running)
* iTerm/Alacritty importers (stay with the terminal surface, D4 step 7)
* `JsonSchema` derivation for user tooling

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
- **D2 — 4 token groups.** Base (maps onto `ThemeColor`), status (ours), terminal (ours), diff (ours).
  Wave *behavior* — glow, radial tint, pulse, and the derived tile/progress tints — is **not** a token;
  it stays code keyed by `Wave`, computed from the one status color via `Colorize::opacity/mix`.
- **D3 — Build base+status now; design *room* for all 4 groups.** Terminal and diff shapes are specified
  here but not built until their consuming surface lands (D4 steps 5/7) — adding a struct field then is
  not a call-site change, so there is no churn cost to deferring.
- **D4 — Sequencing.** This substrate is step 1; it's the sole prerequisite for the wave build (step 2)
  which validates the schema immediately. (See the amended sequencing note above for the settings-pane
  slot.)

---

## 3. Architecture

New module `crates/lens-ui/src/theme/` (lens-ui is the right home: the theme needs `gpui::Hsla` + the
gpui-component bridge; `lens-core` is gpui-free domain types and must stay that way).

```
crates/lens-ui/src/theme/
  mod.rs         LensTheme, globals, cx.lens_theme() accessor, init()/reload()/resolve(), the bridge fn
  tokens.rs      BaseTokens, StatusTokens (+ Wave→Hsla), serde hex helper
  lens-dark.json    "Lens Dark"  (base + status) — embedded default AND the on-disk reload target
  lens-light.json   "Lens Light" (base + status) — embedded default AND the on-disk reload target
```

### 3.1 Data model

`LensTheme` is a plain global holding decoded `Hsla` values (not hex strings — parse once at startup).
All token structs derive `serde::{Serialize, Deserialize}` (that is what `from_json` and a future
exporter use); `Hsla` fields carry `#[serde(with = "hex_hsla")]` (§4.3); `mode` needs no helper —
`ThemeMode` is natively `Deserialize` (snake_case → `"dark"`/`"light"`).

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LensTheme {
    pub name: SharedString,
    pub mode: ThemeMode,        // gpui_component::ThemeMode — Light | Dark
    pub base: BaseTokens,       // group 1 — bridged onto gpui-component Theme.colors
    pub status: StatusTokens,   // group 2 — ours (card tile/border/glow/stat/progress, banners)
    // group 3 (terminal) + group 4 (diff): shapes specified in §5; fields added when their
    // consuming surface lands (D4 steps 5/7). Nothing references them today, so adding them
    // later is a struct change, not a call-site change — zero churn.
}
impl gpui::Global for LensTheme {}
```

`BaseTokens` is the **curated subset** of `ThemeColor` we own — field names mirror their `ThemeColor`
counterparts so the bridge is a trivial field-by-field copy. Everything else in `ThemeColor` rides
gpui-component's default. This set is a **starting cut** (per user: "start here, add/remove as we build
more surfaces") — expect it to grow/shrink as real surfaces land; that churn is data + one bridge line
per field, never call-site churn.

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

/// One saturated color per wave state. Every card consumer uses it directly (border, glow, tile icon,
/// STATUS label, progress-bar fill, branch text) or a *derived tint* computed in code via
/// `Colorize::opacity/mix` — the 12%/14%/30% mixes in the locked render are code, not tokens (D2).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StatusTokens {
    pub ready: Hsla,
    pub working: Hsla,
    pub needs_input: Hsla,
    pub failed: Hsla,
    pub slept: Hsla,
    pub neutral: Hsla,
}

impl StatusTokens {
    /// Total map from Wave → its color. Exhaustive; adding a Wave variant is a compile error here.
    pub fn for_wave(&self, wave: Wave) -> Hsla {
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

**No `on_fill`/contrast token.** The locked card is the 44px **icon-tile** + card-level wave overlay
(handoff decision 2 + `board-home.html`), not a filled text pill — so no surface paints text on a
saturated fill. The throwaway pill in today's `chrome.rs` (and its `pill_text_color`) is deleted by the
wave build (B1), not migrated. If a future *filled banner* ever needs contrast ink, that is a new token
added then, with zero churn to the existing border/tile/stat consumers.

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
/// components render on our palette. Called at init and on every reload.
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
their `init` already populated a complete default `Theme` (light + dark defaults); we just override the
~30 base fields we own on top. Fields we don't touch (tables, sliders, tiles,
red/green/blue/magenta/cyan/yellow, `bullish`/`bearish`, `highlight_theme`) keep gpui-component's
defaults. HighlightTheme (tree-sitter syntax) rides their default for now; authoring it is deferred to
the transcript surface (D4 step 5).

### 3.4 Selection, init, and reload

Themes ship **embedded** (`include_str!`) as the always-works default. If `LENS_THEME_DIR` is set, the
selected theme loads from `<dir>/lens-{mode}.json` on disk instead — and that same on-disk file is what a
**reload** re-reads. For our dev loop, point `LENS_THEME_DIR` at `crates/lens-ui/src/theme/`, so the file
we edit *is* both the compiled-in default and the live reload target (no copy-back).

```rust
/// Pure, testable core: resolve which theme to use and load it. `dir` = the LENS_THEME_DIR override
/// (None → embedded). A bad *external* file falls back to embedded and logs — a stray user edit must
/// not crash a running app. The embedded default is a build-time invariant (panics only if IT is bad,
/// which the §6 parse test prevents).
fn resolve(mode: ThemeMode, dir: Option<&Path>) -> LensTheme {
    const DARK: &str = include_str!("lens-dark.json");
    const LIGHT: &str = include_str!("lens-light.json");
    let (embedded, file) = if mode.is_dark() { (DARK, "lens-dark.json") }
                           else              { (LIGHT, "lens-light.json") };
    if let Some(dir) = dir {
        let path = dir.join(file);
        match std::fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|s| LensTheme::from_json(&s))
        {
            Ok(lens) => return lens,
            Err(e) => eprintln!("lens-theme: {} — using embedded default: {e}", path.display()),
        }
    }
    LensTheme::from_json(embedded).expect("embedded lens theme must parse — build-time invariant")
}

/// Resolve mode (LENS_THEME override else the OS appearance gpui_component::init already synced) and
/// the LENS_THEME_DIR override, then load.
fn load_selected(cx: &App) -> LensTheme {
    let mode = match std::env::var("LENS_THEME").ok().as_deref() {
        Some("light") => ThemeMode::Light,
        Some("dark") => ThemeMode::Dark,
        _ => Theme::global(cx).mode,
    };
    let dir = std::env::var("LENS_THEME_DIR").ok();
    resolve(mode, dir.as_deref().map(Path::new))
}

/// Install the selected theme + bridge. Call once, immediately after gpui_component::init(cx).
pub fn init(cx: &mut App) {
    let lens = load_selected(cx);
    bridge_into_gpui_component(&lens, cx);
    cx.set_global(lens);
}

/// Re-read the selected theme from disk (or embedded) and re-apply it live. Bound to a keybinding.
pub fn reload(window: &mut Window, cx: &mut App) {
    let lens = load_selected(cx);
    bridge_into_gpui_component(&lens, cx);
    cx.set_global(lens);
    window.refresh();   // repaint all views on the new palette — gpui-component's Theme::change pattern
}
```

`main.rs` calls `lens_ui::theme::init(cx)` after each `gpui_component::init(cx)` — **two sites** (live run
+ `--demo`) — and binds a `ReloadTheme` action to a key (e.g. `cmd-shift-t`). The action is handled where
a `Window` is in scope (the root view, exactly like the existing `cmd-.`/`BackToBoard`), calling
`theme::reload(window, cx)`. Reload is **manual only**; an auto file-watcher is the next workstream.

Selection is otherwise **at startup**: if the OS flips dark↔light while running we do not auto re-bridge
(that's the deferred live toggle) — reload, relaunch, or set `LENS_THEME`.

---

## 4. Theme file format

Hex strings (the format importers and future themes reuse). Forward/backward compatible by construction:
the parser does **not** `deny_unknown_fields`, and deferred groups will be `#[serde(default)]` when added,
so (a) today's file omitting terminal/diff parses against a future binary, and (b) an early-authored
terminal block parses against today's binary (ignored). `base` and `status` are required.

**All status values below are placeholders pending on-device eyeballing** (per user). Dark is *seeded*
from the locked `board-home.html` render; light is a first cut. Both will be retuned against real
rendered surfaces during the wave/board build using the reload loop — treat the numbers as starting
points, not final.

### 4.1 `lens-dark.json` — "Lens Dark"

```json
{
  "name": "Lens Dark",
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
    "ready": "#4c8dff",
    "working": "#36c98a",
    "needs_input": "#ff8a3d",
    "failed": "#ff5d5d",
    "slept": "#7a8493",
    "neutral": "#374151"
  }
}
```

Dark base hexes are lifted from the locked `board-home.html :root`
(`--bg #07080b`, `--bg1 #101319`, `--bg2 #151922`, `--bg3 #1c2230`, `--line #222936`, `--line2 #2c3442`,
`--tx #eef2f7`, `--tx2 #9aa4b3`, `--tx3 #5f6a7a`). Status colors are seeded from the D2 wave palette.

### 4.2 `lens-light.json` — "Lens Light" (first-cut)

No locked light SSOT exists, so this is authored for *structural* correctness — light surfaces, dark
text, hue-matched-but-legible accents/status on a light background. The checkpoint is "the schema
expresses light with no dark-baked assumptions," not "final light aesthetics."

```json
{
  "name": "Lens Light",
  "mode": "light",
  "base": {
    "background": "#ffffff",
    "foreground": "#1c2230",
    "border": "#d6dbe4",
    "muted": "#f2f4f8",
    "muted_foreground": "#5f6a7a",
    "popover": "#ffffff",
    "popover_foreground": "#1c2230",
    "accent": "#2f6bd8",
    "accent_foreground": "#ffffff",
    "sidebar": "#f7f8fb",
    "sidebar_foreground": "#5f6a7a",
    "sidebar_border": "#d6dbe4",
    "title_bar": "#f7f8fb",
    "title_bar_border": "#d6dbe4",
    "tab": "#eef1f6",
    "tab_active": "#ffffff",
    "tab_active_foreground": "#1c2230",
    "tab_foreground": "#5f6a7a",
    "input": "#ffffff",
    "ring": "#2f6bd8",
    "selection": "#bcd3ff",
    "scrollbar": "#eef1f6",
    "scrollbar_thumb": "#c2c9d4",
    "list": "#ffffff",
    "list_active": "#eef1f6",
    "list_hover": "#f2f4f8",
    "progress_bar": "#2f6bd8",
    "success": "#1f9d6b",
    "warning": "#d9701f",
    "danger": "#d43d3d",
    "info": "#2f6bd8",
    "overlay": "#0b1220"
  },
  "status": {
    "ready": "#2f6bd8",
    "working": "#1f9d6b",
    "needs_input": "#d9701f",
    "failed": "#d43d3d",
    "slept": "#8a93a3",
    "neutral": "#c2c9d4"
  }
}
```

### 4.3 Deserialization

Fields are typed `Hsla` for clean call sites; a serde `with`-module converts hex↔`Hsla` at the field
level, reusing gpui-component's `Colorize::parse_hex`/`to_hex`:

```rust
mod hex_hsla {
    use gpui::Hsla;
    use gpui_component::Colorize;      // parse_hex + to_hex; reachable via crate-root glob re-export
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

Each `Hsla` field carries `#[serde(with = "hex_hsla")]`. `mode` deserializes directly into
`gpui_component::ThemeMode` (natively `Deserialize`, snake_case). `LensTheme::from_json(&str) ->
anyhow::Result<LensTheme>` wraps `serde_json::from_str`.

---

## 5. Deferred group *shapes* (specified, not built)

Built when their consuming surface lands. Recorded here so the file format and importers have a target
and so adding them later is mechanical. **Provisional** — values are placeholders until authored against
the real surface (per `premature-layer-boundary-binding`: specify the shape, don't lock the values).

### 5.1 Terminal (group 3) — D4 step 7, with the terminal renderer
Feeds the libghostty_vt + ghostty_rs custom gpui renderer (in progress at `../lens-terminal-ws`; no
palette yet). ~20 tokens; target of the iTerm/Alacritty importer.

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

Pure where possible (`resolve` takes an explicit `mode`/`dir` so tests never touch process env):

1. **Both embedded themes parse** — `from_json(include_str!("lens-dark.json"))` and `…lens-light.json`
   are both `Ok`; names/modes are `("Lens Dark", Dark)` / `("Lens Light", Light)`. Makes the `expect` in
   `resolve` a build-time invariant.
2. **Dark-status seed guard** — the six dark `status.*` values equal the `board-home.html` seed hexes.
   Catches accidental drift; when we *intentionally* retune, update the render + this test together.
3. **Light expresses distinctly** — light `base.background` ≠ dark `base.background` and light
   `base.foreground` is darker than its background (a cheap "not dark-baked" sanity check).
4. **`for_wave` totality** — all six `Wave` variants resolve; adding a variant fails to compile.
5. **hex round-trip** — `parse_hex → to_hex → parse_hex` is stable for a sample token.
6. **External override precedence** — `resolve(Dark, Some(tmpdir))` with a modified `lens-dark.json` in
   `tmpdir` returns the on-disk values, not embedded.
7. **Bad external file falls back** — `resolve(Dark, Some(tmpdir))` with a malformed `lens-dark.json`
   returns the embedded default (no panic).
8. **Bridge smoke** (gpui `test_app` if cheap; else skip) — after `theme::init`, `cx.theme().background`
   equals `cx.lens_theme().base.background`, confirming the bridge wrote through.

---

## 7. A2 — hex→token call-site migration (companion, runs right after the substrate)

Not part of the substrate build but the immediate next step it unblocks (D4). `chrome.rs` today has 12
hardcoded hexes. The **pill and its `pill_text_color`** are throwaway — the wave build (B1) replaces the
pill with the icon-tile, so A2 does **not** migrate them; it re-homes the surviving call sites and fixes
the border color (current code uses a different, wrong palette):

| current `chrome.rs`                          | becomes                                        |
|----------------------------------------------|------------------------------------------------|
| `wave_border_color` (6 rgb, wrong palette)   | `cx.lens_theme().status.for_wave(wave)`        |
| kebab menu bg `0x1f2937`                      | `cx.lens_theme().base.popover`                 |
| muted text `0x9ca3af` (×3)                    | `cx.lens_theme().base.muted_foreground`        |
| overlay text `0xf3f4f6`                       | `cx.lens_theme().base.foreground`              |
| overlay scrim `hsla(0,0,0,0.55)`              | `cx.lens_theme().base.overlay.opacity(0.55)`   |
| pill fill + `pill_text_color`                 | *(not migrated — deleted by B1's icon-tile)*   |

`wave_border_color` changes signature to take `&App` (or the resolved `LensTheme`) since it now reads the
global. Existing chrome unit tests are color-agnostic (they assert repo-row text formatting) and are
unaffected.

---

## 8. Files touched

- **New:** `crates/lens-ui/src/theme/mod.rs`, `crates/lens-ui/src/theme/tokens.rs`,
  `crates/lens-ui/src/theme/lens-dark.json`, `crates/lens-ui/src/theme/lens-light.json`.
- **Edit:** `crates/lens-ui/src/lib.rs` (`pub mod theme;` + re-export `ActiveLensTheme`, `LensTheme`; a
  `ReloadTheme` action + its handler in the root view, alongside `BackToBoard`).
- **Edit:** `crates/lens-app/src/main.rs` (call `lens_ui::theme::init(cx)` after both
  `gpui_component::init(cx)` sites; bind the `ReloadTheme` key next to `cmd-.`).
- **A2 (companion):** `crates/lens-ui/src/card/chrome.rs`.

No new dependencies — `gpui-component` (`Colorize`, `Theme`, `ThemeMode`), `serde`, `serde_json`,
`anyhow` are already in `lens-ui`; external loading uses only `std::fs`/`std::env`.

---

## 9. Verification (definition of done for the substrate)

- `cargo test -p lens-ui` green (the §6 tests).
- `cargo run -p lens-app -- --demo` shows the six cards in the seeded wave colors (after A2); running with
  `LENS_THEME=light` shows the light palette on the same surfaces — proving the schema drives both themes.
- **Reload loop:** with `LENS_THEME_DIR=crates/lens-ui/src/theme`, edit a color in `lens-dark.json`, press
  the reload key, and the change appears **without restart**.
- `xtask gate` clean (no warnings / dead code).
- Cross-family review of the diff (per project rules: ≥1 review from a non-author model family).
```