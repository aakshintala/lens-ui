# Spike — large-transcript latency (P3 Task 0 / D12)

**Date:** 2026-07-09
**Verdict:** **The D11 bimodal / byte-window premise HOLDS**, and the D12
"real unknown" resolves decisively: **reconcile MUST bound to the reconnect
tail.** Full-history reconcile-by-id at 100k rows costs **~1.06 s** (p50); the
same operation scoped to the last 50–500 items costs **0.34–2.85 ms** — a
**370×–3100× gap**. Page-load and cold-hydrate are both comfortably inside their
expected envelopes (sub-ms and ~4.9 ms respectively). Everything else in P3-3
(sleep flush, wake disk-paint, reconnect reconcile) can be built on the two
prototyped primitives below.

Design source: `docs/superpowers/specs/2026-07-08-state-model-engine-design.md`
(§2.1 **D11** byte-windowed transcript, **D12** this spike; §4 "P3 → Task 0").
Harness (throwaway): `spikes/large-transcript/` (outside the lint wall; generated
`.db` gitignored, ~515 MiB).

---

## Hardware, build, methodology

- **Hardware:** Apple M3 Pro, 11 cores, 18 GiB RAM, built-in SSD. macOS 26.5.1.
- **Toolchain / profile:** rustc 1.91.1, `cargo build --release` (optimized).
- **SQLite:** `rusqlite` 0.32 **bundled**, WAL journal (matches the shipped
  `TranscriptStore`). `synchronous=NORMAL` for generation only.
- **Real code under test:** the harness `path`-depends on `lens-core` and
  exercises the **shipped** `Item`/`ItemKind` serde, `SqliteTranscriptStore::{
  load_items, reconcile}`, and `persist::map::row_to_item`. Only the primitives
  that *don't exist yet* (windowed page-load, byte-budgeted tail, tail-bounded
  reconcile) are prototyped over a raw `rusqlite::Connection`.
- **Cache discipline:** every measurement is **warm-cache** — a full
  `COUNT(*)` pulls all pages through the OS cache once, then each benchmark runs
  5 warmups before timing. (Cold-cache first-focus after a laptop resume would
  be slower; warm is the steady-state a warm/slept session lives in. Flagged as
  a risk below.)
- **Sampling:** p50/p90 over **30 iters** for the sub-ms measurements; the
  full-history reconcile (which rewrites 100k rows + ~500 MiB per run) is 5 iters.

### Synthetic corpus (the D11 premise, materialized)

| property | value |
|---|---|
| rows (items) | 100 000 |
| large "dumps" | 2 565 (**2.56 %**), `FunctionCallOutput`, 200 000 B text each |
| small "markers" | 97 435, `Message`, 60 B text (~130 B on-wire `ItemKind` json) |
| total payload | 527 543 535 B (**503.1 MiB**) |
| on-disk file | 539 709 440 B (**514.7 MiB**) |
| generation time | **1.59 s** (single batched txn) |

**Note on the mix:** D11 names *both* "~5 % large" *and* "~200 KB dumps" *and*
"~500 MiB / 100k items" — these are over-determined (5 % × 200 KB alone is
~1 GiB). The **dump size** (200 KB) is the parameter the byte-window math turns
on, so the harness holds it fixed and lets the fraction fall out of the
500 MiB / 100k target → **2.56 %** large. Conclusions are insensitive to this
choice; a 5 % / 100 KB corpus lands on the same page/tail/reconcile shape.

**Generation caveat:** the shipped `upsert_item` autocommits **one txn per row**
(a WAL fsync storm — pathological for 100k rows). Generation therefore uses a
single batched raw transaction with identical payload encoding. This is a
generation-only shortcut; it does **not** affect any measured latency. It is a
mild signal that P3's write-through path wants batching if bulk backfill ever
runs on the hot path (steady-state write-through is one item at a time, fine).

---

## Results

### M1 — Windowed page-load (scroll-back), page from mid-history (ordinal ~50 000)

| page size | p50 | p90 | items |
|---|---|---|---|
| 50 items | **117 µs** | 130 µs | 50 |
| 200 items | **379 µs** | 390 µs | 200 |
| ~512 KiB byte-budget | **327 µs** | 334 µs | 79 |

Expectation was ~1–10 ms/page. Actual is **sub-millisecond** at every page size,
because `WHERE ordinal < ? ORDER BY ordinal DESC LIMIT ?` rides the
`UNIQUE(ordinal)` index and touches only the page's rows. Scroll-back is a
non-issue; a prefetch-one-page-ahead policy (D11) has enormous headroom.

### M2 — Byte-budgeted cold-hydrate tail (~8 MB, Slept→focus)

| metric | value |
|---|---|
| p50 / p90 | **4.88 ms** / 4.93 ms |
| 8 MB buys | **1 564 items** (41 large dumps + 1 523 small) |
| decoded | 8.0 MiB |

Expectation was ~5–20 ms. Actual **4.9 ms**, at the *fast* end. The
`ORDER BY ordinal DESC` cursor breaks early once the byte budget is hit, so it
reads ~1 564 tail rows, **not** the 100k-row / 500 MiB history (a full
`load_items()` is 357 ms — 73× slower — confirming the early break works and the
index is used in reverse). **8 MB ≈ ~1 500 items** under this mix — a healthy
resident window, and the count backstop (D11) only matters if a session goes
long stretches with *no* dumps.

### M3 — Reconcile-by-id: scope is everything

**(a) Full-history (shipped `reconcile`, 100k rows):**

| metric | value |
|---|---|
| p50 / p90 | **1 062 ms** / 1 179 ms |
| (baseline: `load_items()` all 100k) | 357 ms |

**(b) Tail-bounded (prototype, reconcile only ordinals ≥ tail_start):**

| tail size | p50 | p90 |
|---|---|---|
| 50 items | **342 µs** | 365 µs |
| 200 items | **1.29 ms** | 1.63 ms |
| 500 items | **2.85 ms** | 3.86 ms |

**The gap: 370× (tail-500) to ~3 100× (tail-50).** Full-history reconcile is
O(transcript): the shipped impl parks *all* 100k ordinals negative, re-upserts
*all* 100k rows (re-serializing ~500 MiB of payload, including every 200 KB
dump), then deletes untouched rows — all in one transaction. At 100k rows that
is **>1 second of blocking work**. Tail-bounded touches only the reconnect
window, and its cost scales with the tail's *byte* content (how many dumps land
in the last N items), not the history depth.

---

## Recommended contract: **reconcile-scope = bounded tail, never full history**

**Contract for P3-3 wake/reconnect:**

> On wake/reconnect, reconcile-by-id **only over the tail since
> `last_seen_seq`** — never over the full transcript. The disk history below the
> tail is immutable-by-assumption between the last seen ordinal and now; server
> compaction/edits that reach below the tail are out of scope for the
> reconnect-reconcile and are handled lazily on scroll-back (re-fetch the page,
> reconcile that page).

Justification is the M3 gap: a full-history reconcile on every wake would put a
**~1 s foreground-visible stall** (or a 1 s actor-thread block that backs up the
event channel) on any multi-day session. The tail-bounded path keeps wake
reconcile in the **single-digit-millisecond** range, matching page-load /
hydrate. This is the D12 "real unknown", resolved: **bounded tail wins by 2.5–3
orders of magnitude; there is no regime where full-history reconcile is
acceptable at scale.**

**Cursor + pagination shape this implies (entangles the deferred `GET /items`
pagination contract, plan 3b-2b):**

- The actor already persists **`last_seen_seq`** (control-store column exists).
  Wake uses it as the reconcile floor: fetch server truth for items **after**
  `last_seen_seq` only.
- `GET /items` therefore needs a **`since`/`after` + `limit`** (or opaque cursor)
  pagination parameter so the client can pull *just the tail*, not the whole
  conversation. **This is the blocking dependency to lift in P3-3** — without
  server-side tail pagination, the client would have to `GET` the full history to
  reconcile, re-introducing the O(transcript) cost on the *network* side (worse
  than the DB cost measured here). Flagged for plan 3b-2b.
- Reconcile floor uses **`ordinal`** on the local side (the DB primitive keys on
  ordinal ranges) and **`last_seen_seq`** on the wire side; the mapping between
  them is the actor's canonical `Vec<Item>` tail. If a compaction below the tail
  shifts ordinals, the tail-bounded reconcile still self-heals within its
  range; deeper drift surfaces on scroll-back re-fetch.
- **Tail width:** 50–500 items all stay ≤ ~3 ms. Recommend keying the tail to
  the **same ~8 MB byte budget** as the resident window (≈1 500 items ≈ ~3–4 ms
  worst case) so wake reconcile ⊆ resident tail — no disk read below the window
  on the reconcile path.

---

## Prototyped primitive shapes (for P3-3 to add to `TranscriptStore` for real)

### 1. Windowed page-load (scroll-back)

```rust
/// One page of the `limit` items immediately BEFORE `before_ordinal`,
/// decoded ascending. Rides the UNIQUE(ordinal) index; touches only the page.
fn load_page(before_ordinal: i64, limit: i64) -> Loaded<Item>;
```
```sql
SELECT item_id, live_seq, kind, payload, agent, depth, turn, created_at
FROM items
WHERE ordinal < ?before_ordinal
ORDER BY ordinal DESC
LIMIT ?limit;         -- caller reverses to ascending for display
```
Column order MUST match `row_to_item`. Reuse `collect_skipping` so a corrupt
page row is skipped-and-reported (same contract as `load_items`), not fatal.

### 2. Byte-budgeted cold-hydrate tail (Slept→focus, and the reconcile floor)

```rust
/// Newest→oldest until cumulative payload bytes exceed `budget` (with a
/// count backstop). Early cursor break → reads only the tail, not history.
fn load_tail_by_bytes(budget: usize, max_items: usize) -> (Loaded<Item>, /*bytes*/ usize);
```
```sql
SELECT item_id, live_seq, kind, payload, agent, depth, turn, created_at,
       length(payload)
FROM items
ORDER BY ordinal DESC;   -- iterate; accumulate length(payload); break at budget
```
Do **not** express the budget as a SQL window-function running-sum — that scans
the whole table (O(transcript)). The early-break cursor is what makes it
tail-only (measured: ~4.9 ms vs 357 ms full).

### 3. Tail-bounded reconcile (wake/reconnect)

```rust
/// Reconcile-by-id ONLY over ordinals >= tail_start against `tail_truth`
/// (upsert at ordinal = tail_start + i; delete tail rows absent from truth).
/// Mirrors the shipped `reconcile`'s park-negative / re-stamp / delete-untouched
/// transaction, scoped to the tail. Never touches history below tail_start.
fn reconcile_tail(&self, tail_start: i64, tail_truth: &[Item]) -> Result<()>;
```
```sql
BEGIN;
UPDATE items SET ordinal = -1 - ordinal WHERE ordinal >= ?tail_start;  -- park tail only
-- for each (i, item) in tail_truth: upsert ON CONFLICT(item_id) at ordinal = tail_start + i
DELETE FROM items WHERE ordinal < 0;   -- drop tail rows the truth didn't touch
COMMIT;
```
`tail_start` = the ordinal of `last_seen_seq`'s item (or the resident-window
floor). The existing full-history `reconcile` stays as-is for the *first ever*
hydrate of a fresh/small session; the tail variant is the **wake/reconnect**
path.

---

## Did the D11 premise hold? **Yes.**

| claim (D11) | evidence |
|---|---|
| Sessions reach ~500–600 MiB / ~100k items | corpus built at 503 MiB / 100k, on-disk 515 MiB |
| Items are **bimodal** (100 B markers vs 200 KB dumps) | modeled exactly; the mix is what makes byte-budgeting (not count) correct |
| Resident window must be **byte-sized, not count-sized** | 8 MB buys 1 564 items *here*, but a dump-heavy stretch would buy far fewer — a fixed item-count window would blow the RAM budget on dumps or starve on markers |
| Full history on disk, lazy page-in on scroll-back | page-load 117–379 µs — trivially cheap, prefetch has headroom |
| Keeps fleet RAM flat (~8 MB/warm session) | 8 MB tail = 1 564 items decoded; 30 warm × 8 MB ≈ 240 MB regardless of session age ✔ |

---

## Implications / risks for P3-3

1. **Wake reconcile MUST be tail-scoped (blocking finding).** Full-history
   reconcile at scale is a ~1 s stall. Build the tail-bounded primitive; do
   **not** call the existing full `reconcile` on wake of a large session.
2. **`GET /items` tail pagination is now on the critical path.** The
   deferred-from-3b-2b pagination contract (`since`/`after` + `limit`/cursor) is
   a **prerequisite** for tail-scoped reconcile — otherwise the network fetch
   re-introduces the O(transcript) cost the DB side just eliminated. Lift it in
   P3-3.
3. **Sleep flush is cheap.** Steady-state write-through is one `upsert_item` per
   item; the only O(transcript) operation in the shipped surface is `load_items`
   (357 ms) and full `reconcile` (1 s) — **neither should run on sleep**. Sleep
   should flush the resident tail (already persisted by write-through) and drop
   RAM; nothing to rewrite.
4. **Cold-cache first-focus risk (unmeasured).** All numbers are warm-cache. A
   focus immediately after laptop resume / a just-opened file pays OS-page-cache
   misses over the tail (~8 MB) — likely tens of ms, still fine, but the
   scroll-back-into-cold-history case wants the off-thread + prefetch-ahead
   policy D11 already calls for. Worth a cold-cache confirmation before shipping
   if any focus path is synchronous on the foreground.
5. **Write-through batching (minor).** Bulk backfill via per-row-autocommit
   `upsert_item` is a fsync storm (generation needed one batched txn). If P3 ever
   backfills history from `GET /items` in bulk (e.g. first hydrate of a large
   remote session), wrap it in one transaction. Steady-state single-item
   write-through is unaffected.
6. **`load_items()` is not the hydrate path.** At 357 ms it must never be the
   Slept→focus load; use `load_tail_by_bytes`. `load_items()` should be reserved
   for small sessions / tests / migration, or deprecated in favor of the paged
   loaders.

---

## Reproduce

```
cargo run --release -p large-transcript          # generate (if missing) + bench
cargo run --release -p large-transcript -- gen    # (re)generate only (~1.6 s, writes ~515 MiB)
cargo run --release -p large-transcript -- bench   # bench an existing db
```
The `.db` lands in `spikes/large-transcript/data/` (gitignored).
