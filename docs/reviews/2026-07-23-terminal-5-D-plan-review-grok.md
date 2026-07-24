# Plan review — Terminal Slice 5 Sub-slice D (fleet-integration)

**Reviewer:** grok-4.5 (adversarial, source-checked)  
**Plan:** `docs/plans/2026-07-23-terminal-slice-5-D-fleet-integration.md`  
**Design SSOT:** `docs/specs/2026-07-22-terminal-slice-5-fleet-membership-design.md` (§3 D row, §4.1–4.2, §9, §10, §13, §14)  
**Handoff context:** `docs/handoffs/2026-07-23-terminal-slice-5-A-executed.md`  
**Scope:** plan-only review against design + actual source. No implementation.

---

## Critical

### C1. `session_loader` field as written is invisible to Task 5 code in `terminal.rs`

**Where:** Plan Task 3 (`store.rs` field) + Task 5 (`terminal.rs` `on_supersede` uses `self.session_loader`).

**Problem:** `FleetStore` is defined in `crates/lens-ui/src/fleet/store.rs`. Sibling module `terminal.rs` already can touch `self.terminals` only because that field is explicitly `pub(crate)` (`store.rs:71-72`). `cards` is `pub`. Task 3 adds:

```rust
session_loader: Option<std::rc::Rc<dyn …SessionLoader>>,
```

with no visibility — i.e. private to `store`. Task 5’s `on_supersede` then does `self.session_loader.clone()`. That is a hard compile error: private fields are not visible across modules, even in `impl FleetStore` blocks living in `terminal.rs`.

**Failure:** Task 5 does not compile.

**Fix:** Declare the field `pub(crate) session_loader: …` (same pattern as `terminals`), or move `on_supersede` / `complete_supersede` into `store.rs`.

---

### C2. `FakeSessionLoader::load` re-enters `FleetStore::update` while Task 5 already holds `&mut FleetStore`

**Where:** Plan Task 3 Fake (`loader.rs`) + Task 5 `on_supersede` + Task 5 tests that call `on_session_control(Superseded)` inside `store.update`.

**Problem:** Task 5 tests (and the production call path via the poller) invoke:

```rust
store.update(cx, |store, cx| {
    store.on_session_control(… Superseded …, cx);
});
```

`on_supersede` then calls `loader.load(..., cx.entity().downgrade(), &mut *cx)` **before** releasing the entity borrow. The planned Fake does a **synchronous**:

```rust
store.update(cx, |store, cx| {
    store.spawn_fake_session(session_id, cx);
});
```

That is a nested `WeakEntity<FleetStore>::update` on the entity already being updated. gpui entity updates are not re-entrant; this panics / returns `Err` at runtime. The “happy path” supersede test therefore cannot pass as written, and any future sync loader would hit the same trap.

(The real `AppSessionLoader` avoids the sync nest by only `cx.spawn`ing, but the Fake — which is what proves Tasks 3/5 — does not.)

**Failure:** `supersede_loads_b_moves_member_and_drives_transfer` panics or treats load as failed → member never moves / no `Transfer`; Task 5 is false-red or false-green depending on how the `Err` is handled.

**Fix (pick one, plan must pick):**
1. Make `FakeSessionLoader::load` schedule `spawn_fake_session` on `cx.spawn` / `background` and return **that** `Task` (never sync-update inside `load`), matching the real loader’s async shape; or
2. Restructure `on_supersede` so `loader.load` is only invoked from a `cx.spawn` after the outer `&mut self` borrow ends (clone loader + ids, spawn, then load+complete inside the task).

Also add an explicit note in the plan: **no `SessionLoader` impl may call `store.update` synchronously from `load` when invoked under an active `FleetStore` update.**

---

## Important

### I1. No in-flight / staleness guard on async supersede

**Where:** Plan Task 5 `on_supersede` / `complete_supersede`; design §10 step 1–3; SessionLoader soundness.

**Problem:** While `loader.load` is in flight, the plan allows:
- a second `Superseded` for the same A→B (no `loading` set) → double GET/seed/spawn;
- completion after A’s terminals were `cascade_end` / `close_terminal`’d → `move` returns empty (benign) but B may still have been spawned as a side effect of a load that is no longer needed;
- completion after a later supersede retargeted elsewhere (not modeled today, but the seam has no generation/token).

`complete_supersede` only checks “moved empty?”; it does not re-validate that this completion still corresponds to the initiating supersede, or that `from` still owns the members that motivated the load.

**Failure:** Duplicate live sessions / wasted work; in pathological replay, Transfer/move ordering races. Design §14 risk (“Headless load-of-B”) is only half-closed.

**Fix:** Add a per-`from` (or `(from,to)`) in-flight token / `supersede_epoch` on `FleetStore`. `on_supersede` bumps it; the spawned completion closure captures the epoch and no-ops unless it still matches **and** `terminals.get(from)` is still non-empty (or equals the pre-load key set). Skip starting a new load when an identical in-flight load exists.

---

### I2. Task 6 wiring site / variable names do not match `main.rs` as it exists

**Where:** Plan Task 6 Step 3 cites “after the store is built at ~`:104`” and shows `config.data_dir`.

**Actual source:**
- `FleetStore::new_live` is at `main.rs:97` (outside the window).
- The spawn loop is inside `cx.open_window` at `main.rs:100-114`, gated on `if let Some(prep) = live_prep`.
- Inside that closure the data dir is `prep.data_dir`, not `config.data_dir`. `conn` is `prep.conn`.

**Failure:** Executor pastes the snippet, misses `prep`, fails to compile, or wires the loader only in the wrong scope (e.g. never set when sessions actually spawn).

**Fix:** Rewrite Step 3 against the real shape: create `AppSessionLoader` from `prep.conn` + `prep.data_dir` inside the `if let Some(prep)` block **before** the `for sid in prep.session_ids` spawn loop; keep `set_session_loader` on that same `fleet.update`.

---

### I3. `AppSessionLoader` import of `GetOpts` as written will not compile

**Where:** Plan Task 6 loader snippet: `use lens_client::{Client, Connection, GetOpts};`

**Actual:** `GetOpts` lives at `lens_client::sessions::GetOpts` and is **not** in `lens_client`’s crate-root `pub use` list (`crates/lens-client/src/lib.rs`). `main.rs` already imports it via `lens_client::sessions::{GetOpts, …}`.

**Failure:** Task 6 fails to compile until imports are fixed. Plan’s “adjust imports” hedge is too soft for a load-bearing snippet.

**Fix:** Pin the import to `lens_client::sessions::GetOpts` (and `SessionSnapshot` if needed) in the plan text.

---

### I4. Design §13 D proof obligation for “both orders → adoption” is only partially resolved

**Where:** Design §3 D row + §9 + §13; Plan “Resolved planning question 4” + Task 2/7.

**Problem:** Design §13 explicitly lists under D: “`4404`-first driving (both orders → adoption fires)”. The plan correctly notes there is **no public lens-ui seam to synthesize a bridge `4404`/`TerminalNotFound`** (`apply_bridge_event` is private; `live_tab_for_test` is crate-private in `lens-terminal`). That part of the claim is true.

But the plan oversells the conclusion. The **resource.deleted-first → created → adopt** order is in principle driveable at lens-ui **if** the tab has `generation` + `current_tid` set — `on_resource_signal` early-returns when `generation` is `None` (`lib.rs:1917-1918`), which is exactly what `open_with_engine_for_test` / `spawn_tab_with_rows` produce (`current_tid: None`, `generation: None` at `lib.rs:613-615`). So D’s Task 2 tests only prove **recorder-level forwarding**, not adoption. That is fine for forwarding fidelity, but it means §13’s “adoption fires” bullet is entirely deferred to A’s e2e + Task 7 rider with no intermediate lens-ui proof and no new test-util identity seam.

**Failure:** Spec coverage hole: a regression that forwards host events but breaks the FleetStore→tab identity path for a bound tab would not be caught by D’s unit tests.

**Fix (plan must choose one and write it down):**
- (Preferred) Add a small `test-util` seam on `TerminalTab` (e.g. `bind_identity_for_test(session, tid, key)`) and one lens-ui test: forward Deleted+Created to a bound Live tab → lifecycle reaches adopt path; keep 4404-first as A + live rider; **or**
- Explicitly amend the design §13 D bullet in the task-1 commit / handoff to “forwarding fidelity only; adoption remains A e2e + live rider” so reviewers stop expecting both-order adoption at lens-ui.

---

### I5. Cited line numbers — several are off or imprecise (load-bearing claims still mostly true)

Checked against HEAD:

| Plan claim | Actual | Verdict |
| --- | --- | --- |
| `FleetStore` struct `store.rs:59-77`, no `Connection`/`Client`/`data_dir` | Struct is `:59-77`; no those fields | **Accurate** |
| `spawn_fake_session` `:325` | `:325` | **Accurate** |
| `spawn_live_session` `:353` | `:353` | **Accurate** |
| poller outcome routing `~85-150` | match `:90-108`; `apply_outcome` control no-op `:145-148` | **Accurate** |
| `TerminalMember` `:24`, `open_terminal` `:59`, `set_terminal_visible` `:73`, `register…` `:232`, `on_terminal_presentation_changed` `:268`, helpers `:389-413`, tests `:415+` | All match | **Accurate** |
| subscription captures session at `:241-247`; early-return `:274-280` | Exact | **Accurate** (known trap is real) |
| `TerminalHostEvent` `:378`, `open_with_engine_for_test` `:637`, `retained_bytes_estimate` `:656`, `on_host_event` `:663`, Transfer `:670` | Enum `:378-380`; open_with… `:637-644`; estimate `:656`; `on_host_event` `:664`; Transfer arm `:670-674` | **Mostly accurate** (off-by-one on `on_host_event`) |
| `TerminalHostEvent` derives `Clone`+`Debug` `:378-380` | `#[derive(Clone, Debug)]` at `:379` | **Accurate** |
| scheduler `load_session` / `SessionNotFound` `:103-105` | `:102-105` | **Accurate enough** |
| `main.rs` spawn `~:104` | store `:97`; spawn loop `:102-114` | **Imprecise** (see I2) |
| startup GET+seed `:454-459` | `:454-459` | **Accurate** |
| `seed_disk` `:632-644` / Task 6 `:632` | `:632-645` | **Accurate** |
| `Sessions::get` `:1281` | Doc+fn at `:1281-1282` | **Accurate enough** |
| `fleet_verify.rs:73` calls `spawn_live_session` headlessly | Call inside update at `:72-74` | **Accurate enough** |
| poller “today both fall into `other =>`” `:145-148` | They hit `other =>` at `:100-107`, which calls `apply_outcome`, whose arms at `:145-148` no-op them | **Wording slightly wrong** — the no-op is not the poller match arm; the poller still card-routes them today |

**Failure:** Executor/reviewer trust erosion; the poller wording could cause someone to “fix” the wrong arm.

**Fix:** Correct the poller description to: “today both variants fall into poller `other =>` (`:100-107`) and are no-oped inside `apply_outcome` (`:145-148`)”. Fix Task 6 site as in I2.

---

### I6. map_item / §4.2 “NOT needed” claim — accepted, with one caveat

**Where:** Plan resolved Q3; design §4.2.

**Problem:** Design already says the persisted-item path is likely unnecessary because Q8 drives `Transfer` from `Superseded`. Plan’s decision matches. Caveat: if live omnigent ever delivers supersede **without** a usable in-memory `Superseded` outcome (e.g. client reconnects mid-rotation and only sees B’s snapshot items), D as planned cannot discover the transferred terminal via snapshot. That is an accepted product risk already latent in Q8, not a plan bug — but Task 7’s supersede rider must explicitly confirm the live event order still includes `session.superseded` on A.

**Failure if rider skips event-order check:** silent production hole with no map_item fallback.

**Fix:** Task 7 Step 2 — assert the live event sequence includes `superseded` on A (not only “scrollback survived”).

---

## Minor

### M1. Task 4 rebinding test is valid (not a false-pass) — plan’s soft-spot note is overstated

**Where:** Plan Task 4 warning + Self-review soft spot.

**Actual:** `with_engine_for_test` sets `lifecycle: Live` / `presentation.lifecycle: Live` (`lib.rs:587-590`). `is_sleepable` includes `Live` (`terminal.rs:47-51`). Existing C test already uses the exact `cx.emit(TerminalEvent::PresentationChanged)` pattern (`terminal.rs:745-747`) and asserts `pending_sleep` clears. Test helpers `insert_terminal_for_test` / `terminal_member_for_test` / `set_member_pending_sleep_for_test` signatures match the plan’s calls.

**Failure:** None if implemented as written — but the plan tells the executor to “confirm before trusting,” which invites an unnecessary redesign.

**Fix:** Replace the soft-spot with: “Confirmed against source: `open_with_engine_for_test` is `Live` → sleepable; emit pattern already used in `cascade_sleep_defers_…`. Keep the pending_sleep assertion.”

---

### M2. `drop(member)` after cloning `tab` is fine for the borrow checker

**Where:** Plan Task 4 focus area / `move_terminal_members` snippet.

**Problem:** Review prompt worried about move/`drop(member)` after `member.tab.clone()`. Fields `last_viewed`/`hidden`/`pending_sleep` are `Copy`; `tab` is cloned; `_sub` remains until `drop(member)`. This compiles.

**Failure:** None.

**Fix:** Keep as-is; optional clippy `drop` is stylistic documentation of subscription teardown order (good).

---

### M3. Task 1 Step 5 is already partially done

**Where:** Plan Task 1 Step 5 adds `lens-ui` dev-dep with `test-util`.

**Actual:** `crates/lens-ui/Cargo.toml:46` already has `lens-terminal = { path = "../lens-terminal", features = ["test-util"] }` under `[dev-dependencies]`, and `test-util` exists on `lens-terminal`. Normal dep without feature remains at `:22`.

**Failure:** Wasted churn / confusing “add” instruction.

**Fix:** Change Step 5 to “confirm existing dev-dep; only add `test-util = []` on lens-terminal if missing (it is not).”

---

### M4. Task 5 / Task 2 tests need imports the plan never lists

**Where:** Task 2/5 test snippets use `TerminalId`, `TerminalHostEvent`, `TerminalResourceSignal`, `SessionControl`, `FakeSessionLoader`, `Rc`.

**Actual:** `terminal.rs` test module currently imports `EngineConfig`, `EngineHandle`, `PER_CELL_BYTES` — not `TerminalId` / `TerminalResourceSignal`. Production imports lack `TerminalResourceSignal`.

**Failure:** First compile of the failing tests fails on unresolved imports (expected for TDD red, but the plan’s “FAIL: SessionControl not found” understates the real error set).

**Fix:** List required imports explicitly next to each test block.

---

### M5. No unit coverage for “loader missing → no-op”

**Where:** Task 5 guards `let Some(loader) = self.session_loader.clone() else { return; }`; no test.

**Failure:** Accidental removal of the early-return could strand terminals only in integration.

**Fix:** One-liner test: supersede with no loader set → member stays under A, no Transfer.

---

### M6. `Rc<dyn SessionLoader>` is appropriate

**Where:** Plan Task 3 decision; design soundness Q.

**Verdict:** Agree. `FleetStore` is a single-threaded gpui entity; `Rc` matches existing patterns (`PtyProbe`, etc.). `Arc` would wrongly suggest cross-thread loader sharing. Object-safe trait shape is fine. Keep the “must not sync-update under active borrow” rule (C2).

---

## Nit

### N1. Search hint `TerminalTab {` for constructors

Constructors use `Self {` inside `impl TerminalTab` (`lib.rs` open path ~540, `with_engine_for_test` ~585). Searching only `TerminalTab {` finds the struct def. Say “every `Self {` in `impl TerminalTab`”.

### N2. Expected test count `217/217`

Handoff tip was `216/216`; branch may have moved. Prefer “existing +1” without a hard total.

### N3. `seed_disk` `pub(crate)`

Child modules already see crate-root private `fn seed_disk` (`fleet_verify` already calls it). `pub(crate)` is harmless documentation, not required.

### N4. Poller path for `SessionControl`

Using `crate::fleet::terminal::SessionControl::…` works; a `use` would match local style. Non-blocking.

---

## Spec fidelity summary

| Design obligation | Plan coverage | Notes |
| --- | --- | --- |
| §4.1 `on_session_control` routing | Task 2 | Good; typed `SessionControl` is a sound boundary |
| §9 resource forwarding to owned terminals | Task 2 | Good; Slice-4 tab filter contract preserved |
| §9/§13 both-order adoption at D | Deferred (Q4) | See I4 — must be explicit in design/handoff or add identity seam |
| §10 load B headlessly | Tasks 3+5+6 | Right seam; C1/C2/I1 must be fixed |
| §10 move A→B, no rekey | Task 4 | Known subscription trap correctly handled |
| §10 drive retain-engine `Transfer` | Task 5 + Task 7 rider | Unit proves forwarding only (no `current_tid`); rider is mandatory |
| §4.2 map_item | Explicitly out | Correct given Q8; rider must confirm live `superseded` (I6) |
| §14 card-bound / focused-replica unperturbed | Task 2 gate + Task 7 review | Good; poller arms are additive before `other =>` |
| §14 do not bypass `on_host_event` | Global constraint | Good; Transfer/resource go through host event |

---

## Risk to landed A/B/C work

- **Card-bound outcome path:** Low risk if poller arms are added *before* `other =>` and `apply_outcome` keeps exhaustiveness no-ops. Coalescing/decay tests remain the regression net.
- **Focused-replica routing:** Untouched by the plan. OK.
- **A’s frozen-engine seam:** Plan correctly routes only via `TerminalTab::on_host_event`. Recorder is append-only under `test-util`. OK — provided Task 1 does not alter match arms.
- **C membership / pending_sleep:** Task 4 is specifically protecting C’s deferred-sleep invariant across re-parent. Necessary and well-motivated.

---

## Compile checklist (executor must treat as blockers)

1. `session_loader` must be `pub(crate)` (C1).
2. Fake loader must not sync-`update` the store from inside `on_supersede` (C2).
3. `new` **and** `new_live` struct literals need `session_loader: None`.
4. Task 6: `GetOpts` from `lens_client::sessions`; wire via `prep`, not `config` (I2/I3).
5. `cx.spawn` / `Task::ready` / `cx.subscribe` shapes match existing lens-ui usage — OK as written **after** C2.
6. `host_events_for_test` requires `test-util` feature unification already present in lens-ui dev-deps — OK.

---

## VERDICT: **NEEDS-REVISION**

Must-fix before execute:

1. **C1** — `pub(crate) session_loader` (or move supersede into `store.rs`).
2. **C2** — eliminate Fake/sync re-entrant `store.update` under an active FleetStore borrow; make Fake async like the real loader (or always invoke `load` only from a detached spawn after the outer update returns).
3. **I1** — add an in-flight/staleness token so async load completion cannot double-load or complete against a stale supersede.
4. **I2 + I3** — rewrite Task 6 wiring/imports against actual `main.rs` / `lens_client` paths.
5. **I4** — either add a bound-identity forwarding→adoption lens-ui test seam, or explicitly amend §13 D’s “both orders → adoption” proof obligation in the recorded planning resolution / handoff so it is not silently dropped.

After those five, the plan is READY-TO-EXECUTE.
