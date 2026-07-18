# Handoff — §18 theming brainstorm: decisions locked, resume at "write the design doc"

**Date:** 2026-07-16 (evening session)
**Branch:** `feat/lens-app-multi-session` (no new commits this session — brainstorm only)
**Status:** Brainstorm decisions reached (fork + token surface + scope + sequencing). **Design doc NOT
yet written.** Next session resumes the `superpowers:brainstorming` flow at step 6 = write the theming
design doc → user review → `writing-plans` → implement the minimal substrate → then the wave build.

**Supersedes the "NEXT: §18 theming brainstorm" section of**
`docs/handoffs/2026-07-16-lens-ui-theming-and-card-audit.md` (that doc's audit ledger A/B/C + B1–B8 is
still the SSOT for the wave build; only its open fork is now resolved here).

---

## Decisions locked this session (user-approved)

### D1 — Substrate fork: BRIDGE, do NOT vendor gpui-component's theme
Own a **`LensTheme` superset** (our file + schema + tokens). Bridge into gpui-component by
**constructing its public `Theme`/`ThemeColor` from our base-token subset** at init/reload and
`set_global` — no fork.

**Why (airtight, root-vs-leaf):** gpui-component's theme is the crate **root** — **85 of its source
files** do `use crate::{ActiveTheme}` + read `cx.theme().field`. A crates.io-compiled component can
**never** see an extended `ThemeColor` in our tree. So "extend their Theme" is only possible by forking
the **entire 60-component crate** and rebasing it on upstream (young, 0.5.x, self-pins gpui) **forever** —
exactly the whole-crate vendor `framework.md:218` rejected. The standing "vendor just the *markdown*
module" decision works precisely because markdown is a **leaf** (a copied leaf still imports the
crates.io `ActiveTheme` and just works); the theme lacks that property. The whole-crate fork's *only*
unique benefit — making gpui-component's own widgets `status.*`-aware — is something we never need
(`status.*` drives our custom card, not their buttons). The two originally-listed options (adopt-only
"map waves onto their tokens", and pure "own struct + reimplement machinery") are **non-starters** per
user.

**Bridge is cheap:** gpui-component's `init` already builds a fully-populated default `Theme`; their
`apply_config` fallback machinery is `pub(crate)` (can't call it) but we don't need to — our reload just
**overrides the ~15 base tokens** our file specifies on top of their default:
`let c = &mut Theme::global_mut(cx).colors; c.background = lens.bg; c.border = lens.border; …`.
Their components render on our base palette; our custom surfaces read `cx.lens_theme()`.

### D2 — Token surface: 4 groups (base maps out; 3 are ours)
- **Group 1 — base (maps 1:1 onto gpui-component `ThemeColor`, one source of truth):** background,
  foreground, border, muted, accent, popover, sidebar.*, title_bar.*, tab.*, input, ring, selection,
  scrollbar.*, list.*, progress_bar — **plus** its generic component-state tokens `success/warning/
  danger/info` and its `HighlightTheme` (tree-sitter syntax → code highlighting free). ~15 we author;
  rest ride their default.
- **Group 2 — `status.*` (OURS, ~12):** 6 wave states × {fill, on-fill fg}. Banners read these too.
  Locked colors (from `board-home.html :root`): ready=blue `#4c8dff`, working=green `#36c98a`,
  needs_input=**orange** `#ff8a3d`, failed=red `#ff5d5d`, slept=gray `#7a8493`, neutral=dim. (purple
  `#b08cff` available.)
- **Group 3 — terminal ANSI (OURS, ~20):** fg/bg/cursor/selection + 8 normal + 8 bright. Feeds the
  **libghostty_vt + ghostty_rs + custom gpui renderer** terminal surface (NOT alacritty — correction
  this session; in-progress at `../lens-terminal-ws`, currently client/core/store only, no
  renderer/palette yet). Target of §18's iTerm/Alacritty importer.
- **Group 4 — diff (OURS, ~6):** added/removed bg+fg, context, hunk-header. gpui-component has
  bullish/bearish + red/green but no diff-semantic bg pairs.
- **Wave *behavior* (glow/tint/pulse period+style) is NOT a token — it's code keyed by `Wave`.** Stays
  out of the theme file.

### D3 — Scope of THIS effort (the minimal substrate)
Only the **token schema is load-bearing** (every call site bakes into `cx.lens_theme().status.working`);
all delivery machinery sits behind the same accessor and adds later with **zero call-site churn**.
- **Build now:** the schema (design with room for all 4 groups; author only base+status+dark) +
  `LensTheme` struct + `cx.lens_theme()` accessor + global + the D1 bridge + **one default dark theme
  "Lens Dark Deep" as embedded JSON** (`include_str!`, parsed once at startup — gives the file *format*
  + serde types importers reuse later; **no watcher, no registry, no multi-theme**; restart to see
  edits). Schema designed so **light is expressible** (semantic names, no dark-baked assumptions).
- **Defer:** external file loading / themes dir, registry / multi-theme, hot-reload watcher, light
  *authoring*, importers, settings picker, JsonSchema.

### D4 — Sequencing (agreed)
1. **Theming schema + dark + bridge** *(next session — the only prerequisite for tokenized call sites)*
2. **Wave build (B1–B5)** — completes the card; small; `--demo`-driven; delivers the wedge; validates
   the schema immediately.
3. **Board packing (B6–B8)** — ordinal slots + adaptive packing + lanes. → **board (the differentiator)
   done.**
4. **Light theme authoring** *(checkpoint, not full machinery)* — forcing function proving the schema is
   truly semantic while only ~2 surfaces exist.
5. **Transcript surface** — markdown vendor+patch, native `list()`, diff, code highlight. The big one.
6. **Composer + elicitation/forms.**
7. **Side pane + terminal + editor** (terminal renderer matures in `lens-terminal-ws` in parallel).
8. **Shell chrome polish.**
9. **Full theming machinery (importers/picker/registry/hot-reload/JsonSchema) — LAST.**

**Two opinionated calls the user accepted:** (a) waves+board **before** transcript (board is the wedge,
small, high value/effort); (b) theming *machinery* is the **least** urgent UI work (nothing depends on
it; themes surfaces that don't exist yet) → **step 9, not "up next."** Hot-reload may creep to ~step 6–7
when authoring starts to hurt. **File as TWO up-next items:** "wave build (B1–B5)" (immediate) +
"full theming machinery §18" (later, standalone).

---

## Resume checklist (next session)
1. Re-enter `superpowers:brainstorming` at step 6 (design already approved through fork+scope+seq).
2. **Write the design doc** → `docs/specs/2026-07-1X-theming-substrate-design.md`. Fully
   specify: the `LensTheme` struct + all 4 token-group schemas (base/status/terminal/diff, even the
   deferred groups' *shape*), the embedded-dark-JSON format, the bridge fn, the `cx.lens_theme()`
   accessor. Commit.
3. Self-review (placeholders/consistency/scope/ambiguity) → user review gate.
4. `writing-plans` → implement minimal substrate → then A2 (hex→token migration) → wave build.

**Grounding facts verified this session:** `ThemeConfigColors` has **no serde catch-all** (unknown keys
dropped — why status.* can't ride their file). `apply_config` is `pub(crate)`. 85 files depend on
`crate::theme`. gpui-component `ThemeColor` already has success/warning/danger/info + red/green/blue/
yellow/magenta/cyan + progress_bar + chart.1–5 + HighlightTheme.
