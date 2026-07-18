# Handoff — Board B-1 (data model & persistence) executed

**Written:** 2026-07-18 · **Branch:** `main` · **HEAD:** `8100cc8` (committed, **UNPUSHED** — main ahead of origin by 4)
**Spec:** `docs/specs/2026-07-18-board-data-model-persistence-design.md` (user-approved)
**Memory:** [[board-b1-executed]]

## TL;DR

B-1 — the **keystone** of the six-way §4 board decomposition — is **code-complete, reviewed, and committed to `main`** (`8100cc8`, not pushed). It adds the pure `BoardLayout` tree and the write-through `SqliteBoardStore` to `lens-core` only. Adversarial review (grok-4.5 + my own trace + probe tests) found **no blocking bug**; grok's one HIGH finding was refuted empirically. Five low/defensive hardening items were applied on top. `cargo run -p xtask -- gate` is green.

**This session picked up a prior session that died mid-fix.** The in-flight fixes (cross-board subtree carry, ungroup slot-splice) had actually landed correctly — only a dirty `cargo fmt` was outstanding.

**Verify green before any follow-up:**
```bash
cargo run -p xtask -- gate     # fmt + workspace clippy (default+demo) + tests + drift. Do NOT pipe through tail.
```

## What shipped (all in `crates/lens-core/`)

- **`src/domain/board.rs`** (new) — pure, DB-free: `Board`, `BoardItem`, `BoardItemKind::{Card,Group}`, `BoardLayout`, `PlacementTarget`, `BoardError`. Ops: `place_session`, `remove_session`, `create_group`, `move_item` (cross-board subtree carry + cycle guard), `ungroup` (children spliced at the group's slot), `rename`/`archive`/`set_collapsed`/`set_color`, `children`, dense ordinal renumbering.
- **`src/persist/board.rs`** (new) — `SqliteBoardStore` + `BoardStore` trait. Write-through, transactional. Bidirectional startup reconcile (lazy-place live sessions, prune tombstoned; **absence ≠ delete**). Corrupt-row skip, read-only-degraded write refusal.
- **`src/persist/schema.rs`** — `SCHEMA_VERSION` **2→3** (additive: `boards`, `board_items` + unique/parent indexes). `src/domain/ids.rs` — branded `BoardId`, `BoardItemId`. `src/{domain,persist}/mod.rs` + `lib.rs` — module wiring + re-exports. `src/persist/transcript.rs` — one test assert switched to the `SCHEMA_VERSION` symbol.
- **`docs/SPEC-GAPS.md`** — B-1 marked spec-written (was already in the prior commit `c855ab6`).

## Review outcome (grok-4.5 + own trace + 6 probes)

- **No blocking correctness bug.** Ordinal density, cross-board move persistence, `ungroup` × `ON DELETE CASCADE` ordering (verified `foreign_keys=ON`; reparent children in DB **before** deleting the group row), unique-index handling, atomic reconcile — all traced clean by two independent reviewers.
- **Grok's "HIGH: `COUNT(*)`-seeded id collision" — REFUTED.** Forced the worst case (3 cards minted same-ms, delete first, reopen): `seq` *is* reused, but the fresh `ms` suffix keeps the full `item_id` unique. Grok assumed the timestamp was reused on remint; it isn't.
- **Codex (3rd family) hung** ~25 min with zero output — killed. grok (non-Claude) already satisfied the review-diversity bar.

## Hardening applied on top (5, all low/defensive)

1. `item_id` `seq` seeded from the **high-water mark** (max embedded seq), not `COUNT(*)` — never reused across delete+reopen. Misleading comment corrected.
2. `place_session` **refuses a tombstoned session** (`is_tombstoned` guard) — no transient card lingering until next load.
3. `reassign_subtree_board` gains a **`seen`-guard** — can't hang on a corrupt cyclic parent graph.
4. Reconcile live-session query is **`ORDER BY created_at, id`** — deterministic loose-append on batch/upgrade placement.
5. **Test-gap closed:** reconcile's prune-of-tombstoned branch was never exercised (old test removed the card first). Added that + 6 folded probes (nested-ungroup persist, remove-middle renumber, move-out-of-group renumber, place-explicit-ordinal, id-seq-no-reuse regression, place-tombstoned-noop). **30 board tests** (was 23).

## Next up — B-2 (packing / scroll / culling)

Dependency order for the rest is in `docs/SPEC-GAPS.md` → "Board (§4) implementation specs". B-2 is next. Key seams B-1 deliberately left open (all recorded in [[board-b1-executed]]):

- **`board_tree(board_id)` ordered-walk / visitor read-API is NOT exposed** — B-1 has only `children(board_id, parent)`. B-2's packer needs the walk; add it then (spec §7).
- **`lens-ui` `BoardView` is NOT wired to `BoardLayout`** — B-1 is `lens-core` only. The UI still uses its placeholder ordering; wiring the gpui replica (spec §6: `BoardLayout` in an `Entity`, UI mutations call `BoardStore` + `cx.notify()`) rides with B-2/B-4.
- **Viewport-freeze heads-up is now B-2's** — the scroll container is a different off→on transition than focus↔board; the current edge-based gate reset won't fire for a card scrolling back into view. See STATUS "Next up" + memory [[viewport-reentry-freeze]].
- **Board CRUD → B-5** (only the default board is seeded; cross-board `move_item` is unit-tested but unreachable at runtime until B-5). **Cost derived at render → B-3.** **FleetStore conn-scoping → B-5.**

## Open decisions for the user

1. **Push?** `main` is ahead of `origin/main` (`b8727ab`) by 5 unpushed commits (`759eb3a`, `c855ab6`, `c21e669`, `8100cc8`, + this handoff's docs-status commit). Solo-project convention is merge-to-main; push is a separate call — deferred to you.
2. **B-2 now, or the other "Next up" thread** (`lens-ui` transcript fan-out)? Terminal Slice 2 is owned by a separate agent on `terminal-ws` — don't double-drive.
