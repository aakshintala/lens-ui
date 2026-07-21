# Board B-3 — Group Chrome & Rollups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fill the B-2 group placeholder box with real group chrome — a colored ring + tint, a header-lane (`● dot · name · [spend · age] · ✓N · ⌄`), and aggregation rollups folded from member cards.

**Architecture:** B-2 shipped `absolute_group` in `crates/lens-ui/src/board/mod.rs`, which today draws a bare neutral bordered box plus full-size member cards. B-3 adds (1) a pure `GroupRollup` fold + formatters over member `SessionCard`s, (2) a `created_at` plumb onto `SessionCard` so the age rollup has a source, (3) a `group_accent` token→color resolver, and (4) rewrites `absolute_group` to thread group metadata (name, color token, completed-count) and render the ring/tint/header from the rollup. A test-only `BoardLayout` injection seam lets a gpui fixture test exercise the group path, which `build_ephemeral_layout` never reaches at runtime (no group is creatable until B-4).

**Tech Stack:** Rust, gpui 0.2.2, `lens-core` (pure domain: `BoardLayout`, `pack`), `lens-ui` (gpui views). No new dependencies.

## Global Constraints

- **Runtime-dormant.** `build_ephemeral_layout` produces **zero groups** (`crates/lens-ui/src/board/layout_adapter.rs:22`), so the group render path is unreachable at runtime until B-4 lands the store→replica write seam. B-3 is therefore **fixture-tested only** — do not add a runtime group source.
- **Geometry is frozen by B-2.** `absolute_group`'s box/member positions (spec §2.4) are correct and tested; B-3 changes **chrome only**, never the ring/member rects. Constants: `CARD_W=280`, `CARD_H=160`, `HEADER=24`, `GAP=16`, `INSET=5`, `CELL_W=296`, `CELL_H=200` (from `lens_core::pack`).
- **`✓N` = completed→Archive count, NOT active-member count** (spec §3.1). The real count is an Archive-side source that does not exist until **B-6**; B-3 carries it as a `completed_count` field defaulting to `0` and renders it. Do NOT invent an active-count badge.
- **Age source = oldest member `created_at`** (spec §3.1). `SessionState.created_at` is **epoch SECONDS** (`crates/lens-core/src/domain/session.rs:29`); the UI clock is **epoch millis**. Convert when formatting. When no member has a `created_at`, age renders `—` (the spec's optional group-`created_at` fallback is intentionally omitted to avoid a seconds-vs-millis unit mix).
- **Palette is a B-3-owned pure resolver, not a theme token yet.** SSOT accent hexes from `docs/design/renders/board-home.html:8-12`: blue `#4c8dff`, orange `#ff8a3d`, green `#36c98a`, purple `#b08cff`. Promoting these to `LensTheme` tokens (dark+light+serde) is a documented follow-up, not B-3 (matches the B-2 arm, which hardcoded its border color).
- **Gate command is `cargo run -p xtask -- gate`** (NOT `cargo xtask gate` — no alias). It uses explicit `-p` lists that exclude `spikes/`. The pre-existing `spikes/board-container` clippy/fmt red is unrelated — do NOT touch the spike. Run `cargo fmt` before the gate.
- **Reviewer diversity (project rule):** every non-trivial task gets ≥1 review from a model family other than the author's; route gpt-5.6 review through `codex exec -s read-only`.

---

## File Structure

- **Create** `crates/lens-ui/src/board/rollup.rs` — pure (no gpui): `GroupRollup` struct, `group_rollup()` fold, `format_group_spend()`, `format_age()`, `group_header_text()`. Unit-tested in-file.
- **Modify** `crates/lens-ui/src/board/mod.rs` — declare `mod rollup;`; add `group_accent()` resolver; add `GroupMeta` + `GroupChromeSnapshot`; thread group metadata through `pack_and_render`; rewrite `absolute_group` to render chrome + emit a snapshot; add `test_layout` field + `set_test_layout_for_test` + `group_chrome_for_test` hooks.
- **Modify** `crates/lens-ui/src/card/model.rs` — add `created_at: Option<i64>` to `SessionCard`; init in `new`; set in the `Rebased` fold arm.
- **Modify** `crates/lens-ui/tests/acceptance_shell.rs` — add one `#[gpui::test]` that injects a group `BoardLayout` and asserts the rendered chrome snapshot.

---

### Task 1: `GroupRollup` pure fold + formatters

**Files:**
- Create: `crates/lens-ui/src/board/rollup.rs`
- Modify: `crates/lens-ui/src/board/mod.rs:1` (add `mod rollup;`)

**Interfaces:**
- Consumes: `crate::card::model::SessionCard` (its `cumulative_cost: Cost` and the `created_at: Option<i64>` field added in Task 2 — this task compiles against the field, so do Task 2 first OR add the field as part of Step 3 here; the plan orders Task 2 second, so implement Task 2's field addition before running Task 1's tests).
- Produces:
  - `pub struct GroupRollup { pub spend_usd: Option<f64>, pub oldest_created_at: Option<i64>, pub completed_count: u32 }` (derives `Clone, Debug, PartialEq`)
  - `pub fn group_rollup(members: &[&SessionCard], completed_count: u32) -> GroupRollup`
  - `pub fn format_group_spend(spend_usd: Option<f64>) -> String`
  - `pub fn format_age(oldest_created_at_secs: Option<i64>, now_ms: i64) -> String`
  - `pub fn group_header_text(name: &str, rollup: &GroupRollup, now_ms: i64) -> String`

> **Ordering note:** `group_rollup` reads `SessionCard.created_at`, which Task 2 adds. Implement **Task 2's Step 3 (add the field) before running this task's tests**, or the fold won't compile. The two tasks are split for reviewability but share this one field dependency.

- [ ] **Step 1: Write the failing tests**

Create `crates/lens-ui/src/board/rollup.rs` with the test module first:

```rust
//! Pure group aggregation (B-3, spec §3.1): fold member `SessionCard`s into the
//! header-lane rollup (spend / age / ✓N-completed) and format it. No gpui — pure,
//! deterministic, unit-tested. `completed_count` is Archive-side (B-6); B-3 carries
//! it as an input defaulting to 0.

use crate::card::model::SessionCard;
use lens_core::domain::ids::SessionId;

#[cfg(test)]
mod tests {
    use super::*;
    use lens_core::domain::usage::Cost;

    fn card(id: &str, cost: Option<f64>, created_at: Option<i64>) -> SessionCard {
        let mut c = SessionCard::new(SessionId::new(id));
        c.cumulative_cost = Cost {
            total_cost_usd: cost,
            ..Cost::default()
        };
        c.created_at = created_at;
        c
    }

    #[test]
    fn rollup_sums_spend_and_takes_oldest_created_at() {
        let a = card("s1", Some(1.50), Some(2000));
        let b = card("s2", Some(2.00), Some(1000));
        let members = [&a, &b];
        let r = group_rollup(&members, 3);
        assert_eq!(r.spend_usd, Some(3.50));
        assert_eq!(r.oldest_created_at, Some(1000)); // min, not max
        assert_eq!(r.completed_count, 3);
    }

    #[test]
    fn rollup_partial_data_is_tolerated() {
        // one member has cost, neither the other; one has created_at, not the other.
        let a = card("s1", Some(0.75), None);
        let b = card("s2", None, Some(5000));
        let members = [&a, &b];
        let r = group_rollup(&members, 0);
        assert_eq!(r.spend_usd, Some(0.75));
        assert_eq!(r.oldest_created_at, Some(5000));
    }

    #[test]
    fn rollup_all_absent_is_none() {
        let a = card("s1", None, None);
        let members = [&a];
        let r = group_rollup(&members, 0);
        assert_eq!(r.spend_usd, None);
        assert_eq!(r.oldest_created_at, None);
    }

    #[test]
    fn format_spend_matches_card_style() {
        assert_eq!(format_group_spend(Some(3.5)), "~$3.50");
        assert_eq!(format_group_spend(None), "—");
    }

    #[test]
    fn format_age_buckets_minutes_hours_days() {
        // created 1000s ago; now 1000s + 2600s = 3600s → 2600s = 43m
        assert_eq!(format_age(Some(1000), 3_600_000), "43m");
        // 2h: created at 0s, now 7200s
        assert_eq!(format_age(Some(0), 7_200_000), "2h");
        // 3d: created at 0s, now 3*86400s
        assert_eq!(format_age(Some(0), 259_200_000), "3d");
        // absent → em dash
        assert_eq!(format_age(None, 3_600_000), "—");
        // future/zero clamps to 0m, never negative
        assert_eq!(format_age(Some(10_000), 0), "0m");
    }

    #[test]
    fn header_text_assembles_spec_order() {
        let r = GroupRollup {
            spend_usd: Some(3.5),
            oldest_created_at: Some(0),
            completed_count: 2,
        };
        assert_eq!(group_header_text("Refactor", &r, 7_200_000), "Refactor · ~$3.50 · 2h · ✓2");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p lens-ui --lib board::rollup 2>&1 | tail -20`
Expected: FAIL — `cannot find function group_rollup` / `GroupRollup` etc. (Note: if `SessionCard.created_at` is not yet added, you will instead get "no field `created_at`" — add Task 2's field first, then this failure resolves to the missing-function errors.)

- [ ] **Step 3: Write the implementation**

Insert above the `#[cfg(test)]` module in `crates/lens-ui/src/board/rollup.rs`:

```rust
/// Pure fold over a group's member cards (spec §3.1). `completed_count` is supplied
/// by the caller (Archive-side; B-6 wires the real source, B-3 passes 0).
#[derive(Clone, Debug, PartialEq)]
pub struct GroupRollup {
    /// Σ member `total_cost_usd`; `None` only when no member reports a cost.
    pub spend_usd: Option<f64>,
    /// Oldest member `created_at` (epoch SECONDS); `None` when no member has one.
    pub oldest_created_at: Option<i64>,
    /// Completed → Archive count (B-6 source); 0 in B-3.
    pub completed_count: u32,
}

pub fn group_rollup(members: &[&SessionCard], completed_count: u32) -> GroupRollup {
    let mut spend_usd: Option<f64> = None;
    let mut oldest_created_at: Option<i64> = None;
    for m in members {
        if let Some(c) = m.cumulative_cost.total_cost_usd {
            spend_usd = Some(spend_usd.unwrap_or(0.0) + c);
        }
        if let Some(ca) = m.created_at {
            oldest_created_at = Some(oldest_created_at.map_or(ca, |o| o.min(ca)));
        }
    }
    GroupRollup {
        spend_usd,
        oldest_created_at,
        completed_count,
    }
}

/// `~$X.XX`, or `—` when unknown. Mirrors `card::chrome::format_spend`.
pub fn format_group_spend(spend_usd: Option<f64>) -> String {
    match spend_usd {
        Some(usd) => format!("~${usd:.2}"),
        None => "—".into(),
    }
}

/// Coarse age bucket from the oldest member's `created_at` (epoch SECONDS) vs the
/// current UI clock (epoch MILLIS). `—` when no source. Never negative.
pub fn format_age(oldest_created_at_secs: Option<i64>, now_ms: i64) -> String {
    let Some(created) = oldest_created_at_secs else {
        return "—".into();
    };
    let now_s = now_ms / 1000;
    let secs = (now_s - created).max(0);
    if secs < 3_600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3_600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// The header-lane text (spec §3): `name · spend · age · ✓N`. The colored dot and
/// the `⌄` caret are rendered as elements, not part of this string.
pub fn group_header_text(name: &str, rollup: &GroupRollup, now_ms: i64) -> String {
    format!(
        "{name} · {} · {} · ✓{}",
        format_group_spend(rollup.spend_usd),
        format_age(rollup.oldest_created_at, now_ms),
        rollup.completed_count
    )
}
```

Then add the module declaration at the top of `crates/lens-ui/src/board/mod.rs:1` (it currently starts with `mod layout_adapter;`):

```rust
mod layout_adapter;
mod rollup;
```

The `use lens_core::domain::ids::SessionId;` import at the top of `rollup.rs` is currently unused by the non-test code; remove it if clippy flags it, or keep it if a later step references it. (Verify with the gate in a later task; for now the fold uses only `SessionCard` + `Cost`.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p lens-ui --lib board::rollup 2>&1 | tail -20`
Expected: PASS — 6 tests pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/lens-ui/src/board/rollup.rs crates/lens-ui/src/board/mod.rs
git commit -m "feat(board): pure GroupRollup fold + header formatters (B-3)"
```

---

### Task 2: Plumb `created_at` onto `SessionCard`

**Files:**
- Modify: `crates/lens-ui/src/card/model.rs:20-65` (struct), `:76-112` (`new`), `:163-193` (`Rebased` arm)
- Test: `crates/lens-ui/src/card/model.rs` (in-file `#[cfg(test)]`)

**Interfaces:**
- Produces: `SessionCard.created_at: Option<i64>` — epoch SECONDS, `None` until the first `Detailed(Rebased)` fold. Consumed by `group_rollup` (Task 1).

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `crates/lens-ui/src/card/model.rs` (after `resources_changed_does_not_clear_branch`):

```rust
    #[test]
    fn rebased_fold_plumbs_created_at() {
        let mut card = SessionCard::new(SessionId::new("s"));
        assert_eq!(card.created_at, None, "fresh card has no created_at");
        let clock = crate::clock::ManualUiClock::new(0);
        let mut baseline = SessionState::new(
            ConnectionId::new("c"),
            SessionId::new("s"),
            AgentId::new("ag"),
        );
        baseline.created_at = 1_700_000_000; // epoch seconds
        card.fold_feed(
            ActorFeed::Detailed(StreamUpdate::Rebased(Box::new(baseline))),
            &clock,
        );
        assert_eq!(card.created_at, Some(1_700_000_000));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p lens-ui --lib card::model::tests::rebased_fold_plumbs_created_at 2>&1 | tail -20`
Expected: FAIL — `no field created_at on type SessionCard`.

- [ ] **Step 3: Add the field and set it**

In the `SessionCard` struct (`crates/lens-ui/src/card/model.rs`, after `pub session_id: SessionId,` at line 21) add:

```rust
    /// §3.1 rollup age source: epoch SECONDS of session creation. `None` until the
    /// first `Detailed(Rebased)` fold (Summary frames don't carry it). Fed to
    /// `board::rollup::group_rollup`.
    pub created_at: Option<i64>,
```

In `SessionCard::new` (after `session_id,` at line 78) add:

```rust
            created_at: None,
```

In `fold_detailed`, the `StreamUpdate::Rebased(state)` arm (after `self.lifecycle = state.lifecycle;` at line 181) add:

```rust
                self.created_at = Some(state.created_at);
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p lens-ui --lib card::model::tests::rebased_fold_plumbs_created_at 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Verify no other `SessionCard` struct literal broke**

Run: `grep -rn "SessionCard {" crates/ | grep -v "cx.new\|impl\|struct SessionCard"`
Expected: no matches (all construction goes through `SessionCard::new`, which now sets the field). If a literal appears, add `created_at: None` to it.

Run the whole card module to confirm nothing regressed:

Run: `cargo test -p lens-ui --lib card::model 2>&1 | tail -20`
Expected: PASS — all card model tests green.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add crates/lens-ui/src/card/model.rs
git commit -m "feat(card): plumb created_at onto SessionCard for B-3 age rollup"
```

---

### Task 3: `group_accent` color resolver

**Files:**
- Modify: `crates/lens-ui/src/board/mod.rs` (add `group_accent` fn + a `#[cfg(test)]` module)

**Interfaces:**
- Produces: `fn group_accent(token: Option<&str>) -> gpui::Hsla` — maps the four SSOT color tokens to their accent color; unknown/`None` → a neutral slate. Consumed by `absolute_group` (Task 4).

- [ ] **Step 1: Write the failing test**

`crates/lens-ui/src/board/mod.rs` has no `#[cfg(test)]` module today. Add one at the end of the file (after the `impl Render for BoardView` block, at file end):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_accent_maps_ssot_tokens() {
        assert_eq!(group_accent(Some("blue")), gpui::rgb(0x4c8dff).into());
        assert_eq!(group_accent(Some("orange")), gpui::rgb(0xff8a3d).into());
        assert_eq!(group_accent(Some("green")), gpui::rgb(0x36c98a).into());
        assert_eq!(group_accent(Some("purple")), gpui::rgb(0xb08cff).into());
    }

    #[test]
    fn group_accent_unknown_and_none_fall_back_to_neutral() {
        let neutral: gpui::Hsla = gpui::rgb(0x6b7280).into();
        assert_eq!(group_accent(None), neutral);
        assert_eq!(group_accent(Some("chartreuse")), neutral);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p lens-ui --lib board::tests::group_accent 2>&1 | tail -20`
Expected: FAIL — `cannot find function group_accent`.

- [ ] **Step 3: Implement the resolver**

Add, as a free function near the bottom of `crates/lens-ui/src/board/mod.rs` (before the `#[cfg(test)]` module):

```rust
/// Group accent color from its persisted `color_token` (spec §3, SSOT palette
/// `docs/design/renders/board-home.html:8-12`). Unknown / `None` → neutral slate.
/// B-3-local resolver; promoting these to `LensTheme` tokens is a documented
/// follow-up (matches the B-2 arm hardcoding its border color).
fn group_accent(token: Option<&str>) -> gpui::Hsla {
    let hex: u32 = match token {
        Some("blue") => 0x4c8dff,
        Some("orange") => 0xff8a3d,
        Some("green") => 0x36c98a,
        Some("purple") => 0xb08cff,
        _ => 0x6b7280, // neutral slate
    };
    gpui::rgb(hex).into()
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p lens-ui --lib board::tests::group_accent 2>&1 | tail -20`
Expected: PASS — 2 tests pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/lens-ui/src/board/mod.rs
git commit -m "feat(board): group_accent token->color resolver (B-3)"
```

---

### Task 4: Render group chrome in `absolute_group` + thread metadata + snapshot hook

**Files:**
- Modify: `crates/lens-ui/src/board/mod.rs` — imports, `BoardView` struct + `mount`, `pack_and_render`, `absolute_group`, add test hooks.

**Interfaces:**
- Consumes: `rollup::{GroupRollup, group_rollup, group_header_text}` (Task 1), `SessionCard.created_at` (Task 2), `group_accent` (Task 3).
- Produces:
  - `struct GroupMeta { name: String, color_token: Option<String>, completed_count: u32 }`
  - `pub struct GroupChromeSnapshot { pub session_ids: Vec<SessionId>, pub name: String, pub accent: gpui::Hsla, pub rollup: rollup::GroupRollup, pub header: String }` (derives `Clone, Debug`)
  - `BoardView.last_group_chrome: Vec<GroupChromeSnapshot>` + `pub fn group_chrome_for_test(&self) -> Vec<GroupChromeSnapshot>`
  - `BoardView.test_layout: Option<BoardLayout>` + `pub fn set_test_layout_for_test(&mut self, layout: BoardLayout)`

- [ ] **Step 1: Add imports, struct fields, and test hooks**

In `crates/lens-ui/src/board/mod.rs`, extend the `lens_core` imports (line 12):

```rust
use lens_core::domain::board::{BoardLayout, BoardNode};
```

Extend the `use lens_core::domain::ids::SessionId;` import is fine as-is; the `rollup` module (declared in Task 1) is in scope as `rollup::…`.

Add fields to `BoardView` (after `last_built: Vec<SessionId>,` at line 56):

```rust
    /// B-3 render snapshot: the group chrome computed at the last render (test hook;
    /// also the eventual B-4 live-inspection point). Recomputed each frame.
    last_group_chrome: Vec<GroupChromeSnapshot>,
    /// B-3 TEST SEAM: a hand-built layout injected in place of `build_ephemeral_layout`
    /// so fixture tests can reach the group path (no group is runtime-creatable until
    /// B-4). `None` in production. B-4's real store→replica seam supersedes this.
    test_layout: Option<BoardLayout>,
```

Initialize them in `mount` (in the `Self { … }` literal after `last_built: Vec::new(),` at line 97):

```rust
            last_group_chrome: Vec::new(),
            test_layout: None,
```

Add the two structs near the top of the file (after the `ShellMode` impl, before `pub struct BoardView`):

```rust
/// Per-tile group metadata threaded from `board_tree` into the renderer (B-3).
/// `completed_count` is Archive-side (B-6); B-3 passes 0.
struct GroupMeta {
    name: String,
    color_token: Option<String>,
    completed_count: u32,
}

/// The chrome computed for one rendered group tile — asserted by fixture tests
/// (the group render path is not runtime-reachable until B-4).
#[derive(Clone, Debug)]
pub struct GroupChromeSnapshot {
    pub session_ids: Vec<SessionId>,
    pub name: String,
    pub accent: gpui::Hsla,
    pub rollup: rollup::GroupRollup,
    pub header: String,
}
```

Add the test hooks alongside the existing `_for_test` methods (after `visible_session_ids_for_test`, line 345):

```rust
    /// Test hook: the group chrome computed at the last render.
    pub fn group_chrome_for_test(&self) -> Vec<GroupChromeSnapshot> {
        self.last_group_chrome.clone()
    }

    /// Test hook (B-3): inject a hand-built layout so the group render path is
    /// reachable. Production uses `build_ephemeral_layout` (no groups until B-4).
    pub fn set_test_layout_for_test(&mut self, layout: BoardLayout) {
        self.test_layout = Some(layout);
    }
```

- [ ] **Step 2: Thread group metadata + snapshot through `pack_and_render`**

In `pack_and_render` (`crates/lens-ui/src/board/mod.rs:189-267`), make three edits.

(a) Replace the layout source line (line 196) so the test seam wins when present:

```rust
        let layout = self
            .test_layout
            .clone()
            .unwrap_or_else(|| build_ephemeral_layout(self.fleet.read(cx)));
```

(b) In the `nodes → parallel` loop (lines 204-213), build a third parallel vec of group metadata. Replace that block with:

```rust
        // nodes → parallel (pack items, per-tile session ids, per-tile group meta)
        let mut items: Vec<Item> = Vec::with_capacity(nodes.len());
        let mut tile_sessions: Vec<Vec<SessionId>> = Vec::with_capacity(nodes.len());
        let mut tile_groups: Vec<Option<GroupMeta>> = Vec::with_capacity(nodes.len());
        for node in &nodes {
            let sessions: Vec<SessionId> = node.leaf_sessions().into_iter().cloned().collect();
            let (item, meta) = match node {
                BoardNode::Card(_) => (Item::card(), None),
                BoardNode::Group { item, .. } => {
                    let meta = match &item.kind {
                        lens_core::domain::board::BoardItemKind::Group {
                            name, color_token, ..
                        } => Some(GroupMeta {
                            name: name.clone(),
                            color_token: color_token.clone(),
                            // ✓N is Archive-side (B-6); B-3 has no source → 0.
                            completed_count: 0,
                        }),
                        // A Group node always carries a Group kind; defensively None.
                        _ => None,
                    };
                    (Item::group(sessions.len()), meta)
                }
            };
            items.push(item);
            tile_sessions.push(sessions);
            tile_groups.push(meta);
        }
```

(c) Compute `now_ms` once (after the `overdraw`/`lo`/`hi` block, line 222) and record the group snapshots. Add before `let mut content = …`:

```rust
        let now_ms = self.fleet.read(cx).clock().now_millis();
        let mut group_chrome: Vec<GroupChromeSnapshot> = Vec::new();
```

Then in the cull loop, replace the `pack::Kind::Group` arm (lines 249-253) with:

```rust
                pack::Kind::Group { .. } => {
                    let meta = tile_groups[placed.item_index].as_ref();
                    let (els, snap) = self.absolute_group(placed, sessions, meta, now_ms, cx);
                    for el in els {
                        content = content.child(el);
                    }
                    group_chrome.push(snap);
                }
```

Finally, after `self.last_built = visible.clone();` (line 257) add:

```rust
        self.last_group_chrome = group_chrome;
```

- [ ] **Step 3: Rewrite `absolute_group` to render chrome and emit a snapshot**

Replace the entire `absolute_group` method (`crates/lens-ui/src/board/mod.rs:301-339`) with:

```rust
    /// A group tile (B-3): a colored ring + tint in the inter-tile gap, a header-lane
    /// (`● dot · name · [spend · age] · ✓N · ⌄`) folded from member cards, plus the
    /// members at full size in their body-zones. Returns the elements and a chrome
    /// snapshot (fixture tests assert the snapshot; the path is not runtime-reachable
    /// until B-4).
    ///
    /// NOTE (B-4): this reads member `SessionCard` entities during `render` to fold the
    /// rollup. B-3 is runtime-dormant so this never executes live; when B-4 makes groups
    /// live, verify this does not re-trip the `.cached()` dirty-tracking freeze
    /// ([[viewport-reentry-freeze]]). If it does, hoist the fold into `sync_card_views`.
    fn absolute_group(
        &self,
        placed: &pack::Placed,
        sessions: &[SessionId],
        meta: Option<&GroupMeta>,
        now_ms: i64,
        cx: &mut Context<Self>,
    ) -> (Vec<AnyElement>, GroupChromeSnapshot) {
        let (fc, fr) = (placed.item.fc, placed.item.fr);
        let x = placed.cell_left();
        let y = placed.cell_top();
        let block_w = fc as f32 * CELL_W - GAP;
        let block_h = fr as f32 * CELL_H - GAP;

        let name = meta.map(|m| m.name.clone()).unwrap_or_default();
        let completed = meta.map(|m| m.completed_count).unwrap_or(0);
        let accent = group_accent(meta.and_then(|m| m.color_token.as_deref()));

        // Fold the rollup from member cards (snapshot the values we need — owned).
        let members: Vec<SessionCard> = {
            let fleet = self.fleet.read(cx);
            sessions
                .iter()
                .filter_map(|s| fleet.cards.get(s).map(|e| e.read(cx).clone()))
                .collect()
        };
        let member_refs: Vec<&SessionCard> = members.iter().collect();
        let rollup = rollup::group_rollup(&member_refs, completed);
        let header = rollup::group_header_text(&name, &rollup, now_ms);

        let snapshot = GroupChromeSnapshot {
            session_ids: sessions.to_vec(),
            name: name.clone(),
            accent,
            rollup: rollup.clone(),
            header: header.clone(),
        };

        let mut out: Vec<AnyElement> = Vec::with_capacity(sessions.len() + 2);

        // Ring + tint box in the gap (spec §3). Sibling of the member cards.
        out.push(
            div()
                .absolute()
                .left(px(x - INSET))
                .top(px(y - INSET))
                .w(px(block_w + 2.0 * INSET))
                .h(px(block_h + 2.0 * INSET))
                .rounded(px(12.0))
                .border_1()
                .border_color(accent)
                .bg(accent.opacity(0.07)) // SSOT color-mix ~7% body wash
                .into_any_element(),
        );

        // Header-lane (top HEADER-tall band): dot · name · spend · age · ✓N · caret.
        out.push(
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(block_w))
                .h(px(HEADER))
                .flex()
                .flex_row()
                .items_center()
                .gap_1p5()
                .px_1p5()
                .child(div().size(px(8.0)).rounded_full().bg(accent))
                .child(
                    div()
                        .text_color(gpui::rgb(0xd6d6de))
                        .child(name.clone()),
                )
                .child(
                    div()
                        .flex_grow()
                        .text_color(gpui::rgb(0x8a8a94))
                        .child(format!(
                            "{} · {}",
                            rollup::format_group_spend(rollup.spend_usd),
                            rollup::format_age(rollup.oldest_created_at, now_ms),
                        )),
                )
                .child(
                    div()
                        .text_color(gpui::rgb(0x8a8a94))
                        .child(format!("✓{completed}")),
                )
                .child(div().text_color(gpui::rgb(0x8a8a94)).child("⌄"))
                .into_any_element(),
        );

        // Members at full size in body-zones (unchanged geometry from B-2).
        for (i, session) in sessions.iter().enumerate() {
            let cc = i % fc;
            let rr = i / fc;
            let mx = INSET + cc as f32 * CELL_W;
            let my = INSET + HEADER + rr as f32 * CELL_H;
            if let Some(tile) = self.absolute_card(session, x - INSET + mx, y - INSET + my, cx) {
                out.push(tile);
            }
        }

        (out, snapshot)
    }
```

Add the `SessionCard` import to the top of `crates/lens-ui/src/board/mod.rs` (the file imports `SessionCardView` at line 4; add the model type):

```rust
use crate::card::model::SessionCard;
```

> **Note on `.gap_1p5()` / `.px_1p5()`:** if these gpui helpers don't exist in this gpui version, substitute `.gap(px(6.0))` and `.px(px(6.0))`. Verify at compile; the exact spacing is a tunable (spec §8).

- [ ] **Step 4: Compile and run the existing board tests**

Run: `cargo test -p lens-ui --lib board 2>&1 | tail -30`
Expected: PASS — `board::rollup` (Task 1), `board::tests::group_accent` (Task 3), and `board::layout_adapter` tests all green; no compile errors. (The group render path still has no dedicated integration test — that's Task 5.)

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/lens-ui/src/board/mod.rs
git commit -m "feat(board): render group ring/tint/header from rollup (B-3)"
```

---

### Task 5: gpui fixture-injection integration test

**Files:**
- Modify: `crates/lens-ui/tests/acceptance_shell.rs` (add one `#[gpui::test]` + any missing imports)

**Interfaces:**
- Consumes: `BoardView::set_test_layout_for_test`, `BoardView::group_chrome_for_test`, `BoardView::visible_session_ids_for_test` (Task 4), the `lens_core::domain::board` public build API.

- [ ] **Step 1: Write the failing integration test**

At the end of `crates/lens-ui/tests/acceptance_shell.rs`, add:

```rust
/// B-3: a group tile renders ring/tint/header chrome and a rollup folded from its
/// member cards. The group path is not runtime-reachable (no group is creatable
/// until B-4), so the test injects a hand-built `BoardLayout` via the test seam.
#[gpui::test]
async fn board_group_renders_chrome_and_rollup(cx: &mut gpui::TestAppContext) {
    use gpui::{Size, px};
    use lens_core::domain::board::{
        Board, BoardId, BoardItemId, BoardLayout, PlacementTarget, DEFAULT_BOARD_ID,
        DEFAULT_BOARD_NAME,
    };
    use lens_core::domain::ids::ConnectionId;

    // Clock at 2h past epoch so the oldest member (created_at=0s) ages to "2h".
    let clock = Arc::new(ManualUiClock::new(7_200_000));
    let s1 = SessionId::new("s1");
    let s2 = SessionId::new("s2");

    let (fleet, c1, c2) = cx.update(|cx| {
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);
        let fleet = FleetStore::new(Arc::clone(&clock) as Arc<dyn UiClock>, cx);
        let (c1, c2) = fleet.update(cx, |f, cx| {
            let c1 = f.spawn_fake_session(s1.clone(), cx);
            let c2 = f.spawn_fake_session(s2.clone(), cx);
            (c1, c2)
        });
        (fleet, c1, c2)
    });
    // Member card data: s1 = $1.50 @ 0s (oldest), s2 = $2.00 @ 100s.
    cx.update(|cx| {
        c1.update(cx, |card, _| {
            card.cumulative_cost.total_cost_usd = Some(1.50);
            card.created_at = Some(0);
        });
        c2.update(cx, |card, _| {
            card.cumulative_cost.total_cost_usd = Some(2.00);
            card.created_at = Some(100);
        });
    });

    // Hand-built layout: one blue group "Refactor" with members s1, s2.
    let board = BoardId::new(DEFAULT_BOARD_ID);
    let g1 = BoardItemId::new("g1");
    let mut layout = BoardLayout::default();
    layout.boards.push(Board {
        id: board.clone(),
        name: DEFAULT_BOARD_NAME.into(),
        ordinal: 0,
        created_at: 0,
        updated_at: 0,
    });
    layout
        .create_group(&board, None, 0, "Refactor", g1.clone(), 0)
        .unwrap();
    layout.set_color(&g1, "blue").unwrap();
    let conn = ConnectionId::new("c");
    let under_group = |ordinal: i32| PlacementTarget {
        board_id: None,
        parent_item_id: Some(g1.clone()),
        ordinal: Some(ordinal),
    };
    layout
        .place_session(conn.clone(), s1.clone(), &under_group(0), BoardItemId::new("c1"), 0)
        .unwrap();
    layout
        .place_session(conn.clone(), s2.clone(), &under_group(1), BoardItemId::new("c2"), 0)
        .unwrap();

    let fleet_for_window = fleet.clone();
    let (board_handle, vcx) = cx.add_window_view(|_, cx| {
        let working_tab = placeholder_tab(cx);
        BoardView::mount(fleet_for_window, working_tab, None, cx)
    });
    board_handle.update(&mut *vcx.cx.borrow_mut(), |b, _| {
        b.set_test_layout_for_test(layout);
    });
    vcx.simulate_resize(Size {
        width: px(1200.0),
        height: px(900.0),
    });
    vcx.run_until_parked();

    let chrome = vcx.read(|cx| board_handle.read(cx).group_chrome_for_test());
    assert_eq!(chrome.len(), 1, "exactly one group tile rendered");
    let g = &chrome[0];
    assert_eq!(g.name, "Refactor");
    assert_eq!(g.accent, gpui::rgb(0x4c8dff).into(), "blue token → SSOT blue");
    assert_eq!(g.rollup.spend_usd, Some(3.50), "spend sums members");
    assert_eq!(g.rollup.oldest_created_at, Some(0), "oldest member wins");
    assert_eq!(g.rollup.completed_count, 0, "✓N is 0 until B-6");
    assert_eq!(g.header, "Refactor · ~$3.50 · 2h · ✓0");
    assert_eq!(g.session_ids, vec![s1.clone(), s2.clone()]);

    // The member cards were built (in the visible band).
    let built = vcx.read(|cx| board_handle.read(cx).visible_session_ids_for_test());
    assert!(built.contains(&s1) && built.contains(&s2), "members built");
}
```

> **Note on the `set_test_layout_for_test` update handle:** the exact way to get a mutable `App`/`Context` to call `board_handle.update(...)` mirrors the existing tests in this file (they use `vcx` for reads and `cx.update`/window dispatch for writes). If `&mut *vcx.cx.borrow_mut()` is not the pattern this harness uses, follow the write pattern already used by `back_to_board_*` / resize tests in this file (e.g. a `vcx.update(|_, cx| board_handle.update(cx, |b, _| …))` form). The requirement is only: inject the layout **before** `simulate_resize` triggers the render whose snapshot you assert.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p lens-ui --test acceptance_shell board_group_renders_chrome_and_rollup 2>&1 | tail -30`
Expected: FAIL initially only if a hook/signature is wrong; if Tasks 1-4 are complete it should compile. If it fails on an assertion, read the actual vs expected and fix the wiring (not the test's expected values, which are derived from the fixture).

- [ ] **Step 3: Make it pass**

If the test fails to compile on the update-handle pattern, adjust per the Step 1 note to match the file's existing write pattern. If an assertion fails, trace back: `spend_usd` wrong → check the fold reads `total_cost_usd`; `header` wrong → check `group_header_text` order; `accent` wrong → check `group_accent`. Do not weaken the assertions.

Run: `cargo test -p lens-ui --test acceptance_shell board_group_renders_chrome_and_rollup 2>&1 | tail -30`
Expected: PASS.

- [ ] **Step 4: Full lens-ui test sweep**

Run: `cargo test -p lens-ui 2>&1 | tail -30`
Expected: PASS — all lens-ui unit + acceptance tests green (no regression in the B-2 culling/back-to-board tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/lens-ui/tests/acceptance_shell.rs
git commit -m "test(board): B-3 group chrome + rollup fixture-injection test"
```

---

### Task 6: Gate + reviewer diversity

**Files:** none (verification only)

- [ ] **Step 1: Run the real gate**

Run: `cargo run -p xtask -- gate 2>&1 | tail -30`
Expected: "all checks passed" — zero clippy warnings, `fmt --check` clean, all workspace `-p` crates' tests green, no contract drift. (Do NOT pipe through `tail` in a way that hides the exit code — check the final status line. The `spikes/board-container` clippy red is pre-existing and excluded from the gate; ignore it.)

- [ ] **Step 2: Cross-family review of the branch diff**

Per project rule, get ≥1 review from a non-authoring model family. Route gpt-5.6 through codex:

Run: `git diff main -- crates/lens-ui/src/board/ crates/lens-ui/src/card/model.rs crates/lens-ui/tests/acceptance_shell.rs > /tmp/b3.diff` then have a reviewer (e.g. `codex exec -s read-only`) check: rollup fold correctness, the seconds-vs-millis age conversion, the `.cached()` read-during-render note, dead-code on the test seam, and spec §3/§3.1 fidelity.

- [ ] **Step 3: Address review, re-gate, and update docs**

Fold any confirmed findings, re-run the gate, then update `docs/STATUS.md` (B-3 → shipped, B-4 next) and the memory index per the end-of-session convention.

---

## Self-Review

**1. Spec coverage** (`docs/specs/2026-07-20-board-packing-and-group-rendering-design.md` §3, §3.1, §7):
- §3 ring + tint → Task 4 (`border_color(accent)` + `bg(accent.opacity(0.07))`). ✓
- §3 header-lane `● dot · name · [spend · age] · ✓N · ⌄` → Task 4 header render + Task 1 `group_header_text`. ✓
- §3 `✓N` = completed count, deferred source → Task 4 `completed_count: 0` + Global Constraints note (B-6 wires). ✓
- §3 members = identical compact card chrome → unchanged `absolute_card` reuse in Task 4. ✓
- §3.1 spend = Σ cumulative_cost → Task 1 `group_rollup`. ✓
- §3.1 age = oldest member created_at → Task 2 plumb + Task 1 `format_age`. ✓
- §3.1 status rollup (collapsed tile) → **out of scope** (spec §1: collapsed render is B-4; §7 geometry captured there). Not a B-3 gap.
- §7 collapsed tile → explicitly B-4 (spec §1). Not in this plan by design.

**2. Placeholder scan:** No "TBD"/"add error handling"/"similar to Task N". Two `> Note` blocks flag version-dependent gpui helpers (`.gap_1p5()`) and the test write-handle pattern — these are honest "verify at compile, here's the fallback" notes with concrete substitutes, not placeholders.

**3. Type consistency:** `GroupRollup` fields (`spend_usd`, `oldest_created_at`, `completed_count`) are used identically in Tasks 1, 4, 5. `group_rollup(members: &[&SessionCard], completed_count: u32)` signature matches all call sites. `group_accent(Option<&str>) -> Hsla` consistent. `GroupChromeSnapshot` fields match between Task 4 (produce) and Task 5 (assert). `created_at: Option<i64>` consistent between Task 2 (add) and Task 1 (read).

**Known cross-task dependency:** Task 1's fold reads `SessionCard.created_at`, added in Task 2 — the ordering note in Task 1 tells the implementer to add Task 2's field first. Flagged, not a bug.
