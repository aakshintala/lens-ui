# §18 Theming substrate — design

**Date:** 2026-07-16
**Branch:** `feat/lens-app-multi-session`
**Status:** Design (brainstorm decisions D1–D4 locked in
`docs/handoffs/2026-07-16-theming-brainstorm-decisions.md`; cross-family reviewed by grok-4.5 +
gpt-5.6-sol, findings folded in). Feeds `writing-plans`.
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
>    before those surfaces need to dock into it. (Ordering (b): wedge first, settings pane right after.)
> 4. Importers stay paired with their **terminal** surface (D4 step 7).
>
> Light-theme *authoring* is pulled forward from D4-step-4 into step 1 (forcing function proving the
> schema is genuinely semantic — no dark-baked field values — before surfaces multiply).

---

## 1. Problem & goal

Every colored surface in `lens-ui` currently bakes raw hex (`gpui::rgb(0x…)`) at the call site — the
card chrome alone has 12 (`crates/lens-ui/src/card/chrome.rs`), and the six wave colors there don't even
match the locked board palette. Before we build the wave card, the board, the transcript, the terminal,
etc., we need **one semantic token surface** so that:

1. Every call site reads a *named* token (`w.status_color(cx.lens_theme())`), never a hex literal.
2. Swapping the whole palette (dark↔light, or a user import later) is a data change, not a code change.
3. The gpui-component widgets we already render on (buttons, inputs, scrollbars, markdown, tree-sitter
   syntax) pick up our base palette automatically — one source of truth, no per-widget theming.
4. Tuning a color is *edit JSON → reload → see it*, not edit → recompile → relaunch.

The **only** load-bearing deliverable is the token *schema*: once call sites read
`cx.lens_theme()`, all later delivery machinery (registry, picker, watcher, importers) slots in behind
the same accessor with **zero call-site churn**. The reload loop is in scope here not because anything
depends on it, but because we're about to tune every color against real pixels and restart-to-see is slow.

### Non-goals (deferred to the §18-machinery + settings-pane workstream)

* config-dir convention / a `themes/` registry / **more than the two built-in themes** / a settings picker
* **auto** file-watcher (this workstream ships *manual* reload only)
* **live OS-appearance toggle** (re-bridging when the OS flips while running)
* iTerm/Alacritty importers (stay with the terminal surface, D4 step 7)
* `JsonSchema` derivation for user tooling

---

## 2. Decisions carried in (from the brainstorm handoff)

- **D1 — Bridge, do not fork.** Own a `LensTheme` superset. Bridge into gpui-component **through its
  public extension point** `Theme::apply_config` (schema.rs:645) — build a `ThemeConfig` from our base
  tokens and apply it. Rationale (airtight): gpui-component's `theme` is the crate **root** — 85 of its
  files `use crate::ActiveTheme` and read `cx.theme().field`. A crates.io-compiled component can never
  see an *extended* `ThemeColor`, so "extend their theme" means forking the entire 60-component crate
  forever — the whole-crate vendor `framework.md:218` rejected. Their widgets never need to be
  `status.*`-aware (status drives our custom card, not their buttons). Using `apply_config` (rather than
  poking `Theme.colors` fields directly — the approach an earlier draft took) is *more* in the spirit of
  D1: it uses their public API, derives the interaction families from our base per **their** rules
  (so we don't duplicate formulas that could drift on upgrade), and stores our config as the mode's
  `dark_theme`/`light_theme` so a later `Theme::change` re-applies **ours** instead of wiping it (§3.3).
- **D2 — 4 token groups.** Base (seeds gpui-component's `ThemeConfig`), status (ours), terminal (ours),
  diff (ours). Wave *behavior* — glow, radial tint, pulse, and the derived tile/progress tints — is
  **not** a token; it stays code keyed by `Wave`, computed from the one status color via
  `Colorize::opacity/mix`.
- **D3 — Build base+status now; design *room* for all 4 groups.** Terminal and diff shapes are specified
  here but not built until their consuming surface lands (D4 steps 5/7) — adding a struct field then is
  not a call-site change, so there is no churn cost to deferring.
- **D4 — Sequencing.** This substrate is step 1; sole prerequisite for the wave build (step 2). (See the
  amended sequencing note above for the settings-pane slot.)

---

## 3. Architecture

New module `crates/lens-ui/src/theme/` (lens-ui is the right home: the theme needs `gpui::Hsla` + the
gpui-component bridge; `lens-core` is gpui-free domain types and must stay that way).

```
crates/lens-ui/src/theme/
  mod.rs         LensTheme, globals, cx.lens_theme() accessor, load/apply/reload, the ThemeConfig adapter
  tokens.rs      BaseTokens, StatusTokens, serde hex helper
  lens-dark.json    "Lens Dark"  (base + status) — embedded default AND the on-disk reload target
  lens-light.json   "Lens Light" (base + status) — embedded default AND the on-disk reload target
```

**Layering:** `theme` is a **leaf** — it does *not* depend on `card::Wave`. `StatusTokens` exposes six
named fields; the `Wave → status color` map lives in `card/wave.rs` (card depends on theme, not the
reverse) as `impl Wave { fn status_color(self, t: &LensTheme) -> Hsla }` (§7).

### 3.1 Data model

`LensTheme` is a plain global holding decoded `Hsla` values (parse once). All token structs derive
`serde::{Serialize, Deserialize}`; `Hsla` fields carry `#[serde(with = "hex_hsla")]` (§4.3); `mode` needs
no helper — `ThemeMode` is natively `Deserialize` (snake_case → `"dark"`/`"light"`).

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LensTheme {
    pub name: SharedString,
    pub mode: ThemeMode,        // gpui_component::ThemeMode — Light | Dark
    pub base: BaseTokens,       // group 1 — seeds gpui-component's Theme via the ThemeConfig adapter
    pub status: StatusTokens,   // group 2 — ours (card tile/border/glow/stat/progress, banners)
    // group 3 (terminal) + group 4 (diff): shapes specified in §5; fields added when their consuming
    // surface lands (D4 steps 5/7). Adding them later is a struct change, not a call-site change.
}
impl gpui::Global for LensTheme {}
```

`BaseTokens` is the **curated subset** we author. Its fields map onto gpui-component `ThemeConfigColors`
fields in the adapter (§3.3); the interaction families (`primary_hover`, `*_active`, `*_foreground`, …)
are **not** authored — `apply_config` derives them from these. This set is a **starting cut** (per user:
"start here, add/remove as we build") — expect it to grow/shrink; that churn is data + one adapter line
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
    pub accent: Hsla,           // our brand/action color → mapped to gpui-component `primary` (§3.3)
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
    pub input: Hsla,            // NOTE: gpui-component `input` is the input *border* color — author from
                                //       a line color (--line), NOT a fill. (grok review.)
    pub caret: Hsla,            // input text cursor (gpui-component `caret`)
    pub ring: Hsla,
    pub selection: Hsla,
    pub scrollbar: Hsla,
    pub scrollbar_thumb: Hsla,
    pub list: Hsla,
    pub list_active: Hsla,
    pub list_hover: Hsla,
    pub progress_bar: Hsla,
    // generic component-state (apply_config derives their *_hover/*_active/*_foreground)
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub info: Hsla,
    // overlay scrim (card disconnect overlay, dialogs)
    pub overlay: Hsla,
}

/// One saturated color per wave state. Consumers use it directly (border, glow, tile icon, STATUS
/// label, progress fill, branch text) or a derived tint via `Colorize::opacity/mix` — the 12/14/30%
/// mixes are code, not tokens (D2). No `on_fill`/contrast token: the locked card is the 44px icon-tile
/// + card-overlay (handoff decision 2 + `board-home.html`), not a filled text pill, so nothing paints
/// text on a saturated fill. A future filled banner needing contrast ink adds a token then — zero churn
/// to existing consumers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StatusTokens {
    pub ready: Hsla,
    pub working: Hsla,
    pub needs_input: Hsla,
    pub failed: Hsla,
    pub slept: Hsla,
    pub neutral: Hsla,
}
```

### 3.2 Accessor

Extension trait mirroring gpui-component's own `ActiveTheme` pattern (so call sites read `cx.lens_theme()`
alongside `cx.theme()`):

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

In render `cx` is `&mut Context<Self>`, which derefs to `App`, so `cx.lens_theme()` resolves.
**Implementation constraint (grok review):** views must read tokens **in `render`**, not cache an `Hsla`
in entity state, or a reload won't repaint them.

### 3.3 The bridge — `LensTheme → ThemeConfig`, apply via the public API

We build a gpui-component `ThemeConfig` from our base tokens and call the **public**
`Theme::apply_config(&Rc<ThemeConfig>)`. That method (schema.rs:645) does three things we want:
(1) stores the config as `self.dark_theme` or `self.light_theme` (by `config.mode`) — so a later
`Theme::change` re-applies **our** config, not gpui-component's default (the wipe hazard grok flagged is
gone); (2) sets `self.colors` via `ThemeColor::apply_config`, which **derives** every interaction family
(`primary_hover = bg.blend(primary·0.9)`, `primary_active = primary.darken(0.2/0.1)`, `*_foreground`
fallbacks, …) from the base colors we supply; (3) applies highlight/fonts/radius (we leave those `None`
→ gpui-component defaults for now).

```rust
use std::rc::Rc;
use gpui_component::{Theme, ThemeConfig};
use gpui_component::theme::ThemeConfigColors;   // exact import path per crate; hex Strings
use gpui_component::Colorize;                    // for Hsla::to_hex

fn to_theme_config(lens: &LensTheme) -> ThemeConfig {
    let b = &lens.base;
    let hex = |c: Hsla| Some(SharedString::from(c.to_hex()));  // try_parse_color accepts "#RRGGBB"
    ThemeConfig {
        name: lens.name.clone(),
        mode: lens.mode,
        colors: ThemeConfigColors {
            background: hex(b.background),
            foreground: hex(b.foreground),
            border: hex(b.border),
            muted: hex(b.muted),
            muted_foreground: hex(b.muted_foreground),
            popover: hex(b.popover),
            popover_foreground: hex(b.popover_foreground),
            // our brand color seeds `primary` (buttons/switch/checkbox read primary, NOT accent);
            // `secondary` (subtle button bg) seeds from muted; gpui-component's `accent` (menuitem
            // hover bg) seeds from list_hover. *_hover/*_active/*_foreground are left None → derived.
            primary: hex(b.accent),
            primary_foreground: hex(b.accent_foreground),
            secondary: hex(b.muted),
            accent: hex(b.list_hover),
            input: hex(b.input),
            caret: hex(b.caret),
            ring: hex(b.ring),
            selection: hex(b.selection),
            scrollbar: hex(b.scrollbar),
            scrollbar_thumb: hex(b.scrollbar_thumb),
            list: hex(b.list),
            list_active: hex(b.list_active),
            list_hover: hex(b.list_hover),
            progress_bar: hex(b.progress_bar),
            sidebar: hex(b.sidebar),
            sidebar_foreground: hex(b.sidebar_foreground),
            sidebar_border: hex(b.sidebar_border),
            title_bar: hex(b.title_bar),
            title_bar_border: hex(b.title_bar_border),
            tab: hex(b.tab),
            tab_active: hex(b.tab_active),
            tab_active_foreground: hex(b.tab_active_foreground),
            tab_foreground: hex(b.tab_foreground),
            success: hex(b.success),
            warning: hex(b.warning),
            danger: hex(b.danger),
            info: hex(b.info),
            overlay: hex(b.overlay),
            ..Default::default()   // everything else rides gpui-component's default for this mode
        },
        highlight: None,
        ..Default::default()
    }
}

/// Foreground-thread, pure (no I/O): install both globals. Widgets read `cx.theme()` on paint.
fn apply(lens: LensTheme, cx: &mut App) {
    Theme::global_mut(cx).apply_config(&Rc::new(to_theme_config(&lens)));
    cx.set_global(lens);   // our own call sites read base/status as Hsla via cx.lens_theme()
}
```

(`ThemeConfigColors` field/import names to be confirmed against the crate during implementation — the
mapping is mechanical.) We keep `LensTheme` as its own global because our custom surfaces read
`base.*`/`status.*` as `Hsla` directly; the config path is only how gpui-component's own widgets get our
palette.

### 3.4 Selection, loading (off-thread), and reload

**I/O rule (MANDATORY — AGENTS.md:21 / .agents/rust-ui.md:8):** never block the gpui foreground thread;
all disk reads happen off it. So loading is split from applying:

```rust
/// Pure: parse + validate mode. No I/O, no env — fully unit-testable.
fn parse_theme(json: &str, expected: ThemeMode) -> anyhow::Result<LensTheme> {
    let t: LensTheme = serde_json::from_str(json)?;
    anyhow::ensure!(t.mode == expected,
        "theme mode {:?} != expected {:?} for this file", t.mode, expected);
    Ok(t)
}

/// Off-thread I/O: resolve the source for `mode` and parse it. External file wins if present+valid;
/// otherwise the embedded default. Returns Err only if the *embedded* default is bad (a build bug).
fn load(mode: ThemeMode, dir: Option<&Path>) -> anyhow::Result<LensTheme> {
    const DARK: &str = include_str!("lens-dark.json");
    const LIGHT: &str = include_str!("lens-light.json");
    let (embedded, file) = if mode.is_dark() { (DARK, "lens-dark.json") }
                           else              { (LIGHT, "lens-light.json") };
    if let Some(dir) = dir {
        let path = dir.join(file);
        match std::fs::read_to_string(&path).map_err(anyhow::Error::from)
            .and_then(|s| parse_theme(&s, mode))
        {
            Ok(lens) => return Ok(lens),
            Err(e) => eprintln!("lens-theme: {} — using embedded default: {e}", path.display()),
        }
    }
    parse_theme(embedded, mode)
}

/// Resolve mode: LENS_THEME override (warn on unknown value) else the OS appearance.
fn select_mode(cx: &App) -> ThemeMode {
    match std::env::var("LENS_THEME").ok().as_deref() {
        Some("light") => ThemeMode::Light,
        Some("dark")  => ThemeMode::Dark,
        Some(other)   => { eprintln!("lens-theme: ignoring LENS_THEME={other:?}"); Theme::global(cx).mode }
        None          => Theme::global(cx).mode,   // synced from the OS by gpui_component::init
    }
}
```

- **Startup:** the read happens **before the render loop is live**. `main` resolves `mode` + the
  `LENS_THEME_DIR` override and calls `load(...)` (plain I/O, not on the gpui fg thread — either before
  `Application::run`, or via `cx.background_executor().spawn` early in the window setup), then `apply(...)`
  on the foreground. If `load` returns `Err` (embedded default itself is unparseable — a build bug),
  `main` prints and exits non-zero, matching the existing `eprintln!`+`process::exit(1)` pattern in
  `main.rs`. **No `panic!`/`expect`** in the theme module (no-process-panic).
- **Reload** (manual, `ReloadTheme` keybinding): read+parse on `cx.background_executor()`, then on the
  foreground: on `Ok`, `apply(...)` + `cx.refresh_windows()`; on `Err`, **keep the current active theme**
  (do *not* reset to embedded — a half-saved edit shouldn't blow away the running palette) and log.
  Uses `cx.refresh_windows()` (app.rs:755), not `window.refresh()` — the theme is app-global.

Selection is **at startup** otherwise: if the OS flips dark↔light while running we do not auto re-bridge
(deferred live toggle) — reload, relaunch, or set `LENS_THEME`.

`main.rs` calls the startup load+apply after each `gpui_component::init(cx)` — **two sites** (live run +
`--demo`) — and binds `ReloadTheme` to a key (e.g. `cmd-shift-t`), handled where a `Window`/`cx` is in
scope (like the existing `cmd-.`/`BackToBoard`), calling the reload path.

---

## 4. Theme file format

Hex strings (the format importers and future themes reuse). Forward/backward compatible: the parser does
**not** `deny_unknown_fields`, and deferred groups will be `#[serde(default)]` when added, so today's file
omitting terminal/diff parses against a future binary and vice-versa. `base` and `status` are required.

**All status values below are placeholders pending on-device eyeballing** (per user). Dark is *seeded*
from the locked `board-home.html` render; light is a first cut. Both will be retuned against real surfaces
during the wave/board build using the reload loop. Status colors double as small text (STATUS label,
branch), so they must clear the §6 contrast test against the card surface — the light values here are set
text-safe (≥3:1 on white for the five active waves) but will move during eyeballing.

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
    "input": "#2c3442",
    "caret": "#4c8dff",
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

Dark base hexes are lifted from the locked `board-home.html :root` (`--bg #07080b`, `--bg1 #101319`,
`--bg2 #151922`, `--bg3 #1c2230`, `--line #222936`, `--line2 #2c3442`, `--tx #eef2f7`, `--tx2 #9aa4b3`).
`input` is a *border* color (`--line2`), not a fill. Status colors are seeded from the D2 wave palette.

### 4.2 `lens-light.json` — "Lens Light" (first-cut)

No locked light SSOT exists, so this is authored for *structural* correctness — light surfaces, dark
text, hue-matched-but-legible accents/status on a light background.

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
    "input": "#c2c9d4",
    "caret": "#2f6bd8",
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
    "slept": "#6b7280",
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

Each `Hsla` field carries `#[serde(with = "hex_hsla")]` (bare `Hsla` serde is RGBA-shaped, not hex —
confirmed in review). `mode` deserializes directly into `gpui_component::ThemeMode`.

---

## 5. Deferred group *shapes* (specified, not built)

Built when their consuming surface lands. Recorded so the file format + importers have a target and
adding them later is mechanical. **Provisional** — values are placeholders (per
`premature-layer-boundary-binding`: specify the shape, don't lock the values).

### 5.1 Terminal (group 3) — D4 step 7, with the terminal renderer
Feeds the libghostty_vt + ghostty_rs custom gpui renderer (in progress at `../lens-terminal-ws`). ~20
tokens; target of the iTerm/Alacritty importer.

```rust
pub struct TerminalTokens {
    pub foreground: Hsla, pub background: Hsla, pub cursor: Hsla, pub selection: Hsla,
    pub normal: AnsiSet, pub bright: AnsiSet,
}
pub struct AnsiSet { pub black: Hsla, pub red: Hsla, pub green: Hsla, pub yellow: Hsla,
                     pub blue: Hsla, pub magenta: Hsla, pub cyan: Hsla, pub white: Hsla }
```
JSON key: `"terminal": { "foreground": …, "normal": { "black": …, … }, "bright": { … } }`.

### 5.2 Diff (group 4) — D4 step 5, with the transcript surface
gpui-component has `bullish`/`bearish` + red/green but no diff-semantic bg pairs. ~6 tokens.

```rust
pub struct DiffTokens {
    pub added_bg: Hsla, pub added_fg: Hsla, pub removed_bg: Hsla, pub removed_fg: Hsla,
    pub context_fg: Hsla, pub hunk_header: Hsla,
}
```
JSON key: `"diff": { "added_bg": …, … }`. Each gets `#[serde(default)]` on `LensTheme` when added.

---

## 6. Testing

Pure where possible (`parse_theme`/`load` take explicit `mode`/`dir`, so tests never touch process env):

1. **Both embedded themes parse + mode matches file** — `parse_theme(include_str!("lens-dark.json"),
   Dark)` and `…("lens-light.json", Light)` are `Ok` with names "Lens Dark"/"Lens Light".
2. **Mode-mismatch rejected** — `parse_theme(dark_json, Light)` is `Err` (guards the selection bug where a
   wrong `mode` in a file would flip the global mode and re-select a different file on next reload).
3. **Dark-status seed guard** — the six dark `status.*` equal the `board-home.html` seed hexes; when
   intentionally retuned, update render + this test together.
4. **Light expresses distinctly** — light `base.background` ≠ dark's and light `base.foreground` is darker
   than its background (cheap "not dark-baked" check).
5. **Status text-contrast** — for each theme, the five active `status.*` (excluding `neutral`, the
   de-emphasized idle state rendered via `muted_foreground`) have WCAG contrast **≥ 3:1** against the card
   surface (`base.list`). Durable guard for "status is legible as a label"; the light placeholders are set
   to pass and retuning must keep passing.
6. **hex round-trip** — `parse_hex → to_hex → parse_hex` stable for a sample token.
7. **External override precedence** — `load(Dark, Some(tmpdir))` with a modified `lens-dark.json` returns
   the on-disk values.
8. **Bad external file falls back** — `load(Dark, Some(tmpdir))` with a malformed file returns the
   embedded default (`Ok`, no panic).
9. **`Wave → status` totality** (in `card/wave.rs`) — all six `Wave` variants resolve; adding a variant
   fails to compile.
10. **Bridge (mandatory, gpui `test_app`)** — after startup `apply`: `cx.theme().mode` == `lens.mode`;
    `cx.theme().background` == `lens.base.background`; `cx.theme().primary` == `lens.base.accent`; a
    **derived** family is non-default (`cx.theme().primary_hover != ThemeColor::default().primary_hover`);
    and after a subsequent `Theme::change(lens.mode, …)` the palette is **still ours** (`primary` unchanged)
    — proving the config-store defeats the wipe hazard.

---

## 7. A2 — hex→token call-site migration (companion, runs right after the substrate)

Not part of the substrate build but the immediate next step it unblocks (D4). `chrome.rs` today has 12
hardcoded hexes. The **pill + `pill_text_color`** are throwaway — the wave build (B1) replaces the pill
with the icon-tile, so A2 does **not** migrate them; it re-homes the surviving call sites and fixes the
border color (current code uses a different, wrong palette). The `Wave → status color` map lives in
`card/wave.rs` (keeps `theme` a leaf):

```rust
// crates/lens-ui/src/card/wave.rs
impl Wave {
    pub fn status_color(self, t: &LensTheme) -> Hsla {
        match self {
            Wave::Ready => t.status.ready,      Wave::Working => t.status.working,
            Wave::NeedsInput => t.status.needs_input, Wave::Failed => t.status.failed,
            Wave::Slept => t.status.slept,      Wave::Neutral => t.status.neutral,
        }
    }
}
```

| current `chrome.rs`                          | becomes                                        |
|----------------------------------------------|------------------------------------------------|
| `wave_border_color` (6 rgb, wrong palette)   | `wave.status_color(cx.lens_theme())`           |
| kebab menu bg `0x1f2937`                      | `cx.lens_theme().base.popover`                 |
| muted text `0x9ca3af` (×3)                    | `cx.lens_theme().base.muted_foreground`        |
| overlay text `0xf3f4f6`                       | `cx.lens_theme().base.foreground`              |
| overlay scrim `hsla(0,0,0,0.55)`              | `cx.lens_theme().base.overlay.opacity(0.55)`   |
| pill fill + `pill_text_color`                 | *(not migrated — deleted by B1's icon-tile)*   |

Existing chrome unit tests are color-agnostic (repo-row text formatting) and unaffected.

---

## 8. Files touched

- **New:** `crates/lens-ui/src/theme/mod.rs`, `crates/lens-ui/src/theme/tokens.rs`,
  `crates/lens-ui/src/theme/lens-dark.json`, `crates/lens-ui/src/theme/lens-light.json`.
- **Edit:** `crates/lens-ui/src/lib.rs` (`pub mod theme;` + re-export `ActiveLensTheme`, `LensTheme`; the
  `ReloadTheme` action + its handler in the root view).
- **Edit:** `crates/lens-app/src/main.rs` (startup `load`+`apply` after both `gpui_component::init(cx)`
  sites, off the fg thread; bind the `ReloadTheme` key; on startup `load` Err, `eprintln`+`exit`).
- **A2 (companion):** `crates/lens-ui/src/card/chrome.rs`, `crates/lens-ui/src/card/wave.rs`.

No new dependencies — `gpui-component` (`Colorize`, `Theme`, `ThemeConfig`, `ThemeConfigColors`,
`ThemeMode`), `serde`, `serde_json`, `anyhow` are already in `lens-ui`; loading uses `std::fs`/`std::env`.

---

## 9. Verification (definition of done for the substrate)

- `cargo test -p lens-ui` green (the §6 tests, incl. the mandatory bridge test).
- `cargo run -p lens-app -- --demo` shows the six cards in the seeded wave colors (after A2); running with
  `LENS_THEME=light` shows the light palette on the same surfaces — proving the schema drives both themes,
  and gpui-component buttons/inputs (once present) pick up the palette via `apply_config`.
- **Reload loop:** with `LENS_THEME_DIR=crates/lens-ui/src/theme`, edit a color in `lens-dark.json`, press
  the reload key, and the change appears **without restart** (off-thread read → foreground apply →
  `refresh_windows`).
- `xtask gate` clean (no warnings / dead code).
- Cross-family review of the diff (per project rules) — this design already had one (grok-4.5 +
  gpt-5.6-sol); the *implementation* diff gets another.
```