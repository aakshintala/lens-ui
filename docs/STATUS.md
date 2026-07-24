# Lens — STATUS

Lean, living status for the Lens design effort. Keep this doc **small**: it holds
the current forward-looking state only. **Full dated session entries live in
[`STATUS-ARCHIVE.md`](./STATUS-ARCHIVE.md)** — write each session's detail there
and roll older "Recent" pointers off this page as they age.

_Last curated 2026-07-23 (**Transcript ▶ T-3 (message & reasoning content — markdown) MERGED to `main`** — `main`→`t3` reconcile brought in the 42-commit terminal slice-5 landing (below); conflicts were only `Cargo.lock`, `STATUS.md`, and doc curation (code auto-merged: no reduce/ file overlap — T-3 = `items/scratch/transforms/view`, slice-5 = `folds/update`); full `xtask gate` re-run GREEN on the merged tree + both T-3 real-window probes re-run teeth-verified before the fast-forward. Demo now SHOWCASES T-3 (`6e9c261`): `LENS_DEMO_FOCUSED` seed gained a curated tail block — assistant markdown (h2/h3, bold/italic, inline code, lists, ```rust fence w/ highlight, blockquote, table, link) + user segment pipeline (```md fence + autolink) + reasoning summary→full (7s) & encrypted (🔒); verified on-device. **Was:** COMPLETE on `t3-message-reasoning-content` `3df8bc6`, MERGE-READY.** All 6 tasks (T3-0 vendor gpui-component markdown behind cap-lints + P1/P2 streaming/selection patches + `md::init`; T3-M typed `RowContent`/`ContentKey`/durable reasoning `duration_ms`; T3-1 assistant `MarkdownView` + D1 streaming coalesce + D11 identity; T3-2 `security.rs` fail-closed boundary + P5/P6 image/link gates; T3-3 user segment pipeline + validated autolinks; T3-4 reasoning four states + summary→full + live stick-to-bottom + P3) subagent-driven: composer-2.5 authors + **grok-4.5** per-task cross-family reviews + controller fix waves. **Every Critical/Important fixed & executably verified** — D11 reproject-clobber (upsert-preserve, guard test fails-without-fix), P2 selection carve-out + reasoning stick-to-bottom both **teeth-verified at a real window** (2 probes: `focused_finalize_probe`, `focused_reasoning_probe` — both feature-gated behind `probe`), security choke-point airtight (single `validate_*` gate on every paint+click+nav path, no `img()` fetch, NavigateToFile fail-closed). **Final Opus whole-branch review (BUILT the gate) = merge-quality; caught the gate RED on `cargo fmt --check`** (per-task gates ran check+clippy+test, never fmt) → fixed: glue formatted, vendored `md/` files get `#![cfg_attr(rustfmt, rustfmt::skip)]` (rustfmt.toml `ignore` is nightly-only) so they stay byte-faithful to upstream. **`xtask gate` GREEN** (fmt+clippy+lens-ui/core tests; lens-terminal parallel flake unrelated, passes isolated). Deferred (roll-up, all verified safe): T2/I2 coalesce doesn't gate reproject (live-tail bounded, post-T-6 perf), CodeBlock syntax-highlight for non-md user fences, minor polish. Handoff `docs/handoffs/2026-07-23-t3-executed.md`. **Big learnings: 2 real-window-probe false-green traps** (sticky-latch pass, arming while bounds=0×0, `wait_frames < 100ms reparse throttle`, `use_keyed_state` is paint-only) — the RUN + teeth-check is the only proof; **per-task gates must run `fmt --check`** (a building whole-branch reviewer caught what 6 per-task no-build reviews missed). ——— (prior) **T-2b (disk-windowing) MERGED to `main` `b2dfc51`** (merge = `main`↔branch reconcile, ZERO conflicts, full `xtask gate` re-run GREEN exit 0 incl. real-window render+stream_perf in budget, NOT pushed). **Demo reviewed by user 2026-07-23** — the bare undecorated "Message" blocks are EXPECTED (T-1 emits thin `ViewBlock`, no meta); a transcript UI-polish pass is DEFERRED to post-T-6 because T-3/T-4/T-6 each redraw every block (memory [[transcript-ui-polish-deferred]]). **NEXT: T-3 (message & reasoning content — markdown) in a FRESH session, branch off `main`.** ——— (prior context) branch `t2b-disk-windowing` `ac9f7da`; user set the "see the demo before merge" boundary. `FocusedTranscript` → bounded, byte-budgeted, LRU-evicted RESIDENT WINDOW over the disk transcript with scroll-back paging + scoped reconcile. All T1–T6 done subagent-driven (composer build + grok per-task reviews); **full `xtask gate` GREEN** (clippy -D warnings + fmt + tests lens-core 310/lens-ui 197 + benches + drift), real-window probe exit 0, **demo renders** (`docs/evidence/t2b-focused-demo.png`; `LENS_DEMO_FOCUSED=1 cargo run -p lens-app --features demo`). Consts VALIDATED by out-of-gate `xtask focused-sweep` (50k items=33ms cold-focus/12.6MB resident, ~30× faster than the ~1s full-history reconcile, ½ the 24MB cap) — NO tuning. **End-of-workstream CODEX (gpt-5.6) whole-branch review earned its keep** — found a Critical every per-task review+probe+test missed (incremental live-tail reproject silently drops a settled-band Delta when `live_section_lo==None`); fixed F1(Crit)+F2/F3/F4/F7 each w/ non-vacuous test, F1 fix regressed (append yank) → universal scroll-anchor identity re-pin (probe caught it), rejected F8, deferred F5/F6; grok fix-wave review = SHIP. Handoff `docs/handoffs/2026-07-23-t2b-executed.md`; memory [[t2b-disk-windowing-executed]]. ——— (prior) **`origin/main` merged into `lens-transript`** to pick up the landed terminal workstream — reducer seam reconciled: T-0 `active_response`/response-id model × terminal turn-counter unified in the `Incomplete`/`Cancelled`/`Failed` arms; lens-core 301 tests green. **Terminal workstream LANDED ON `main`** — Slices 0–4 (VT foundation → render → input → clipboard/mouse → byte-accounting/perf → lifecycle mechanisms), first terminal on main; in parallel on main: **B-4a cull-at-scale residual CLOSED** (`5de3b93`) + B-4b collapse-toggle design LOCKED. **Transcript T-0 + T-1 + T-2 (ALL 15 tasks) LANDED ON `main`** (`60425d2`, PUSHED) via two catch-up merges (terminal workstream + B-4b), reducer + board seams reconciled and grok-reviewed clean (0 findings); end-of-workstream review had fixed 3 Criticals (Task-12 "crux" was FALSELY validated; memory `false-green-probe-drives-production-path`), lens-store DELETED, flaky Task-7 DETERMINISTIC (0/200). Full gate GREEN on `60425d2` (lens-core 301 + lens-ui 162, clippy/fmt/no-drift). Merge reconciliation lesson in memory `reducer-two-workstream-merge-reconciliation`. **NEXT (new session): transcript ▶ T-2b (disk-scale)** — see `docs/handoffs/2026-07-22-t2b-kickoff.md`; branch fresh off `main`. Also live on main: **terminal Slice 5** (lens-ui `FleetStore`) + **board visual pass** (B-4b held for it). **Reorg:** disk-scale → **T-2b**; live tool-tail → **T-4**; polymorphic `ContentTab` → terminal-UI-integration, SPEC-GAPS)._

_(prior curation, now on `main` — **▶▶▶ TERMINAL SLICE 5 (A+B+C+D) MERGED TO `main` `2404bc8`** — fast-forward after a `main`→branch reconcile (3 conflicts: lens-ui `Cargo.toml` rusqlite-drop × lens-terminal-add, `STATUS.md` dual curation, `Cargo.lock`); **full `xtask gate` GREEN on the merged tree incl. the real-window render + stream_perf harnesses in budget**. NOT pushed. **Sub-slice D was executed + whole-branch-reviewed + ALL LIVE RIDER LEGS CLOSED.** **Riders DONE (`003e242`)**: the three outstanding legs were logged as "needs the GUI app + a human watching" — wrong framing, since **there is no production terminal surface until Slice 6**, so `lens-app` cannot host them. Built instead as two automated phases in `crates/lens-terminal/tests/terminal_live.rs` (already a real GPUI window around a production `TerminalTab`), **both PASS ×2 consecutively** vs live omnigent 0.5.1. **P9** (`LENS_LIVE_TRANSFER=1`) drives the real 4-call `/clear` rotation over REST and asserts (a) the attach **survives the transfer untouched**, (b) `resource.deleted` + `Transfer{new_session}` return the tab to `Live` **against B** with `output_gap` set and the pre-rotation marker still on screen — closing the scrollback-survival and `output_gap` legs **mechanically** (marker survival *is* scrollback survival; `output_gap` asserted, not eyeballed), and standing as the **only** proof cross-session engine reuse actually lands (the in-crate test can only show the branch is *entered* — its offline attach against B fails by construction). **P10** (`LENS_LIVE_4404_FIRST=1`) forces the 4404-first leg: deletes the terminal resource and forwards **nothing**, asserting the tab parks in `ReplacementWaiting` **holding its frozen engine** on the close code alone, then relaunches the same key and forwards the late `deleted`+`created`, which must still drive `adopt()` to `Live`. **Two live findings**, both folded into the code: (1) **a `…/transfer` is attach-transparent — it produces NO 4404** (P10's first draft assumed it would and failed; the tab stayed `Live` with a healthy bridge — the WS binds to the terminal/tmux socket, not the session), so the host's forwarded `resource.deleted` is the **sole** trigger for the supersede follow with no transport backstop; P9 now asserts that transparency directly, and deleting the resource became the only cheap live 4404 source; (2) **delete+create returns the SAME deterministic id** (`terminal_shell_main`) even though the delete demonstrably took — confirming [[terminal-resource-event-granularity]] from the other direction and explaining why the generation guard keys on the delete/create **events**, not on an id change (an id-comparison guard would be a no-op; P10's second draft asserted id-inequality and failed on this). `reqwest` joins lens-terminal dev-deps (transfer + session-create routes are `include_in_schema=False` upstream → unmodeled in lens-client); `cargo run -p xtask -- gate` **all checks passed**. Results doc `docs/handoffs/2026-07-23-terminal-slice-5-D-rider-results.md`. **NEXT = the deferred **foreground blocking handshake** fix in its own session**, then **Slice 6** (production terminal surface + E2E-in-app) — now better motivated: the load-B window is bounded by the 30s `REPLACEMENT_WAIT`, so the handshake cost is a **budget**, not just a hitch. — prior curation this session:  All 7 tasks done subagent-driven (composer-2.5 authors, grok-4.5 cross-family review per task — codex quota exhausted this week). Commits `620dc41`(test-util host-event recorder seam) · `8e198fb`(poller → typed `SessionControl` → `on_session_control` + resource fan-out to owned terminals) · `e8b0f65`(`SessionLoader` seam + `session_loader`/`supersede_epoch`/`supersede_in_flight`) · `b3b9eb0`(**`move_terminal_members` — THE TRAP: re-parent A→B with a REBOUND subscription**, verified closed on every exit path incl. gpui `Subscription::drop` genuinely unsubscribing) · `3a832b0`+`e69b4f6`+`aab685e`(`Superseded`→load-B→move→`Transfer` + 6 guard tests; review pass 1 caught a **vacuous** load-failure test and an **untested** already-tracked guard) · `80a0014`(real `AppSessionLoader`: background blocking GET → `seed_disk` → foreground `spawn_live_session`, wired at `main.rs` `prep.conn`/`prep.data_dir`; **no headless test by design** — the rider is its proof) · `9b22bcf`(surface supersede load failures instead of `is_ok()`-discarding them — a failed `/clear` was previously SILENT) · `e465f4a`+`9e9c62f`(whole-branch review fixes). **The whole-branch review earned its keep: it found a CRITICAL every per-task review structurally could not see** — `supersede_in_flight` was correctly per-target but `supersede_epoch` was a single GLOBAL `u64` bumped by every supersede, so an unrelated C→D supersede falsely marked an in-flight A→B completion stale → **A's terminals stranded under the dead session, no `Transfer`, scrollback follow silently failed, and B left as a fully-spawned wasted live session**, with no log and no panic. Root cause: the plan mirrored sub-slice A's `reconnect_epoch`, but that epoch is per-**tab** while supersedes are per-**session**. Fixed to `supersede_epochs: HashMap<SessionId, u64>` keyed by `from`, locked by a genuinely-overlapping concurrent-supersede test (gated loader parks both loads) plus a same-source stale test proving the guard was not weakened away. Also added `ReentrantSessionLoader`, a **poison** fake that synchronously `store.update`s from inside `load`, locking the "never invoke `load` under an active `FleetStore` update" gpui non-re-entrancy contract that nothing — not even the live rider — would otherwise have caught. Final gate: lens-terminal 217/217, lens-ui 191/191, `cargo run -p xtask -- gate` **all checks passed**. **DEFERRED by user decision to a dedicated session right after the riders:** `AppSessionLoader`'s `Client::new` is a multi-RTT blocking handshake on the app executor and `spawn_live_session` does another + a blocking stream open → mid-`/clear` UI hitch risk; deferred because `spawn_live_session` is shared with the app **startup** path (cross-cutting signature change outside slice 5) and because **nobody has measured the hitch — the rider is the first observation**. **NEXT: live riders against a live omnigent 0.5.1** (skill `installing-omnigent-from-source`) — (1) scrollback survives `/clear` **and** the captured SSE really contains `session.superseded` w/ `target_conversation_id`=B (D deliberately has **no `map_item` fallback**, so the entire follow depends on that outcome arriving); (2) 4404-first real interleaving; (3) transfer `output_gap` visual — **then merge whole slice-5 (A+B+C+D) to main together.** Records: `.superpowers/sdd/whole-branch-review.md`, ledger `.superpowers/sdd/progress.md`. **Prior curation 2026-07-23:** Terminal Slice 5: B+C review RESOLVED + A grill DONE + A plan written. codex `gpt-5.6` review of composer-authored B+C returned DON'T-MERGE with 8 findings (4 HIGH); all fixed test-first (`694870b` control-signals-off-lossy-ring + cascade-wake pending_sleep + `TerminalHostEvent::End` teardown + `real-window` feature split; `6e169ae` engine 2-atomic retained-bytes + lens-drive JSON; 5/7 re-scoped to S6 — no production FleetStore-terminal caller yet). **A fresh Opus re-review caught a real HIGH the fixes missed** — the initial `open()` attach spawn was unguarded → resurrected an `Ended` tab with a live engine (leak); fixed `b12676e` with the same epoch-guard the other 4 spawns use. Cherry-picked main's terminal-test speed fix (`e5c72fb`, 31s→3s). Gate green (combined `--lib` 174+311+208+177, clippy/fmt; real-window bins no longer activate in the ordinary gate). **A grill Q1–Q6 decided** → unified `adopt(session,tid)` (same→fresh/drop-retained · changed→reuse), `TerminalHostEvent::Transfer{new_session}` through the reuse branch + cross-session no-double-feed test, `Existing` 4404 hard-detach, `scrollback_lines`→`scrollback_bytes` default 10MB (byte-budget bug), whole slice-5 merges to main together after D+review+live-riders. **A plan** = 4 TDD tasks at `docs/plans/2026-07-23-terminal-slice-5-A-lifecycle.md`. **▶▶ SUB-SLICE A EXECUTED + FINAL-REVIEWED (2026-07-23)** — subagent-driven (composer-2.5 authors + fresh-Opus per-task reviews + Opus whole-slice, codex out this week); commits `d8b13c7`(scrollback byte-fix 10MB) · `de760e0`(OpenOrCreate 4404→ReplacementWaiting) · `eb7290a`+`58bc36c`(retain frozen engine + unified `adopt()` same→fresh/changed→reuse, apply-time epoch-guarded both branches + strengthened discriminators) · `dc43f09`+`6b65a28`(`TerminalHostEvent::Transfer`→reuse branch; redundant engine-level transfer-seed test DELETED — cross-session no-double-feed is a live-rider contract). **Whole-slice-A Opus review = READY-TO-MERGE (with B/C/D)**: 216/216 (`--test-threads=4`), clippy/fmt clean, frozen-seam checklist A–F all PASS (epoch guard on every spawn; no retained-engine leak on {same-session drop, 30s timeout, Existing 4404, Transfer}; Transfer reuses exact engine; Slice-4 frozen-state gate holds; 4404-first→delete→create→adopt chain proven e2e). No Critical/Important; Minor (10MB literal dup) + Nit (transfer `output_gap` visual → live rider). Known gate flake `wheel_no_tracking...` = [[worker-stall-gate-busy-spin-flake]] (isolated 5/5 pass). Resume ledger `.superpowers/sdd/progress.md`. **▶▶ SUB-SLICE D PLANNED + CROSS-FAMILY-REVIEWED (2026-07-23) — NOT started, zero `crates/` changes.** Plan `docs/plans/2026-07-23-terminal-slice-5-D-fleet-integration.md` (`4baa644`, revised `f9d3108`); 7 TDD tasks (recorder seam → `SessionControl` routing + resource forwarding → `SessionLoader` seam → `move_terminal_members` → `Superseded`→load-B→move→`Transfer` → real `AppSessionLoader` → riders). **The design's 4 deferred D-planning questions are RESOLVED, source-verified:** (1) headless load-B is *not* UI-entangled (`fleet_verify.rs:73` proves it) but `FleetStore` retains no `Connection`/`Client`/`data_dir` (`store.rs:59-77`) and a brand-new B must be GET+seeded before `spawn_live_session` (`scheduler.rs:103-105` → `SessionNotFound`) → **user decision: full supersede in D behind an injected `SessionLoader` seam** (store logic headless-tested with a fake; real GET→seed→spawn in lens-app, rider-proven); (2) the GET is blocking → the seam returns a `Task` so IO runs off-foreground; (3) **§4.2 `map_item` NOT needed** (Transfer driven off the outcome, not B's snapshot) → *no fallback*, so the rider must assert the live event order contains `session.superseded` on A; (4) **DESIGN AMENDMENT — §13 D's "both orders → adoption fires" becomes "forwarding fidelity at D"**: lens-ui cannot synthesize a `4404` (`apply_bridge_event` private) and a for-test tab has `generation: None` so `on_resource_signal` early-returns — A's `fourohfour_first_then_delete_create_adopts` already binds the chain with production-authored state; a `bind_identity_for_test` seam was REJECTED as a false-green risk [[false-green-probe-drives-production-path]]. **THE TRAP D must not miss:** `TerminalMember`'s subscription captures its owning `SessionId` (`terminal.rs:241-247`) and `on_terminal_presentation_changed` early-returns on a missing key (`:274-280`) → a naive member move A→B silently kills C's deferred `pending_sleep`; Task 4 rebinds the subscription. **grok-4.5 plan review = NEEDS-REVISION → folded → READY-TO-EXECUTE** (`docs/reviews/2026-07-23-terminal-5-D-plan-review-grok.md`): 2 real Criticals — C1 `session_loader` must be `pub(crate)` (sibling-module invisibility, same reason `terminals` is), C2 the loader must NOT be invoked inline from `on_supersede` because the poller calls `on_session_control` under an active `FleetStore` update and **gpui entity updates are not re-entrant** (the fake's `store.update` would panic) → `load()` now runs inside the spawned task; plus I1 `supersede_epoch` staleness guard + `supersede_in_flight` dedup (mirrors A's `reconnect_epoch` discipline), I2/I3 Task-6 wiring corrected to `prep.conn`/`prep.data_dir` at `main.rs:101` + `lens_client::sessions::GetOpts`. grok's source-check also RESOLVED a soft spot (Task 4's `pending_sleep` assertion is valid — `with_engine_for_test` is `Live`, `is_sleepable` accepts `Live`). Handoff `docs/handoffs/2026-07-23-terminal-slice-5-D-planned.md`. **NEXT: EXECUTE D (subagent-driven, Task 1 → 7) → live riders (supersede scrollback + live `superseded` event order, 4404-first ordering, transfer output_gap visual) → merge whole slice-5 to main together.** **Review routing:** codex quota exhausted this week → Opus (fresh subagent, builds+tests) or grok-4.5. Memories [[feature-unification-gate-trap]], [[lifecycle-signals-off-diagnostic-ring]], [[codex-quota-exhausted-week-2026-07-23]]. Handoff `docs/handoffs/2026-07-22-terminal-slice-5-A-grill-B-done.md`. **Prior curation 2026-07-22:** **Terminal Slice 5 design GRILLED + REVISED** on `terminal-slice-5-fleetstore` — `docs/specs/2026-07-22-terminal-slice-5-fleet-membership-design.md` rewritten with a Q1–Q10 decision ledger. Key outcomes: Slice 5 is **not** pure-headless — `4404`-first + `session.superseded` both need deliberate **`lens-terminal` lifecycle changes** (verified against omnigent + lens-terminal source, not assumed); supersede **loads B + retains the engine** so scrollback survives `/clear` (de-risked: no byte-replay contract + `reconnect_seed` tests), discriminated by `session_id`-changed vs same-session agent-switch (pane `kill-server`'d → fresh); pressure = ordinal fraction-freed + `hidden && Live` eligibility (RSS can't attribute per-terminal); cascade Wake-non-hidden + `pending_sleep`. **Re-split into A (terminal-lifecycle) · B (core-surface) · C (fleet-membership) · D (fleet-integration).** Memories [[terminal-supersede-vs-agentswitch-semantics]], [[process-per-engine-isolation]]. **BUILD ORDER REVISED → SEQUENTIAL** (`B → A → C → D`): evaluated parallel A/B/C, **no real wall-clock gain** (A is long pole *and* gates D; B/C would finish early and wait; parallel adds gate contention + merge reconciliation + divided attention on A). **▶▶ Sub-slice B DONE + committed `1bbcdef`** (composer-2.5; gate green fmt+clippy-Dwarnings+tests, +10 tests 501→511) — core-surface `SessionEvent::Resource*` payloads + `StreamUpdate::{Superseded,TerminalResource*}` + `ActorOutcome::{Superseded,TerminalResource(TerminalResourceSignal)}` control-path; `map_item` deferred (§4.2). **⏳ B PENDING cross-family (codex) review before merge** (validate 4 judgment calls). Downstream compile unblocked (`9b5c541`): added the new-variant match arms in lens-ui (card/model, focused = no-op control-path; poller = interim-until-D) + lens-drive (JSON) — **workspace compiles green**. **▶▶ Sub-slice C DONE + committed `2c12a91`** (composer-2.5; gate green fmt+clippy-Dwarnings+lens-ui tests, 173 unit +6 acceptance) — FleetStore nested `terminals` membership + open/visible/close + cascade Sleep-all/Wake-non-hidden (+pending_sleep Q5) + pressure LRV Warning-fraction-freed/Critical (hidden&&Live Q10) + idle auto-sleep; `TerminalTab::retained_bytes_estimate()` accessor. lens-ui now deps lens-terminal (+test-util dev-dep). **Opus fixed a composer flaky test** (pressure tests read the ASYNC worker-sampled estimate → raced to 0 → nothing slept, ~60% fail; `spawn_tab_with_rows` now barriers on a post-feed build, gated on rows-fed since `build_now` no-ops when not dirty; verified 40x single + 5x full parallel). **C PENDING cross-family review — batch with B.** **▶ Sub-slice A grill IN PROGRESS, paused mid-Q1** — grounded in the real state machine (A reframed into 5 concrete `lens-terminal` changes; `enter_replacement_waiting` retain-engine is global; 4404-branch lives in `apply_bridge_event`; no `Transfer` event yet). Q1 (unified session-keyed `adopt()` vs two paths) awaits answer. **NEXT: answer Q1 → finish A grill (Q2–Q6) → A plan → execute A.** Handoff `docs/handoffs/2026-07-22-terminal-slice-5-A-grill-B-done.md`. Provisional-open: idle threshold (~10m), Warning fraction. — earlier this session: **`origin/main` merged into `lens-transript`** to pick up the landed terminal workstream — reducer seam reconciled: T-0 `active_response`/response-id model × terminal turn-counter unified in the `Incomplete`/`Cancelled`/`Failed` arms; lens-core 301 tests green. **Terminal workstream LANDED ON `main`** — Slices 0–4 (VT foundation → render → input → clipboard/mouse → byte-accounting/perf → lifecycle mechanisms), first terminal on main; in parallel on main: **B-4a cull-at-scale residual CLOSED** (`5de3b93`) + B-4b collapse-toggle design LOCKED. **Transcript T-0 + T-1 executed**, **T-2 ALL 15 tasks DONE + CLOSED** on `lens-transript` (still UNMERGED, user keeps-as-is) — end-of-workstream review fixed 3 Criticals (Task-12 "crux" was FALSELY validated; lesson in memory `false-green-probe-drives-production-path`), lens-store DELETED, flaky Task-7 DETERMINISTIC (e7a9ee8, 0/200), pre-merge gate GREEN on 5790203. Resume ledger `.superpowers/sdd/progress.md`. **Full gate GREEN post-merge** (8ea22d8; all crates incl. lens-terminal, lens-core 301 + lens-ui 150, clippy/fmt/no-drift). **Next:** terminal Slice 5 (lens-ui `FleetStore`) + board B-4b. **Reorg:** disk-scale → **T-2b**; live tool-tail → **T-4**; polymorphic `ContentTab` → terminal-UI-integration, SPEC-GAPS ——— **IN PARALLEL, LANDED ON `main` while slice-5 was on its branch (transcript + board workstreams; reconciled into this branch by the pre-merge `main`→branch merge):** **Transcript ▶ T-2b (disk-windowing) MERGED to `main` `b2dfc51`** (merge = `main`↔branch reconcile, ZERO conflicts, full `xtask gate` re-run GREEN exit 0 incl. real-window render+stream_perf in budget, NOT pushed). **Demo reviewed by user 2026-07-23** — the bare undecorated "Message" blocks are EXPECTED (T-1 emits thin `ViewBlock`, no meta); a transcript UI-polish pass is DEFERRED to post-T-6 because T-3/T-4/T-6 each redraw every block (memory [[transcript-ui-polish-deferred]]). **NEXT: T-3 (message & reasoning content — markdown) in a FRESH session, branch off `main`.** ——— (prior context) branch `t2b-disk-windowing` `ac9f7da`; user set the "see the demo before merge" boundary. `FocusedTranscript` → bounded, byte-budgeted, LRU-evicted RESIDENT WINDOW over the disk transcript with scroll-back paging + scoped reconcile. All T1–T6 done subagent-driven (composer build + grok per-task reviews); **full `xtask gate` GREEN** (clippy -D warnings + fmt + tests lens-core 310/lens-ui 197 + benches + drift), real-window probe exit 0, **demo renders** (`docs/evidence/t2b-focused-demo.png`; `LENS_DEMO_FOCUSED=1 cargo run -p lens-app --features demo`). Consts VALIDATED by out-of-gate `xtask focused-sweep` (50k items=33ms cold-focus/12.6MB resident, ~30× faster than the ~1s full-history reconcile, ½ the 24MB cap) — NO tuning. **End-of-workstream CODEX (gpt-5.6) whole-branch review earned its keep** — found a Critical every per-task review+probe+test missed (incremental live-tail reproject silently drops a settled-band Delta when `live_section_lo==None`); fixed F1(Crit)+F2/F3/F4/F7 each w/ non-vacuous test, F1 fix regressed (append yank) → universal scroll-anchor identity re-pin (probe caught it), rejected F8, deferred F5/F6; grok fix-wave review = SHIP. Handoff `docs/handoffs/2026-07-23-t2b-executed.md`; memory [[t2b-disk-windowing-executed]]. ——— (prior) **`origin/main` merged into `lens-transript`** to pick up the landed terminal workstream — reducer seam reconciled: T-0 `active_response`/response-id model × terminal turn-counter unified in the `Incomplete`/`Cancelled`/`Failed` arms; lens-core 301 tests green. **Terminal workstream LANDED ON `main`** — Slices 0–4 (VT foundation → render → input → clipboard/mouse → byte-accounting/perf → lifecycle mechanisms), first terminal on main; in parallel on main: **B-4a cull-at-scale residual CLOSED** (`5de3b93`) + B-4b collapse-toggle design LOCKED. **Transcript T-0 + T-1 + T-2 (ALL 15 tasks) LANDED ON `main`** (`60425d2`, PUSHED) via two catch-up merges (terminal workstream + B-4b), reducer + board seams reconciled and grok-reviewed clean (0 findings); end-of-workstream review had fixed 3 Criticals (Task-12 "crux" was FALSELY validated; memory `false-green-probe-drives-production-path`), lens-store DELETED, flaky Task-7 DETERMINISTIC (0/200). Full gate GREEN on `60425d2` (lens-core 301 + lens-ui 162, clippy/fmt/no-drift). Merge reconciliation lesson in memory `reducer-two-workstream-merge-reconciliation`. **NEXT (new session): transcript ▶ T-2b (disk-scale)** — see `docs/handoffs/2026-07-22-t2b-kickoff.md`; branch fresh off `main`. Also live on main: **terminal Slice 5** (lens-ui `FleetStore`) + **board visual pass** (B-4b held for it). **Reorg:** disk-scale → **T-2b**; live tool-tail → **T-4**; polymorphic `ContentTab` → terminal-UI-integration, SPEC-GAPS)._

---

## Next up

- **⏳ DEFERRED (owned, not forgotten): terminal slice-5 `/clear` follow does blocking network
  work on the foreground.** Deferred by user decision 2026-07-23 to a dedicated session right
  after the riders, which are now done — **this is the next terminal task, before Slice 6.**

  **The defect.** `AppSessionLoader::load` (`crates/lens-app/src/loader.rs`) correctly runs the
  first `Client::new` + `sessions().get()` + `seed_disk` on the background executor — but then
  does a **second `Client::new` on the foreground** (`loader.rs:59`), a multi-RTT blocking
  handshake, and hands that client to `FleetStore::spawn_live_session`, which **also blocks on
  the foreground** opening the SSE stream (`fleet/store.rs:407-410`
  `client.sessions().stream(&session_id)`) plus `live::open_stores`. So every `/clear` supersede
  stalls the UI thread for two handshakes and a stream open.

  **Why the riders sharpened it.** The live order is `resource.deleted` *then*
  `session.superseded` (memory `omnigent-clear-rotation-wire-contract`), so the delete parks the
  tab in `ReplacementWaiting` — which arms `TerminalTab::REPLACEMENT_WAIT`, a **30s** timeout —
  *before* the load of B begins. The foreground cost is therefore not merely a hitch: it spends a
  **budget**. Overrun it and the tab detaches and the retained engine (and its scrollback) is
  dropped, which is precisely what sub-slice A's retain-frozen-engine work exists to prevent.

  **Why it was deferred rather than fixed inline.** (1) The fix reaches outside slice 5:
  `spawn_live_session` is shared with the **app startup** path (`lens-app/src/main.rs:120`) and
  `fleet_verify.rs:73`, so making its network work async is a cross-cutting signature change.
  (2) **Nobody has measured it.** The rider was the first live observation of this path; the fix
  should start from a number (handshake + stream-open latency against a real server), not a guess.

  **Shape of the work.** Measure first; then either hoist the handshake+stream open into the
  background half of `load` and pass ready-made handles to `spawn_live_session`, or give
  `spawn_live_session` an async variant and migrate the startup path with it. Keep the
  `ReentrantSessionLoader` poison fake green — the "never invoke `load` under an active
  `FleetStore` update" gpui non-re-entrancy contract must survive the refactor.

- **BOARD visual pass + masonry + max-col cap/centering — LANDED ON `main` 2026-07-22** (`board-visual-polish`→`main`, PUSHED; gate GREEN; codex cross-family reviewed). `1144f16` visual pass (card wash **bleed → opaque `muted` base** so the group wash sits behind cards; wash 0.07→0.12; **dark titlebar** via transparent native + in-app `gpui_component::TitleBar`, `min_h(0)` shell fixes a silent scroll-break; themed nav rail; title ellipsis via `min_w(0)`; context bar suppressed on dormant states; demo = 2-member group B + 2 loose). `014e140` **pixel-masonry** — `pack.rs` `Placed.gy`→`py`, `item_height()` SSOT, per-column shortest-col backfill, uniform `GAP` everywhere (killed the phantom-HEADER-lane 48px gaps + 72–120px group chasms; root cause: `HEADER==GAP==24`, header never fits an integer card-grid); `PAD=16` breathing room; `RAIL_W = CARD_W+2·GUTTER+2·PAD` so a 1-col group box fits (no h-scroll); 2×2 kept on wide, 1×N only when it can't fit. Verified live dark+light, wide board + focused 1×4 rail. `021491d` **max-col cap + centering** — `pack.rs` `max_cols_for_width(logical_w)` breakpoints (≥1400→4, ≥2000→5, ≥3400→6, ceiling 6; tuned for 1800/2056/3840 logical-pt screens, NOT physical — 14"/16" MBP "More Space" widths) + `Packing::used_cols()` (occupied-col count → few-tile boards center on occupied width, not the empty cap); board `cols = cols_for_width(avail).min(max_cols)`, block centered via `center_offset = max(0,(pane_width−content_extent)/2)` into `content` left, `padded` widened to `max(pane_width, content_extent)` (no clip); rail passes `usize::MAX` (no-op). **Dropped the `.max(CELL_W)` clamp on `avail`** (codex P2: fed a fictitious pane width into centering on narrow windows → spurious offset + right-ring clip; `cols_for_width` self-clamps). Demo window 1340→1440 (above the 4-col breakpoint). Codex (gpt-5.6) review = 1 P2 (fixed), no others. On-device verified: 3-col (capped from 4) + 4-col both center, no clip. Fixed cards ⇒ max-cols ≡ max-px-width; block identical across screens, only margins grow. Handoff `docs/handoffs/2026-07-22-board-masonry-centering.md` (mission complete).
  - **Vertical centering (2026-07-22, on `main`):** vertical mirror of `center_offset` in `board/mod.rs` — a board shorter than the viewport is now centered vertically (`v_center_offset = (viewport_h − block_height)/2` when `block_height ≤ viewport_h`, else 0; `padded.h = block_height.max(viewport_h)`). Hard-snaps to top-aligned as sessions push `content_height` past the viewport (accepted trade-off — no animation). Codex (gpt-5.6) review caught the one non-obvious edge: when the block fits, `scroll_top` is pinned to 0 for culling so a tall scrolled board that just shrank below the viewport can't ride a stale negative offset and blank its tiles for a frame (vertical culling has no horizontal analog to lean on). Gate: fmt + clippy clean, lens-ui 162/162. Overflowing boards are byte-identical to before.

- **▶ ACTIVE: shared terminal workstream — Slices 0/1a/1b merged; 1c DONE; 1d COMPLETE (live-proven); Slice 2 SERIAL: 2a (input) DONE + C2 CLOSED, 2d (presentation) DONE + real-window-gated, ▶▶ 2b (clipboard/OSC-52 + Cmd+V paste) EXECUTED + DONE (2026-07-21): all 5 tasks (OSC-52 on_clipboard_write cap-before-clone → foreground ClipboardPolicy seam + ClipboardWriteRequest/Notice + on_host_event → EngineCommand::Paste bracketed engine-side never-drop/epoch → Cmd+V intercept read-only-gated/multiline-warn/capped → demo Deny-default + benches + inspect + live rider), each codex-gpt5.6-reviewed + fix waves, Opus whole-slice = SHIP-WITH-FIXES (caught a cross-task read-only-gate bypass on the deferred-warn paste path — dispatch_paste now gates, regression-tested) → fixed. Full 2b slice `018820b..57bccde` (11 commits). FINAL FULL GATE GREEN (fmt + workspace clippy + lens-terminal test-util,live-tests clippy + 132 lib tests + benches + demo). `terminal-ws` unpushed (backup push + `main` merge = user's call). ▶▶ 2c (mouse) **DONE (2026-07-21)** — full slice `f1922c5..c924fd9`. T5 fg lowering (thin, zero mode logic; immediate forward; `SetAccess`-wired on open AND teardown) → T6 mouse-local toggle + arbitration goldens → T7 coalesce-reset + per-mode motion + benches. **Whole-slice codex review = 9 findings, ALL folded** (F1 wheel epoch-recheck, F2 LocalClick click-time-frame **token correlation**, F3 notch-cap/overflow, F4 Any-motion local-policy, F5 Button-mode latched-button, F6 coalesce tracking-toggle reset, F7 multi-button latch guard, F8 jitter-click via `gesture_dragged`, F9 honest LocalClick-drop). **Re-review = 3 more** (Re-1 pre-egress epoch recheck FIXED; Re-2 F2-token FIXED per user; Re-3 F6-no-move-toggle documented residual). Also root-caused a suite flake (test-only worker stall gate busy-spun → starved build workers → now sleeps). **T8 DONE:** `mouse_realwindow` real-window proof (4 phases: localclick/select+copy/report/read-only PASS) + live **P6** mouse-report round-trip vs omnigent 0.5.1 (`LENS_LIVE_MOUSE_REPORT=1`, PASS) + both specs updated (DP3 engine-side; XTSHIFTESCAPE RESOLVED-DEFERRED). **DP3 AMENDED:** arbitration/latching/coalescing engine-side at ordered-stream position; `Frame` mode hint rejected. **176 lib tests, workspace+test-util clippy clean, fmt, benches compile, stable 8/8.** Plan `docs/superpowers/plans/2026-07-21-terminal-slice-2c-mouse.md`. **NEXT — remaining slices RESHAPED 2026-07-21 (design/grilling pass; design spec Build-sequence revised, memory [[terminal-slice-3plus-replan]]):** old 2-slice tail (Slice 3 lifecycle&fleet → Slice 4 perf) is superseded by **3 → 4 → 5 → 6**: **Slice 3** byte-accounting (thin per-tab retained-bytes *estimate*) + perf acceptance (demo-hosted, thin multi-tab spawner); **Slice 4** lifecycle *mechanisms* (full generation guard, Sleep/wake teardown, `ReplacementWaiting`; `Ended` **inert** — no 0.5.1/**0.6.0** termination signal, verified; module/demo, host-agnostic); **merge `terminal-ws`→`main`** after 3+4 (pure `lens-terminal`+demo, low-risk — first terminal landing on main); then on a fresh branch off main: **Slice 5** lens-ui **minimal** `FleetStore` membership + fleet policy (memory-pressure LRV trim/disconnect, when-to-sleep, `session.superseded` as sub-slice **5-super** lens-core-first) + **Slice 6** full production terminal surface + **E2E-in-app** (old "lens-ui integration out of scope" deliberately expired). Byte-*accurate* FFI = fail-closed conditional (no C-ABI accessor + compressed scrollback; escalate only if the estimate is ordinally unreliable — RSS covers absolute budget). Demo = permanent module isolation/perf rig. **▶▶ SLICE 3 (byte-accounting + perf) DONE (2026-07-22)** — full slice `af0b605..f30f894` on `terminal-ws`, `xtask gate` GREEN. `EngineInspect.total_rows`+`retained_bytes_estimate` (sampled per build; `PER_CELL_BYTES` provisional 4, ordinal-only). **Job A** `stream_perf_realwindow` real-GPUI (in macOS gate): paint_p95 3.1 ms / build_p95 0.57 ms under sustained 4-tab streaming, hidden-tab suppression asserted. **Job B** `rss_probe` bin + `xtask terminal-rss-sweep` (out-of-gate acceptance): 1k–50k rows × {compressible,incompressible} → **estimate ordinally reliable, NO flips → byte-accurate FFI conditional NOT triggered.** **Two real bugs found by Job B + fixed:** (1) 64 MiB worker stack (`67e8192`) — libghostty scrollback overflowed the ~2 MiB default at ~2000+ rows (real product crash); (2) `max_scrollback` is a BYTE budget, not lines (vendored doc wrong). Memory [[terminal-max-scrollback-bytes-and-worker-stack]]. Plan `docs/plans/2026-07-22-terminal-slice-3-byte-accounting-perf.md`; handoff `docs/handoffs/2026-07-22-terminal-slice-3-executed.md`. **Slice-3 codex follow-ups TRIAGED + CLOSED (2026-07-22, `59b3b06`):** filter = carry only genuine dep limits. FIXED (all perf-gate correctness) — I6 Job-A false-green (sustained post-flip build floor), I7 build-p95 aliasing (per-distinct-build sampling; second half already fixed by I1), a torn read + empty-samples panic caught by the review-of-the-fixes, `retained_bytes_estimate` doc (ordinal score NOT bytes), `render_realwindow` 400×100 budget 8→10 (load-flake re-baseline), 64 MiB stack comment tied to the byte cap. DECIDED — I5 keep the spec's flip-not-scale ordinal criterion (no code). CARRIED (honest C-ABI/dep limits) — I2 alt-screen `total_rows` under-count + uncharacterised libghostty stack shape. DEFERRED to Slice 4 — Minor worker `.expect()` (belongs with engine-spawn lifecycle). One codex High (I7 Δ>1 build-miss) ACCEPTED-with-doc: clean fix impossible (event ring is `BytesFed`-flooded), residual small at 2.5× budget margin. Verified: clippy clean, `inspect` lib 16/16, `render_realwindow` 4.779<10, `stream_perf` foreground `all budgets OK` (paint 3.494, build 0.559). **So Slice 3 is fully closed — no open follow-ups into Slice 4.** **▶▶ SLICE 4 (lifecycle mechanisms) EXECUTED (2026-07-22)** — full slice `8ff7cc8..f5ced39` on `terminal-ws` (14 commits), subagent-driven (composer-2.5 authors + **Grok-4.5** per-task cross-family reviews + fix waves; Codex reserved for workstream end per user, low credits). Pure `lens-terminal`+demo+xtask (merge-safe). Delivered: pure `GenerationGuard` reducer (`generation.rs`); resource-signal correlation → `ReplacementWaiting`/`Detached`/adopt + `reconnect_epoch` cancellation (bound to real reconnect exit arms); exact-key successor adoption (fresh engine) + bounded 30s `ReplacementWaiting` timeout; Sleep/Wake (apply-time `is_dirty` re-check) / Reattach (4405) host actions; fallible `EngineHandle::spawn`→`Detached(EngineSpawnFailed)` (folds Slice-3 Minor); demo `ctrl-alt-{s,w,r,x,d}` chords + opt-in P7/P8 live riders. `Ended` stays inert. **Whole-branch Grok review caught 1 Critical the per-task reviews structurally couldn't see** (`f5ced39`): `apply_bridge_event` had no lifecycle gate → a late bridge close (common `4404` on reset) clobbered `ReplacementWaiting`/`Sleeping` → fixed (gate frozen states + unify detach teardown/epoch). **Headless gate GREEN** (fmt, clippy `--all-targets` ×2 configs + demo, `--lib` 206/206 at `--test-threads=4`, benches). Plan `docs/plans/2026-07-22-terminal-slice-4-lifecycle-mechanisms.md`; handoff `docs/handoffs/2026-07-22-terminal-slice-4-executed.md`; memory [[terminal-slice-4-executed]]. **Deferrals (documented, SPEC-GAPS):** reconnect-path full guard (upstream generation token), `4404`-first adoption ordering (Slice-5 bridge↔host event model), inspect correlation state (Slice-6). **⏳ OPEN before merge:** run the **foreground gate** `! cargo run -p xtask -- gate` (real-window harnesses frame-starve headless) + optional demo smoke / live riders. **Immediate action:** foreground gate → then **merge `terminal-ws`→`main`** (first terminal landing; user's call) → then Slice 5.
    - **▶ SLICE 2b (clipboard/OSC-52 write + Cmd+V paste) — DONE (2026-07-21).** RE-CUT per the
      2026-07-20 spec amendment to **OSC-52 output-clipboard write policy + Cmd+V paste ONLY**
      (selection + Cmd+C copy MOVED to 2c — they share 2c's mouse-capture stack + XTSHIFTESCAPE
      arbitration). Policy state is session-scoped behind an injectable **`ClipboardPolicy`** trait
      (in-memory `SessionClipboardPolicy` now; `lens-ui` injects a persisted impl later). Executed
      subagent-driven: composer-2.5 per task + **codex `gpt-5.6-sol` high-effort** cross-family
      per-task reviews (user rule 2026-07-21: gpt-5.6 always via codex, never Cursor) + fix waves +
      Opus whole-slice. Security-critical ordering all in place: cap-before-clone (1 MiB, before any
      owned alloc), post-encode epoch-recheck (covers empty output), reject-not-truncate paste (1 MiB),
      OSC-52 reads never reach the callback, read-only gate on BOTH the immediate and deferred-warn
      paste paths. Plan `docs/superpowers/plans/2026-07-20-terminal-slice-2b-clipboard-paste.md`.
      **Documented deferrals (carried, not bugs):** (1) **always-warn-on-multiline** — the foreground
      has no live mode-2004 (bracketed-paste) snapshot until 2c, so 2b uses the safe over-approximation
      (warn on any multiline, suppressible via "don't warn again"); suppressing the warn while bracketed
      paste is active is deferred to 2c. (2) **Menu Edit→Paste (`OsAction::Paste`) not wired** — only the
      Cmd+V keystroke path is intercepted (`handle_key_down`); the app-menu paste command is a distinct
      gpui path deferred to `lens-ui` menu integration (the standalone demo has no app menu). (3) **empty
      OSC-52 write still mints a host prompt** (writes empty string on Allow) — left intentionally, since
      suppressing could drop a legitimate "clear clipboard" intent; the bounded pending map rate-limits nag.
      **Live paste rider PASSED (2026-07-21)** — `terminal_live` P5 round-trip against a live omnigent
      0.5.1 shell (ephemeral rider-shell bundle, zero LLM cost): real clipboard→bracketed-encode→PTY→shell
      echo→frame paint on a real macOS display (`terminal_live: P5 paste round-trip OK` / `PASS`). P5 drives
      the production `handle_paste` path; the Cmd+V OS-keystroke intercept stays hermetically proven by
      `real_cmd_v_keystroke_routes_to_paste_not_key_encoder` (FIFO sentinel). 2b is now hermetically + live-proven.
  - **▶ SLICE 2 (interaction) — RE-CUT TO SERIAL; 2a + C2 + 2d DONE (2026-07-20).** 2d (presentation)
    executed subagent-driven (composer-2.5 per task + grok-4.5 cross-family per-task reviews + fix waves +
    Opus whole-slice = SHIP); the Opus review caught a title-clear-vs-full-channel invariant divergence
    (per-task reviews structurally couldn't see it) → FIXED via tri-state latest-title slot. The end-of-slice
    real-window gate (run on a real macOS display) caught 3 latent bugs in the never-executed Task-4 harness
    (frame-clobber-by-sampler, sync-read-of-async-emit, dropped Subscription) → all fixed; production click path
    was correct. Both `presentation_realwindow` (click→OpenUrlRequest e2e) and `render_realwindow` (perf all
    in-budget; C2-era over-budget flag confirmed environmental) green. 2d slice `bdd8695..5e6f28b`. Design spec
    [`docs/specs/2026-07-17-terminal-slice-2-interaction-design.md`](./specs/2026-07-17-terminal-slice-2-interaction-design.md).
    **Task 0 DELETED — dissolved into 2a/2d** (serial removes the parallel merge hazard at the root; 2d edits 2a's
    committed code). Plans: `docs/superpowers/plans/2026-07-17-terminal-slice-2-{2a-input,2d-presentation}.md`.
    Phases **2a input/IME/focus/read-only → 2d OSC-output → 2b clipboard/OSC52 → 2c mouse**, serial on `terminal-ws`.
    **2a DONE**: all 6 tasks via composer-2.5 + per-task gpt-5.6 review + fix waves + broad-slice review; gate-green
    (both clippy configs, 77 lens-terminal + 162 lens-client tests, real-window keystroke validated). Reviews caught
    real bugs each task (macOS option-as-alt clobber, reversed epoch-revoke, Tab/Enter/Shift double-emit, worker
    epoch TOCTOU, reply-evicts-user-egress). **✅ Critical C2 CLOSED (2026-07-20)** via **per-transport egress
    channels** (`fd79d54..2921da8`; plan `docs/superpowers/plans/2026-07-18-terminal-c2-per-transport-egress.md`):
    the shared untyped `Vec<u8>` egress became a swappable typed `Sender<EgressFrame>` owned by the worker
    (in-order `SetEgress`), each bridge owning its own receiver → emitted residue stays on the OLD channel
    (drain-dropped on stop); + `access_epoch` bump on every teardown revokes un-encoded upstream input.
    **T4 hardening (option A):** grok/Opus reviews diverged → source-verified adjudication (Opus GO correct;
    grok's Criticals need a live old bridge, unreachable today since every teardown = bridge self-stop). C1
    (false EngineStopped) closed BY-CONSTRUCTION (`signal_stop` synchronous-before-swap + Disconnected-arm
    suppression); C2 (reply-source) NARROWED (inbound-arm re-checks `stop` before feeding) + **join-before-attach
    documented residual** (only needed if a live-bridge teardown path is ever added, e.g. `on_host_event` →
    server downgrade). Reconnecting-input-semantics decision resolved in 1d. Ledger `.superpowers/sdd/progress.md`.
    Architecture = Option A (single-owner engine + ONE ordered command stream). **Progress+notifications spike-resolved
    bucket-C (absent from pinned C ABI) → DEFERRED + parent matrix amended.** Plans: grok authored → Opus review →
    consolidated **gpt-5.6-sol-high** review ($1.67; 8 Critical + 11 Important) → folded + verified. **Merge-seam
    finding resolved via Task 0** (single-writer foundation; both plans collide on `VtEngine::new`/`WorkerChannels`/
    `build_frame`/`render` — see [[terminal-parallel-worktree-task0-foundation]]). **XTSHIFTESCAPE (2c) still open.**
    Memory [[terminal-slice-2-design-ghostty-precedent]]. **Execution handoff (self-contained):
    [`docs/handoffs/2026-07-17-terminal-slice-2-execution.md`](./handoffs/2026-07-17-terminal-slice-2-execution.md).**
  - **✅ SLICE 1d (convergence) — ALL CODE DONE (T1–T9), gate GREEN, AND live rider PASSED, subagent-driven (composer-2.5 build +
    cross-family review: Opus inline per-task + codex on T4 reconnect machine + codex whole-branch). 17 commits
    `f4d3080..39ee7b3` on `terminal-ws` (unpushed). Ledger `.superpowers/sdd/progress.md`.**
    - **Shipped:** bridge thread (Select multiplex attach↔engine, OutboundSaturated/AttachDisconnected/
      EngineStopped policy events); `TerminalRuntime` off-foreground teardown (panic-free unique-Arc stop);
      `open()` C2 background discover/attach + foreground `cx.spawn` wake sampler (durable policy_tx survives
      reconnect); close-code policy + single 30s `RetryWindow` + reconnect state machine (read-only downgrade
      → read-only reconnect; preflight GET; transient-retryable); resize-before-input on connect+reconnect;
      retained-engine reconnect-seed acceptance (viewport Frame equality + fail-closed scrollback delta);
      `TerminalInspect`; standalone GPUI demo (handshake-before-GPUI); live rider harness (real-window,
      4-phase: attach→painted-marker→abort→Reconnecting→reattach+output_gap).
    - **Reviews earned their keep:** codex T4 caught 2 Critical (reconnect race + foreground-block) + 4
      Important → hardened (foreground bridge-spawn eliminates the race; single window; read-only reconnect).
      codex whole-branch caught 6 Important integration issues (engine-death stuck-Live; initial resize
      skipped; rider false-green; abort machinery leaking into production; inspect not durable across
      reconnect) → all folded.
    - **✅ LIVE RUN DONE (2026-07-17) — rider PASSED all 4 phases vs omnigent 0.5.1** (`tests/terminal_live.rs`,
      `--features live-tests,test-util`, real GPUI window): P1 attach→Live · P2 `echo LENSMARK_<pid>\r` to a
      real **zsh** PTY + proved a PAINTED frame carried the marker · P3 abort-attach→Reconnecting · P4
      reattach→Live+`output_gap`. **Both OMNIGENT-OPERATIONAL blockers from the prior attempt resolved (never
      rider bugs):** (1) the design Q is ANSWERED — a declared `terminals:` entry spawns a real tmux-backed
      bare shell (default `command`) SEPARATE from the agent TUI (`runner/app.py:16228`), gated server-side on
      the agent spec's `terminals:` block (`sessions.py:17565`); no billable turn; (2) the recipe = an ephemeral
      `rider-shell` agent bundle declaring `terminals.shell.command: zsh`, launched `omnigent run <bundle>
      --server` (spawns its own runner, `sleep`-held stdin keeps the REPL alive), rider `open()` open-or-creates
      the declared PTY. Full recipe + yaml-shape gotchas in memory [[omnigent-terminal-attach-live-run]]; the
      throwaway session was torn down, server+host daemon left online for future riders. Handoff
      [`docs/handoffs/2026-07-17-terminal-slice-1d-live-run.md`](./handoffs/2026-07-17-terminal-slice-1d-live-run.md).
      Merge `terminal-ws` → user's call (staying on branch through the rest of the workstream).
    - **Deferred (roll-up in ledger):** 1b engine flake `hidden_tab_suppresses_publish_until_visible`
      (pre-existing, timing under parallel load); EngineHandle::spawn readiness result (1b); thread-exhaustion
      foreground-panic extreme edge; per-task lens-terminal clippy should include `--features test-util`
      (missed T3's render_realwindow break for ~6 tasks).
  - **✅ SLICE 1c (render) — DONE, `xtask gate` GREEN on `terminal-ws` (unpushed).** T1–T7 correctness
    (`874c817`→`ae12b8b`) + perf-block resolution (`63f490f`→`f4f0d15`). Real-window harness
    (`tests/render_realwindow.rs`, `harness=false`, `test-util`-gated, xtask executes on macOS **in
    `--release`**): **Menlo gate PASSES**; `Frame` paint (PerRow/PerCell routing + debug-guard that
    PerRow never gets a wide cell), full SGR + `underline_quad_color` (I10a) + invisible-width (I10b),
    render Inspect ring; `TerminalTab` renders via shared `TabRenderState::render_element` (I6). Clippy
    clean `-D warnings` (normal + `test-util`).
    **2 plan deviations (both user-approved, in commit msgs):** (1) `render_test_api` gated on
    `test-util` not `cfg(test)`; (2) Menlo gate drops the emoji's-own-left-edge probe (real hardware:
    ASCII grid/box-drawing/CJK/post-emoji-resync all exact; emoji left edge drifts 2.857px under
    per-row shaping, which the renderer never uses for emoji — wide rows → per-cell).
    **⛔→✅ The "per-cell perf block" was a DEBUG-BUILD ARTIFACT.** The gate measured the harness in
    **debug** (~5.4× slower per-cell than release, from unoptimised `Font` clones). The 8.3ms budget is
    a 120fps *product* target; in **release** (what ships) every workload meets it: dense-wide 200×50
    **2.5–3.7ms**, 400×100 **4.8–6.2ms** (beats the *absolute* 8.3, not just the 20ms interim),
    pathological **2.4–3.3ms**. Shaping is ~0.06ms — the recommended per-glyph shape cache / **C-a
    reopen is RETIRED** (never the bottleneck). Fixes: gate now runs `--release` (`b1cc3e2`);
    resolve-once-per-row cleanup (`ad7e049`; **codex/gpt-5.5 review caught a per-cell decoration
    paint-order flip → fixed `730fc83`**); release-calibrated per-phase budgets (ascii 3.0 / wide-200
    5.5 / wide-400 8.0 / pathological 6.0ms — carry ~30% gate-load margin so no flap). Plan:
    `docs/plans/2026-07-16-terminal-slice-1c-perf-resolution.md`.
    **✅ Engine gate flake FIXED.** Both `engine::handle` flakes
    (`build_failure_retries_on_next_pump` + `stop_publishes_final_frame_before_join`) shared ONE root
    cause: a process-global `static TEST_BUILD_FAILURES` the fault-injection test set, which any
    concurrent test's worker consumed (its own build then wrongly succeeded; bystanders lost a frame).
    Moved injection to a per-handle `Arc<AtomicUsize>` shared only with that handle's worker — no
    cross-test contamination, zero-cost in production. 120/120 clean stress runs.
  - **✅ SLICE 0 (surface freeze) DONE + merged** (`fdba839`→`635eaa7`): froze the opaque public
    names/seam invariants `lens-ui` binds to (`open`/`TerminalTarget`/`AccessIntent`/`TerminalKey`/
    `TerminalOpenOptions`, 7-variant `Lifecycle`, `TerminalHostEvent`/`TerminalEvent`, opaque `Frame`,
    `focus_handle`/`presentation`) + `lens-terminal` & `lens-terminal-demo` crate skeletons. **codex
    (gpt-5.5) review** caught 3 evolvability issues, folded: `Lifecycle` = permanent payload-free
    `Copy` tag (detail rides `Presentation`); `TerminalOpenOptions` `#[non_exhaustive]`+`with_*`
    setters; `Frame` dropped `Clone`/`Default` (shared as `Arc<Frame>`).
  - **✅ SLICE 1a (`lens-client` transport) DONE + merged** (branch `terminal-1a`, 8 commits
    `0f7f23a`→`9e7b16f`): typed REST CRUD (`Terminals` subservice — **superseded** the dead
    Value-leaking `create_terminal`/`delete_terminal`/`transfer_terminal` wrappers, no callers), WS
    attach on a **contained tokio current-thread runtime + tokio-tungstenite** bridged to sync via
    bounded crossbeam queues (NO `transport=`), typed frame codec, close-code **classification**
    (4404/4405/4500/1008; policy deferred to 1d), gated `AttachInspect` ring, benches, feature-gated
    live rider (create/attach/input/resize/delete + 4404). **gpt-5.6-sol review** caught 6 real
    issues, all fixed (`9e7b16f`): `close()` deadlock on outbound saturation, unbounded connect/close,
    **inbound-saturation `Closed` signal lost** (now drops the Sender → guaranteed `Disconnected`),
    **queue bench deadlock**, `bench_api` `Message` leak (now typed), silent runtime-build failure.
    162 lib tests. Plan `docs/plans/2026-07-16-terminal-slice-1a-lens-client-transport.md`.
  Original design ([`specs/2026-07-14-terminal-workstream-design.md`](./specs/2026-07-14-terminal-workstream-design.md))
  assumed **porting Ghostty VT source** via the gpui-ghostty wrapper (adopt/adapt/exclude inventory,
  WP0 provenance gate). **That model is now superseded.** This session:
  - **WP0 plan revised** through Opus review (B1–B7) + **5 rounds of gpt-5.6-sol** review, committed
    (`73738d5`); then **executed Tasks 1–4** subagent-driven (composer-2.5 + cross-family):
    xtask verifier + CLI (17 tests, `db5a0b4`→`354d405`), archive hashes (`d9b2194`), adoption
    inventory 45+742 (`5bb16ec`). **These are now built on the obsolete model — repurpose/discard.**
  - **Task 5 (Zig build probe) hit a macOS-26 wall → resolved:** vanilla ziglang.org Zig ≤0.15.2 can't
    link natively on macOS 26 (Xcode 26 bug, ziglang/zig#31658); the **Homebrew/Nix patched `zig@0.15`
    (0.15.2)** works. Ghostty **v1.3.1** `lib-vt` builds natively with it. Memory
    [[zig-ghostty-macos26-scissor]].
  - **DECIDED MODEL (memory [[terminal-vt-adoption-model]]), PROVEN on hardware:** the terminal C API
    (`terminal.h`/`screen.h`/`render.h`/`grid_ref*.h`) lives ONLY on Ghostty **dev** (release v1.3.1
    `vt.h`=7 parser headers; dev `a887df42`=29 incl. the full terminal surface). **[libghostty-rs]**
    (Uzaaft, MIT/Apache) already binds it — `libghostty-vt-sys` (checked-in bindings) + `libghostty-vt`
    (safe `Terminal`/`vt_write`/`RenderState`/`Cell`/scrollback), pinning Ghostty dev in `build.rs`.
    **VERIFIED:** builds on macOS 26.6 w/ patched `zig@0.15` in 33s, 29 tests pass, example drives a
    real terminal. **Model = VENDOR libghostty-rs + BUILD FROM SOURCE** (patched zig prereq;
    ~25-33s cached build) — NOT a shim, NOT a source port, NOT prebuilt (no CI yet; prebuilt =
    flip-a-switch later). gpui-ghostty = reference only. WP0's provenance apparatus collapses to
    **dependency vetting** (pin libghostty-rs + Ghostty commit + license closure).
  - **✅ MECHANICAL EXECUTION DONE (2026-07-15, this session)** — commits `ae1f385`/`014f9a9`/`e155230`
    on `terminal-ws` (unpushed):
    - **Task 2 — vendored + wired + link-proven.** `vendor/libghostty-rs/` (2 crates @ `46a9d2ac`,
      684K); the Ghostty source is **crates-only, NOT vendored** — reversed the "pinned vendored
      Ghostty source tree" plan: its ~57-152M tree is the same large-artifact-before-CI anti-pattern
      we reject for a prebuilt `.a`, so the pin stays in `build.rs` (`a887df42`, blobless fetch,
      cached) and a `GHOSTTY_SOURCE_DIR` vendor is deferred to the **same CI trigger** as prebuilt.
      Wiring: crates **EXCLUDED** (not member) → Cargo cap-lints them (clippy `--workspace -D warnings`
      stays clean); `.cargo/config.toml` `ZIG`→keg-only `zig@0.15` + a 1-line `build.rs` patch. Proof:
      `spikes/libghostty-link` (bytes→cell); from-source build re-verified post-`cargo clean` (24.94s).
      Provenance + patch list: `vendor/libghostty-rs/README.md`. Memory [[terminal-vt-vendored-executed]].
    - **Task 1 — dead WP0 apparatus removed.** xtask terminal-provenance CLI/lib/tests/fixtures +
      toml/thiserror/sha2 deps; `vendor/gpui-ghostty-e3025981/`; generate-terminal-adoption.sh; WP0
      plan+review docs. codegen/drift/gate intact (`cargo test -p xtask` 2/2).
    - **Docs superseded** — banners on the source-port design + roadmap (VT-adoption + `--workspace`
      gate lines flagged dead; model-independent parts still hold).
  - **✅ DESIGN-PASS SPIKES DONE (2026-07-16, this session)** — both design questions answered;
    merged to `terminal-ws` (unpushed). Spec `docs/specs/2026-07-15-terminal-spikes-design.md`,
    plans `docs/plans/2026-07-15-terminal-spike-{a,b}-*.md`. Memory
    [[terminal-render-ptyattach-spikes-executed]].
    - **Spike A — render viability → VERDICT: full-snapshot repaint contract.** Standalone GPUI
      probe (grok-built, Opus+codex reviewed). S1 (reshape every row every frame, no cache) full-redraw
      p95 = **2.77 ms @ 200×50** ≤ 8.3 ms budget → Ghostty dirty-row tracking is **not** load-bearing;
      per-row `ShapedLine` cache (S2) barely helps (2.45 ms) → shaping isn't the bottleneck. Wide/emoji
      need per-cell glyph placement (per-row `shape_line` drifts). Liftable `paint.rs` kept +
      codex 3-item punch-list (findings `docs/spikes/2026-07-15-terminal-render-viability.md`).
      ⚠ p95 is paint-closure CPU only (no vsync/present) — re-measure end-to-end when building real.
    - **Spike B — PTY-attach contract → DOCUMENTED + LIVE-VERIFIED** vs omnigent 0.5.1
      (`docs/spikes/2026-07-15-pty-attach-contract.md`, corpus in `captures/2026-07-15-pty-attach/`).
      **Wire is transport-independent** (control=default & pty both deliver **raw VT binary**; tmux
      control-mode consumed server-side → **NO tmux parser in the client**). Attach `ws:// /v1/…/attach`,
      101 before terminal lookup, no auth on dev; input=binary bytes (also the `on_pty_write` back-channel),
      resize=JSON text; reconnect to same `terminal_id` = current-screen redraw, **no byte-replay**
      (transient gap); typed close codes 4404(stop, live-confirmed)/4405(detach)/4500(retry).
  - **✅ SLICE 1b (`lens-terminal` engine core) DONE + merged** (branch `terminal-1b`, 8 commits
    `376dd1c`→`8de30f7`): non-`Send` `VtEngine` on a pinned `std::thread`, Lens-owned `Frame` seam
    (no Ghostty types escape `engine/vt.rs`; full SGR carried for 1c), throttled publish-and-wake
    (`ArcSwapOption` + coalesced waker + `recv_timeout` throttle wake), DA/DSR reverse channel,
    hidden-tab suppression, gated `EngineInspect` ring, offline replay tests (`attach`/`resize`
    captures), Criterion benches (parse ~12µs / frame-build ~590µs @ 200×50). Composer self-ran
    cursor Bugbot (3 concurrency fixes). **grok-4.5 review** caught 4 publish/lifecycle edges, all
    fixed (`8de30f7`): `build_frame` `Err` dropped the dirty bit, `SetVisible(true)` no-force,
    `Stop` abandoned a dirty frame, `Drop` joined on the dropping thread (now detach; join via
    `stop()` only, off-foreground). 16 tests. Plan
    `docs/plans/2026-07-16-terminal-slice-1b-lens-terminal-engine.md`.
  - **Process (this session):** Slice 0 authored + reviewed inline (Opus); 1a∥1b built **in parallel
    isolated git worktrees** by **composer-2.5** (subagent-driven, per-task TDD commits), each
    cross-family reviewed by a **different family** (gpt-5.6-sol on 1a, grok-4.5 on 1b), fixes
    delegated back to composer, then merged to `terminal-ws`. **Full gate green:** `cargo fmt`,
    `clippy --workspace --all-targets -D warnings`, `cargo test --workspace` (lens-client 162 /
    lens-core 202 / lens-terminal 14+2 / all crates). `xtask gate` lists updated for the 2 new crates.
    ⚠ **unpushed** on `terminal-ws`.
  - **✅ 1c + 1d PLANS DONE (2026-07-16, this session)** — authored by **grok-4.5**, cross-family
    reviewed (**codex/gpt-5.6 + Opus** source-verification, **15 findings / 5 Criticals** folded),
    revised, Opus diff-verified, committed `f12a933` (**unpushed**).
    `docs/plans/2026-07-16-terminal-slice-1{c,d}-*.md`. Criticals fixed: `#[gpui::test]` NoopTextSystem
    false-green → real-window `harness=false` gate (memory [[gpui-test-noop-text-system]]);
    off-thread→entity wake impossible → `cx.spawn` sampler + `async_channel`; `TerminalRuntime`
    teardown-ownership; reconnect-seed acceptance (leg-2 seed + scrollback probe); fail-closed perf
    gate xtask-executed on macOS. **NEXT = BUILD 1c → 1d (sequential, 1d needs 1c)** — see
    **[`docs/handoffs/2026-07-16-terminal-slice-1c-1d-build.md`](./handoffs/2026-07-16-terminal-slice-1c-1d-build.md)**
    (self-contained driver for a fresh session).
  - **Plan detail (build):** **1c** full-snapshot render layer (lift `spikes/terminal-render/src/paint.rs`
    split at the `collect_rows`/paint seam — engine already produces the owned `Frame`; apply the 3
    codex fixes + per-cell wide/emoji + **full SGR** the `Frame` now carries + **gate system `Menlo`**
    live-GPUI resolution/alignment, bundle-font fallback) → **1d** convergence (wire `open()`/
    `TerminalTab`/`presentation()`, transport↔engine bridge, `cx.notify` waker, close-code **policy**,
    lifecycle subset + gap marker, retained-engine-seed acceptance test, standalone GPUI demo, live
    proof vs omnigent 0.5.1). 1c needs 1b (done); plan 1c/1d against the now-landed APIs.
    GPUI 0.2.2 + omnigent 0.5.1 pins unchanged.

- **▶ Board B-2..B-6 (board-home)** — §4 board is now decomposed into **six specs B-1..B-6**
  (`docs/SPEC-GAPS.md` → "Board (§4) implementation specs"; supersedes the old B6/B7/B8 framing — B7
  "stable ordinal ordering" dissolved into B-1's ordinal slots, no separate sort task).
  **B-1 (data model & persistence) shipped 2026-07-18** (`8100cc8`; `lens-core` `BoardLayout` tree +
  `SqliteBoardStore`, schema v3). Remaining, in dependency order:
  - **B-2 — packing/scroll/culling SHIPPED 2026-07-21** (`db5b7c2..14b474c`, 10 commits, merged to
    main **unpushed**). `lens-core::pack` pure packer (`foot`/`pack`/`cols_for_width`/`intersects_band`);
    `BoardLayout::board_tree` ordered group-aware walk (skips archived); `lens-ui` absolute-masonry
    `overflow_scroll` container (both board N-col + focus rail 1-col via one `pack_and_render`) with
    band-culling; **container-driven visibility gate** (cards init HIDDEN, `set_visible` via `App::defer`)
    that **retired** the paint-time `last_bounds` gate + `recover_viewport_gates_on_reentry` + `last_mode`
    and fixes the scroll/re-entry freeze at the root. **Basis B (locked):** the packer walks an in-memory
    `BoardLayout` fabricated from `FleetStore` by a PROVISIONAL `build_ephemeral_layout` stub — B-4 deletes
    it when it lands the persisted store→replica seam with the first writes. Subagent-driven build: 6 tasks,
    cross-family review each (codex gpt-5.6), Opus whole-branch review **READY**; `xtask gate` green;
    release demo launches clean (live gate confirmed: animating cards tick, Slept frozen). Memory
    [[board-b2-executed]]; plan `docs/plans/2026-07-21-board-b2-packing-scroll-culling.md`; handoff
    `docs/handoffs/2026-07-21-board-b2-executed.md`.
  - **B-3 — group chrome & rollups SHIPPED 2026-07-21** (`3045590..75b78bb`, 7 commits, merged to main
    **UNPUSHED**). Filled the B-2 placeholder arm with real chrome: `board/rollup.rs` pure `GroupRollup`
    fold (Σspend / oldest-`created_at` age / `completed_count`) + formatters; `group_accent` token→color
    resolver (4 SSOT accents + neutral); `absolute_group` renders ring+accent+7%-tint + header-lane
    (`● dot · name · spend·age · ✓N · ⌄`) folded from member cards; `created_at` plumbed onto `SessionCard`
    (Detailed/Rebased); `test_layout` injection seam + `group_chrome_for_test` hook drive a fixture
    integration test (the group path is not runtime-reachable under basis B). Subagent-driven (composer-2.5
    implementers, codex gpt-5.6 cross-family review of board logic = clean + 1 Minor age-overflow fixed,
    Opus whole-branch review = **SHIP**). `xtask gate` green. **Design deviations from the spec bullet:**
    (a) the `group_of(&SessionCard)` seam was NOT built — group membership is threaded as `GroupMeta`
    through `pack_and_render` from the `board_tree` walk, so a card-keyed reverse lookup is unnecessary;
    (b) `✓N` renders `completed_count: 0` — the real Archive-side count wires in **B-6**. Plan
    `docs/plans/2026-07-21-board-b3-group-chrome-rollups.md`. **3 Minors carried into B-4** (Opus review):
    the render-dead `group_header_text`/inline-header duplication (add a live rendered-chrome assertion,
    render from one source); the integration test proves data-wiring not pixels (correct under
    NoopTextSystem — B-4 adds the live check); spec §3 fidelity nits (`.border_1()` 1px vs 1.5px; flat
    wash vs glow/vignette — gpui 0.2.2 has no radial gradient, [[wave-card-body-wash]]).
  - **B-4 — drag/move + context-menu grouping — decomposed into B-4a…B-4d.** At design time B-4 was split
    into a **foundation slice B-4a** (store→replica write-path; NO interactions) + interaction follow-ons
    B-4b (collapse + §7 collapsed-tile) / B-4c (drag/move) / B-4d (context-menu grouping).
    - **B-4a — store→replica write-path foundation — EXECUTED 2026-07-22** on branch `board-b4a`
      (base `0f18ea7`, 20 commits). Plan `docs/plans/2026-07-22-board-b4a-store-replica-write-path.md`
      (v2, codex-reviewed REWORK folded). Subagent-driven: composer-2.5 implementers + codex gpt-5.6
      per-task cross-family review + final whole-branch review + Opus controller adjudication. `BoardReplica`
      (`board/replica.rs`, ~930 lines) = in-memory `BoardLayout` + serialized single-in-flight off-thread
      `run_op` pump (`cx.spawn`→`background_executor().spawn`→`WeakEntity::update`), typed BUSY-retry w/
      backoff, recovery force-reopen, suppress-stuck reconcile (no tombstone loop), deterministic
      session-sorted placement; retired `build_ephemeral_layout` + `test_layout` seam; non-blocking
      `ReplicaState` banner; demo seeds a "Demo group" (B-3 chrome live). **Reviews caught (all fixed):**
      ~10 false-green tests (composer blind spot → controller load-bearing rewrites w/ sabotage-verify),
      the C1 tombstone infinite-loop (self-introduced by re-diff-on-reply), a buggy `gate_epoch` composer
      over-reach for a non-existent race, and non-deterministic HashMap placement (flaky acceptance).
      **Perf:** `board_tree` bench 11.8µs @ 1000+group; at-scale demo (`LENS_DEMO_N=125`) launches stable;
      **Frame-budget E2E MET 2026-07-22** (user ran LENS_DEMO_N=125 ≈1000 cards on a display → smooth ~120fps; logs confirm no whole-board storm). **Cull-at-scale residual CLOSED 2026-07-22** (`5de3b93`): two sabotage-verified regression tests prove off-screen (culled) animating cards hold NO anim timer — `card::view::hidden_animating_card_holds_no_timer` (atomic drop transition) + `board::culled_animating_cards_hold_no_timer_at_scale` (150-card end-to-end: live-timer set == visible band exactly across a deep scroll). No production change; drop mechanism was already correct. Gate green (clippy -D warnings,
      fmt, lens-core 254 / lens-client 150 / lens-ui 83 lib + 5 acceptance). Memory [[board-b4a-plan-executed]].
      **MERGED + PUSHED 2026-07-22** (board-b4a FF→main, `4d31c9d..c189d4c`, incl. previously-unpushed B-2/B-3). NEXT interaction slices B-4b/c/d; **B-4d blocker:** non-idempotent-retry commit-phase tracking (design §8 seam). Original design spec:
      `docs/specs/2026-07-21-board-b4a-store-replica-write-path-design.md` — grilled + gpt-5.6 codex
      spec-review folded + §3 re-grilled. Replaces `build_ephemeral_layout` with a persisted `BoardLayout`
      via a new `BoardReplica` gpui entity; **off-thread store access** (`Arc<Mutex>` + `cx.background_spawn`
      behind a serialized single-in-flight `run_op`; renders read the in-memory replica, never SQLite) —
      the codex review caught that inline SQLite violates AGENTS.md's MANDATORY off-thread rule. Conn pinned
      to the app `Connection.id` (`"lens-app"`) so FleetStore placement converges with `load_layout`'s
      sessions-table reconcile; explicit `ReplicaState` (Loading/Writable/Degraded/LoadFailed/Stale) + always-
      allowed recovery `Load` + non-blocking banner; demo seeds a group (B-3 chrome renders live for the first
      time); MANDATORY frame-budget benchmark = E2E lens-ui on-device measurement (not just the pure lens-core
      pack bench). Verifies the B-3 `.cached()` member-read-during-render carryforward now that groups render
      for real. Memory [[board-b4a-design]]; handoff `docs/handoffs/2026-07-21-board-b4a-design-locked.md`.
    - **B-4b — collapse toggle (+§7 collapsed-tile) — EXECUTED 2026-07-22** on branch `board-b4b`
      (has `main`/terminal merged in; **NOT merged — held for a board visual pass**). Subagent-driven
      (composer + grok-4.5 per-task + codex final), all reviews clean. Caret-only `⌄`/`▸`, commit-gated
      `SetCollapsed` op, collapsed 1×1 rollup tile, unified `✓N iff N>0`, deadline-wake for stale
      collapsed time-waves. Plus an on-device **ring-gutter fix** (`62c8951`,`3801ce1`): the NeedsInput/
      Failed expanding attention ring leaked past the group border; trimmed `RING_REACH_PX 12→6` +
      `board::GUTTER = reach+1 = 7`, pinned by two compile-time `const _` asserts (`GUTTER>=reach`,
      `2*GUTTER<=GAP`). Demo now seeds two adjacent colored groups + dark default. Gate green
      (98 lens-ui lib tests, clippy, fmt). **⏳ NEXT: a full board visual pass** (on `board-b4b`) then
      foreground gate → merge. **Two seed issues + branch state:** handoff
      `docs/handoffs/2026-07-22-board-visual-pass.md` — (1) group bg fill 7%→maybe 10–12%; (2) groups
      don't reflow in the focused rail (`pack.rs:102/111` stores unclamped `fc`; pre-existing).
    - **B-4c — drag/move — SPIKE DONE (2026-07-23, `spikes/board-drag/`, verdict GO).** De-risked the
      reverse hit-test (cursor → `(parent, ordinal)`) over the forward-only masonry. Findings:
      `docs/spikes/2026-07-23-board-b4c-drag-reverse-hittest.md`. (1) **gpui mechanics = low risk** —
      `0.2.2` ships `on_drag`/`on_drag_move`/`on_drop`/`can_drop`, drop dispatch is **hitbox-based**, so
      absolute-masonry positioning is orthogonal (real-window proof deferred to the build). (2) **Write-path
      = low risk** — domain `move_item(id, board, parent, ordinal)` already covers reorder + in/out-of-group +
      cross-board; needs only a new `Op::MoveItem` mirroring `SetCollapsed`. (3) **Reverse hit-test crux:
      masonry is forward-only + ordinal order is NOT spatially monotonic** → no closed-form inverse; resolver
      **scans the placed tiles** (into-group body = clean grid; top-level = nearest-tile + reading-order
      side). Pure `resolve_drop` + `to_move_ordinal`, **10/10 tests** incl. the non-monotonic backfill case.
      **codex (gpt-5.6) cross-family review** confirmed all geometry, caught 1 Medium (partial-last-row empty
      trailing cell resolved before-last-member instead of append) → fixed + regression test. **Open design
      decisions surfaced for the brainstorm:** insertion convention (nearest-tile vs marker vs reflow-preview;
      lean = live reflow-preview), no natural end-of-list target (append when `cursor.y > content_height`),
      group body-vs-header semantics, edge auto-scroll. **DESIGN LOCKED + GRILLED 2026-07-23**
      (`docs/specs/2026-07-23-board-b4c-drag-drop-design.md`, `a78a640`; 4 decisions folded). **PLAN
      WRITTEN + OPUS-REVIEWED 2026-07-23** (`docs/plans/2026-07-23-board-b4c-drag-drop.md`, 7 tasks;
      grok-4.5 author + Opus source-verify). **EXECUTED + TASK-6 DEVICE-VERIFIED 2026-07-23** on branch
      `board-b4c-drag` (`4708ffe`..`f84d340`; Composer-authored, Opus-reviewed; full `xtask gate` GREEN
      incl. real-window stream_perf + no drift). Tasks 1–5,7 landed to plan (store `move_item` + §7 clamp
      were already present as the plan review predicted). **Real-window verify caught 5 bugs the tests
      structurally cannot** — all fixed + confirmed on device: (1) app-breaker — drag handlers must bind
      to the VIEWPORT, not the tightly-sized content box, or drops outside the tiles strand the drag;
      (2) drag image is the LIVE `SessionCardView` entity (not a ghost stub); (3) collapsed groups
      draggable; (4) into-group insertion opens a real gap (member shift + downward-only grow);
      (5) flicker = reflow feedback loop via the COORDINATE TRANSFORM — froze the block centering on the
      committed layout during a drag, not just the resolver (§4.1). Accepted tradeoff: `INTO_GROUP_INSET`
      = 24px makes end-of-group drops need precise aim (one-const tune). **MERGED to `main` `92d5001`
      2026-07-23** (clean ff after `main`→branch reconcile — Cargo.toml auto-merged, toolchain bumped
      to 1.95.0 with main; full `xtask gate` GREEN on the merged tree under 1.95; UNPUSHED). Memory
      [[board-b4c-drag-spike]]. **NEXT: B-4d** (context-menu grouping, gated on the §8 non-idempotent
      seam) or the deferred foreground-handshake terminal task.
    - **B-4d — context-menu grouping.** Still **gated** on the non-idempotent-retry commit-phase seam
      (design §8) that B-4a deferred — drag-to-*group* is B-4d, not B-4c. Adds `write()` op variants via
      B-4a's `run_op` seam.
  - **B-5 — multiple boards + rail switcher** — board CRUD (B-1 seeds only the default board), the
    externally-discovered-session landing policy, and `FleetStore` connection-scoping.
  - **B-6 — archive-as-board surface.**
  - **Wiring gap (partly closed by B-2):** `BoardView` now reads a `BoardLayout` (via `board_tree`) and
    renders from it — but under **basis B** that layout is the ephemeral `build_ephemeral_layout` stub, NOT
    the persisted `SqliteBoardStore`. The real store→replica wiring (spec §6) + all board **write** paths
    ride with **B-4**, which deletes the stub.
  - **Freeze RESOLVED by B-2:** the scroll-into-view / focus↔board re-entry freeze is fixed at the root —
    the container-driven visibility gate (cards init HIDDEN, `set_visible` via `App::defer`) replaced the
    paint-time `last_bounds` gate + `recover_viewport_gates_on_reentry`. [[viewport-reentry-freeze]] closed.
  - Grounding: specs `2026-07-18-board-data-model-persistence-design.md` (B-1) +
    `2026-07-20-board-packing-and-group-rendering-design.md` (B-2+B-3); handoff
    `docs/handoffs/2026-07-20-board-b2-b3-design-and-spike.md`; memories [[board-b1-executed]],
    [[board-b2-b3-design]].

- **▶ `lens-ui` transcript fan-out** — the first real consumer of the Detailed feed + the disk
  `RowSource`/D23 render window (markdown + virtualization spikes both GO). Plugs into the slot API the
  shell skeleton publishes; sibling parallel surfaces (terminal via `lens-terminal::open`, workspace,
  permissions) can fan out against `ContentTab`/`TabHandle`. Product design is **complete**
  (`docs/design/conversation-transcript.md`, §1–§21); this workstream is **implementation decomposition +
  gpui/lens-ui specifics**, not product design.
  - **Decomposed 2026-07-20 (brainstorm) into build-slices T-1..T-7** (resliced 2026-07-21: sub-agent
    span promoted to its own slice T-5 — it's a child-*session* feature, not a depth transform — pushing
    turn-lifecycle→T-6, composer→T-7), each its own brainstorm→spec→plan→build cycle, dependency order
    below. Slices are *internal build increments* (like Board B-1..B-6) — the **surface** is not declared
    done until its closer lands. Two real surfaces fall out: **History view** (read-only transcript, no
    composer — §18, used for archived/sleeping sessions) is complete after **T-6**; **Chat column (full)**
    is complete after **T-7**. No functionality is deferred *out* of the workstream — the earlier
    "composer/interrupt/permissions belong elsewhere" framing was the error and is corrected: they are
    **T-7**, in-scope.
  - **T-0 — Authoritative turn identity (lens-core / lens-client). ✅ DONE 2026-07-21**
    (`c8e0c63..d6c7e4f` on `lens-transript`, unmerged; plan
    `docs/plans/2026-07-21-transcript-t0-turn-identity.md`, design
    `docs/specs/2026-07-21-transcript-t0-turn-identity-design.md`). Server **`response_id`** is now the
    single authoritative turn signal: lens-client retains it on `stream::Item` + `ResponseEvent::InProgress`;
    catch-up maps it (was hard-coded `turn:0`); `BlockContext.response_id: Option<ResponseId>` **replaces**
    `turn: u32`; live items are stamped with their **own** wire id (synthesized items fall back to the new
    `SessionState.active_response` scalar, never fabricating); `response.in_progress` sets `active_response`
    + emits `StreamUpdate::ActiveResponseChanged`, cleared on every terminal `response.*`; persisted
    additively (SCHEMA_VERSION still 3, legacy `turn` col kept, written 0) + promoted in reconcile. Executed
    subagent-driven (composer impl + codex gpt-5.6 cross-family review per task + Opus synthesis); full gate
    green. **Live rider PASSED** (`crates/lens-core/tests/t0_live_rider.rs` replays real 0.5.1 SSE through the
    built stack; plus a fresh `/items` drift-drive re-confirmed `response_id` present / `created_at` null).
    **Descoped by evidence:** real `created_at`/durations → **T-6** (null on `/items`, snapshot-only, epoch
    **seconds**); the `stream.turn` non-completed Ready-counter bug is a **separate** Board handoff, not T-0.
    **Unblocks T-1** (a real `active_response` signal now exists; transcript replica *consumption* = T-2).
  - **T-1 — ViewBlock projection pipeline (pure). ✅ DONE 2026-07-21**
    (`crates/lens-core/src/reduce/view.rs`; plan
    `docs/plans/2026-07-21-transcript-t1-viewblock-projection.md`, spec
    `docs/specs/2026-07-21-transcript-t1-viewblock-projection-design.md`). §3/§4. Pure staged pipeline over
    `&[Item]` + `StreamScratch` → `Vec<ViewBlock>`; new `reduce/view.rs` in **lens-core**; exhaustive
    `ItemKind` match; no gpui, 21 inline table-driven tests, `xtask gate` green. The spine. Built via
    composer-2.5 + codex (gpt-5.6) cross-family review — 2 findings fixed (reused-`call_id` exactly-once;
    ResourceEvent sibling test). **Unblocks T-2..T-7** (all render off `Vec<ViewBlock>`). Key resolutions: staged
    (filter→project→group) not uniform pipe; turn identity = authoritative **`response_id`** (from T-0),
    NOT a `scratch.turn` heuristic; `group_work_section` groups agent work by `response_id`, user messages
    + non-response items are ordinal-positioned siblings; liveness = turn's `response_id` == session active
    `response_id`; `WorkSection` drops `open` (render owns) and drops `meta` entirely (all fields need
    per-turn data → **T-6**); streaming variants carry `&MessageAcc`/`&ReasoningAcc` (stable identity);
    **`OptimisticUser` dropped** (pending is composer-owned → T-7); **`SubAgentSpan` dropped**
    (child-session model → T-5); `ReconnectBreak` emission → T-2.
  - **T-2 — Focused view scaffold + live disk-sourced surface. ▶ EXECUTING 2026-07-22 (13/15 tasks done, gate-green, on `lens-transript` c53179f..2886508, UNMERGED).**
    Subagent-driven (cursor composer-2.5 impl · codex gpt-5.6 + grok-4.5 + Opus reviews). **Phase A (Tasks 1–6) DONE.** **Phase B: 7–13 DONE, 14–15 not started.** Progress ledger + full per-task detail: `.superpowers/sdd/progress.md` (RESUME THERE). Reviews caught & fixed real defects: 2 Criticals (OutputItemDone-supersede orphan; reader-worker channel-drop/foreground-open), coalesce-drops-keyed-signals (Tasks 4/5), latent lens-store break, staged-finalize crux bugs (Task 12, 3 rounds), Task-13 scroll follow-mode bug (visible_range pre-scroll vs is_scrolled post-scroll — codex). **Both crux + scroll real-window proofs PASS** sandbox-disabled (probes now exit trustworthy codes — see [[gpui-list-scroll-and-realwindow-probe-gotchas]], [[t2-real-window-probe-sandbox]]). **Task-12 collapse nuance VALIDATED non-defect** (2 locking tests). **OPEN:** Tasks 14 (`ReconnectBreak` marker) + 15 (`syncing…` debounce + release perf gate + Opus synthesis), flaky Task-7 mock-handshake test (CI reliability), Minors (rusqlite-in-lens-ui layering, lens-store gate-scope gap).
    _(orig plan context below)_ PLAN-COMPLETE 2026-07-22
    (plan `docs/plans/2026-07-22-transcript-t2-focused-view-scaffold.md` — 15 tasks, Phase A lens-core
    (Tasks 1–6, exact code) + Phase B lens-ui (7–15, spike-referenced); handoff
    `docs/handoffs/2026-07-22-transcript-t2-plan-complete.md`; spec rev 4
    `docs/specs/2026-07-21-transcript-t2-focused-view-scaffold-design.md` incl. **D-3 refinement to per-run
    sections**). Four spec-deferred mechanism items resolved in-plan: (1) **per-run `(response_id, run_index)`
    sections**, chronological order preserved (real `claude-native-todos.sse` shape), collapse flag per-response;
    (2) `Retired{acc_id, Finalizing{item_id}|Discarded}` at Completed/terminal/reconnect; (3) live re-projection
    index `live_section_start`; (4) **re-fire → precise `TranscriptRewritten{ordinal}` signal** (3-signal
    actor→replica disk contract: append/in-place/coarse-reconcile). Three gpt-5.6/codex review rounds (design); all
    mechanical/plumbing findings closed; the three hard decisions resolved w/ user — **D-3→A′**
    (WorkSection-from-birth, two-level retained entities, finalize = render-flag flip, no remount; needs a
    T-1 amendment: response-keyed uniform grouping incl. live), **D-1→z** (cache settled sections, per-response
    live projection, coarse invalidate-on-reconcile), **D-2→ii** (reducer `Retired{item_id|Discarded}`).
    **NEXT: execute the plan via subagent-driven-development in a fresh session** — start Task 1 (T-1
    amendment). Phase A (Tasks 1–6) high-confidence (verbatim lens-core code); Phase B (7–15) lifts the
    `transcript-virtual` spike, needs real-window iteration. §16/§17. **First real consumer of `Vec<ViewBlock>`.** Mount focused view in `#chat-slot`
    via a `focused_transcript_tab(replica) -> TabHandle` factory (`ContentTab` left an inert marker — protocol
    deferred, SPEC-GAPS); a **store-side `FocusedTranscript` replica** created on `Promote`/dropped on `Demote`,
    fed the detailed frames by the **existing single poller fanning out** (no channel tee — the feed is
    single-consumer); replica opens a **2nd read conn** to `{session_id}.db` (WAL), baseline = full `load_items`,
    steady-state = **forward-delta ranged read** `(last_rendered, committed_ordinal]` on `TranscriptAdvanced`
    (one small new `TranscriptStore` primitive); live tail from `ScratchChanged`; liveness from
    `ActiveResponseChanged`; `Rebased`(scalars-only) refreshes scalars, **never** clear-reloads items; lift
    `RowSource`/`RowStore` from spike (id-keyed upsert, flash-free finalize — the two-id-space hazard is
    handled, mandatory EntityId-stable test); native `list()`/`ListAlignment::Bottom`; four §16 scroll
    contracts + "↓ N new" pill; `ReconnectBreak` = replica-injected synthetic marker on `Reconnected`-with-gap.
    Renders every `ViewBlock` variant as **stubs** for T-3/T-4 content. **Bucket-C dep already satisfied**
    (`GET /items` pagination ships in lens-client). **Descoped → T-2b.**
  - **T-2b — Disk windowing, scroll-back paging & bounded-tail reconcile. (next after T-2, NOT deferred).**
    The [[large-transcript-latency-spike-2026-07]] scale primitives on `TranscriptStore`: swap T-2's full
    `load_items` baseline for a **byte-budgeted tail window**; add **backward** page-load (`WHERE ordinal < ?
    ORDER BY ordinal DESC LIMIT ?`) for scroll-back; scope reconcile to the **resident tail** (full-history
    reconcile is a >1s stall on multi-day sessions). Only needs T-2's RowSource; makes multi-day sessions
    correct. Independent of content rendering (T-3/T-4).
  - **T-3 — Message & reasoning content.** §5/§6/§7. Vendor+patch gpui-component markdown (3 spots:
    debounce reset, `clear_selection` on reparse, `list_state.reset` scroll-jump); markdown-vs-verbatim
    channels + user backtick-gating; sanitization pre-pass; streaming safe-prefix / stable identity;
    reasoning + capped scroll region.
  - **T-4 — Tool spans, native tools, resource markers.** §8/§9/§12. Tool-span render (archetypes,
    truncation tiers, inline edit diff); native tools; §12 inline resource markers. **Bucket-B stubs
    live here** — "show full → editor/Review", "dock to Canvas", "open terminal" render **inert/disabled**;
    **no invented inline fallbacks** (they'd be ripped out by the real surfaces). **+ live in-progress
    tool-tail feed extension** (moved here from T-2 2026-07-21): in-flight `FunctionCall`s sit in the actor's
    above-watermark working set and are **not** carried by today's feed (scratch has only
    `open_message`/`open_reasoning`); shipping them so a running tool renders live before its output is a
    lens-core actor/feed change, and it belongs where tool-span render lives — not T-2.
  - **T-5 — Sub-agent spans (child-session model).** §8.6. Sub-agents are child *sessions*
    (`session.child_session.created/updated`, linked by `parent_session_id`), **not** `ctx.depth` items —
    so this is a real feature, not a T-1 transform. Reducer folding of `child_session.*` into a
    parent↔child registry + live status; project `SubAgentSpan` at the spawn point; §8.6 render (collapsed
    span, peek, output-in-transcript); **navigate-into-child** shares the shell's session-focus machinery
    (the one cross-surface seam). Reuses T-4's span/output render. Prereq: reducer child-session fold.
  - **T-6 — Turn lifecycle, compaction, agent-changed, todos, minor items.** §4/§10/§11/§13/§14.
    Work-section collapse lifecycle (expand/override state — T-1 emits no `open`); the whole
    `WorkSectionMeta` (duration/model/tokens/cost/agent-transitions — T-1 emits none). **Prereq for the
    chip's model/token/cost:** model `response.completed.response.usage` — per-turn usage/model IS on the
    wire (`openapi.json:2573+`) but `ResponseEvent::Completed` is currently a unit variant that discards
    it; retain it per-turn. Compaction marker, AgentChanged marker, inline todos (forms 1–3), minor items,
    reconnect break. **← History view complete here.**
  - **T-7 — Composer & complete live turn (the chat closer).** §15/§18. Always-sends composer; optimistic
    user bubble (`⋯ sending` → settle on `session.input.consumed` → `⚠ failed·retry`); **Esc-interrupt**
    (+ new lens-core `Interrupt` command + lens-client call — server already echoes `session.interrupted`/
    `response.incomplete`); **permission/elicitation dock + widget integration** (reuse the GO elicitation
    spike; round-trip binary/form/url/plan/codex; emit `approval{action,content}` — **this workstream owns
    the integration**); **send-recovery** (never drop send text) + **input history** (up/down).
    **← Chat column (full) complete here.**
  - **Carry-forward arch notes:** a Summary-mode card consumer MUST tolerate occasional
    `Detailed(TranscriptAdvanced)` watermarks (catch-up/deferred-commit emit them regardless of mode).
    §3.5 Ready *policy* (seen_turn detector / `last_completed_at` stamp / per-card decay one-shot /
    focus-suppress) is lens-ui work over §3.4's `last_completed_turn`. Design spec REVIEW-CLOSED:
    `docs/specs/2026-07-14-lens-ui-shell-skeleton-design.md` — settled, don't re-litigate.

- **✅ Fixed 2026-07-21 — turn counter only bumped on `response.completed`.** Cancel/incomplete turns
  now bump `state.stream.turn` so the card flashes `Wave::Ready` ("just finished"). As-built (differs
  from the original handoff shape after two codex reviews): **Incomplete/Cancelled** bump the counter;
  **Failed** does NOT (it surfaces via `Wave::Failed`, and status is not folded atomically with the
  event — bumping would flash a transient green). All three **discard** open scratch (not finalize) —
  committing a synthetic local partial would permanently duplicate omnigent's durable `interrupted`
  `/items` row (messages reconcile by `item_id` only). `crates/lens-core/src/reduce/{mod,folds}.rs`;
  handoff `docs/handoffs/2026-07-21-turn-counter-non-completed-terminal-bug.md` (as-built appended);
  memory [[turn-counter-noncompleted-bug]]. **Live-verified 2026-07-21:** interrupted a streaming
  claude-sdk turn vs omnigent 0.5.1 — the partial is flushed via `response.output_item.done` (durable
  `/items` row under a **server** id) BEFORE `response.cancelled`, so the reducer commits it under that id
  and the Cancelled-arm discard is a no-op for the message → partial preserved, no loss, no duplicate
  (discard validated; the omnigent source's "Phase 2 TODO / not persisted" docstring is wrong). **One
  follow-up remains:** native `turn.completed/failed/cancelled` are deferred → `Unknown` → ignored, so the
  same bug persists on the native-runner surface. Merge-collision with T-0
  (`lens-transript`) in the same `reduce` match block stands — logically independent, textual merge only.

- **📋 SPEC-GAPS backlog** — independent, un-specced/partial items tracked in
  [`docs/SPEC-GAPS.md`](./SPEC-GAPS.md) (incl. #10 keyboard shortcuts + macOS app menu, Cmd+Q dead).

## Deferred, with a clean seam

- **lens-client modeling follow-on** — flip the 13 byte-verified SSE families `SCHEMA-DERIVED→MODELED`
  (capture done, memory `live-event-recapture-findings`); grow the two under-modeled payloads (`child{}`,
  elicitation `params`). Still-blocked families (`turn.*`, `response.created/queued`, codex reasoning)
  need a codex sub / OpenAI key.
- **lens-client small hardening** — `info.databricks_features: Value` leak; `ClientError::NotFound`
  rename + typed `Validation`/422; `/items` pagination; gated live-reconnect smoke.
- **WS terminal-attach client (Plan 7)** — no `terminal.rs`/`tungstenite` yet; workspace/terminal half
  of the contract is a known build-order deferral (converging with sibling `lens-terminal-ws`).
- **`session.superseded` reducer-drop** (`folds.rs:136` discards `target_conversation_id`) blocks
  terminal supersession-reattach — lens-core must surface it; terminal-integration-era.
- **Notifications v2** — server push for the fully-quit case (needs an upstream omnigent push channel).
- **Reducer normalization** — two status vocabularies (`SessionStatusValue` 6-val live vs
  `SessionStatus` 3-val snapshot) + two usage representations to normalize consumer-side.

## Open small decisions

- **Tunables (verification pass):** auto-sleep threshold (~10m), poll cadence (~10s), ring-buffer size
  (10 MB), transcript truncation tiers, `cost_samples` cadence.
- **Undecided UX:** terminal-`transfer` UX, managed-provider selection, policy/skill in-app authoring,
  multi-depth breadcrumb, exact-vs-range version pin.
- **Build artifact:** all status/harness/render glyphs are real Lucide SVGs (bell, triangle-alert,
  loader-circle, alarm-clock, check, moon, coffee, circle-dot, folder, git-branch). Only chrome
  furniture is still unicode — the kebab `⋮` and close `✕` (trivially swappable to `ellipsis-vertical`/
  `x` if/when a fully-bespoke set is wanted).

## Recently shipped (all on `main` unless noted)

- **Board B-3 — group chrome & rollups (2026-07-21):** filled the B-2 group placeholder with real chrome —
  `board/rollup.rs` pure `GroupRollup` fold (Σspend / oldest-`created_at` age / `completed_count`) +
  formatters; `group_accent` token→color resolver; `absolute_group` ring+accent+7%-tint + header-lane folded
  from member cards; `created_at` plumbed onto `SessionCard`; `test_layout` injection seam + `group_chrome_for_test`
  hook + fixture integration test (path runtime-dormant under basis B). `group_of` seam dropped (membership
  threaded as `GroupMeta` via `board_tree`); `✓N`=0 until B-6 Archive source. Subagent-driven (composer-2.5,
  codex gpt-5.6 board-logic review clean+1-Minor-fixed, Opus whole-branch SHIP). `xtask gate` green. **`3045590..75b78bb`,
  merged to main (UNPUSHED)**. Plan `docs/plans/2026-07-21-board-b3-group-chrome-rollups.md`.
- **Board B-2 — packing/scroll/culling (2026-07-21):** `lens-core::pack` pure packer + `board_tree`
  walk + `lens-ui` absolute-masonry `overflow_scroll` container (board N-col + rail 1-col via one
  `pack_and_render`) with band-culling + container-driven visibility gate that retired the paint-time
  `last_bounds` gate/`recover_viewport_gates_on_reentry`/`last_mode` (freeze fixed at root). Basis B:
  ephemeral `build_ephemeral_layout` stub feeds the tree (real store→replica = B-4). Subagent-driven
  (6 tasks, composer-2.5 implementers, codex gpt-5.6 per-task review, Opus whole-branch **READY**);
  `xtask gate` green; release demo launches clean. **`db5b7c2..14b474c`, merged to main (UNPUSHED)**.
  Memory [[board-b2-executed]]; handoff `docs/handoffs/2026-07-21-board-b2-executed.md`.
- **Board B-1 — data model & persistence (2026-07-18):** `lens-core` `BoardLayout` recursive
  Board→(Card|Group) tree + `SqliteBoardStore` (control-tier `lens.db`, schema **v2→v3** additive, lazy
  placement no backfill), ordinal-slot placement, mutation ops (place/move/ungroup/group/archive/…),
  bidirectional startup reconcile (lazy-place live, prune tombstoned). Adversarial review (grok-4.5 +
  probe tests; grok's "HIGH id-collision" refuted empirically) → 5 hardening fixes (high-water-mark id
  seed, tombstone place-guard, cycle seen-guard, deterministic reconcile order, +7 tests). 30 board
  tests, full `xtask gate` green. Committed **`8100cc8` (UNPUSHED)**. Spec
  `2026-07-18-board-data-model-persistence-design.md`; memory [[board-b1-executed]]; handoff
  `docs/handoffs/2026-07-18-board-b1-executed.md`.
- **Wave build B1–B5 + follow-ups (2026-07-17):** Lucide glyph tiles, context pbar, Slept/Wake/Retry
  seams, `loader-circle` spinner, canvas `paint_path` sweep, Scheduled countdown, viewport-gated
  20fps/1Hz anim driver, `demo` feature-gate; on-device visual pass; per-wave card-body wash; header
  3-tier type + host pill + per-wave activity line; **perf 30→20fps** (~35% CPU, `wave-perf-fps-attribution`).
  Spec `2026-07-17-wave-behaviors-design.md` §11. Handoff `2026-07-17-wave-build-visual-pass-merged.md`.
  - **Viewport re-entry freeze — RESOLVED (2026-07-17):** focus→board no longer freezes the off-screen
    card's spinner/pulse. Reset lives in `BoardView`'s fleet-observe effect; 3 regression tests; codex
    review addressed. Memory [[viewport-reentry-freeze]]. **Unpushed on `main`** (see below).
- **§18 Theming substrate (2026-07-16):** `crates/lens-ui/src/theme/` — `LensTheme` global (base+status
  tokens, hex↔Hsla serde, dark+light JSON), `cx.lens_theme()`, gpui-component bridge, external-file load
  + `cmd-shift-t` reload, `shortcuts.rs`. **On `main`, load-bearing for the cards.** Palettes tuned
   during the 2026-07-17 §11 on-device visual pass (bg ramp, wave status colors, context-bar thresholds,
   per-wave wash intensities) — no longer placeholders; residual fine-tuning is cheap via the reload
   loop. Memory [[lens-ui-theming-fork]].
- **`lens-ui` shell skeleton Plan 2 + card/board audit (2026-07-15/16):** §4–§7 skeleton merged; wave
  colors un-swapped, Needs-input=orange, icon-tile readout. Gate now covers lens-ui/lens-app.
- **lens-core §3 ActorFeed gate (2026-07-15):** unified `ActorFeed` FIFO, scheduler dual-mode,
  seed-on-spawn + emit-on-Demote, enriched `SummaryUpdate`. Grok-authored plan, subagent-driven.
  Memory [[grok45-as-plan-author]].
- **state-model engine P0–P3 (2026-07-08 → 07-12):** domain types → pure reducer → two-tier SQLite
  persistence → actor + store + commands + P3-3a/b lifecycle. All merged. Memories `state-model-*`.
- **lens-client (2026-06-25 → 07-10):** REST surface (Plans 2a–2e), SSE event modeling (Plan 3 series),
  benchmarks, pre-consumer hardening (Plan 4), omnigent pin `0.3.0.dev0 → 0.5.1`. Memories `plan3*`, `plan4*`.

## Housekeeping

- **`main` is AHEAD of `origin` by 5 (unpushed, as of 2026-07-18):** `759eb3a` (status fix) ·
  `c855ab6` (SPEC-GAPS §4 board → B-1..B-6) · `c21e669` (docs relocate → specs/plans) · `8100cc8`
  (B-1 board data model) · this docs-status commit. `origin/main` is at `b8727ab`. Push decision
  deferred to the user.
