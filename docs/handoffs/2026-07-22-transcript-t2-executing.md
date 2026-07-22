# Handoff — Transcript T-2 EXECUTING (12/15 tasks) — 2026-07-22

**Branch:** `lens-transript` (UNMERGED), tip `84043e5`. Session base was `0878737`.
**Plan:** `docs/plans/2026-07-22-transcript-t2-focused-view-scaffold.md` (15 tasks).
**Resume ledger (authoritative, per-task detail + all findings):** `.superpowers/sdd/progress.md`.
**Execution model:** subagent-driven-development — cursor-delegate `composer-2.5` implementers, `codex` gpt-5.6 + `grok-4.5` + Opus subagent reviews. Every task TDD + `cargo run -p xtask -- gate` green.

## Done (gate-green)
- **Phase A (Tasks 1–6):** T-1 amendment (per-run WorkSection), `Reconnected{gap}`, `AccId`, `Retired{acc_id,disposition}`, `TranscriptRewritten{ordinal}`, read-only `TranscriptReader` + transactional ranged read. lens-core complete.
- **Phase B 7–12:** reader factory + reconcile epoch; poller fan-out via `WeakEntity<FleetStore>`; `FocusedTranscript` replica; serialized reader worker; production `RowStore`; **two-level retained entities + staged finalize (the crux)**.
- **Crux real-window proof PASSES** — `cargo run -p lens-ui --bin focused_finalize_probe` exits 0 (staged finalize flash-free on real pixels). MUST run with the sandbox disabled — see the memory `t2-real-window-probe-sandbox`.

## Open — RESUME HERE
1. **Task 13 scroll contracts (real-window debug, orchestrator-only):** view implemented + mounted in `#chat-slot` (`62a5c76`); a **Critical re-entrancy panic** was caught by the scroll probe and FIXED (`84043e5` — observe callback in `new()` did a nested `weak.update` while leased → `cx.notify()` direct; scroll handler direct, not deferred). Panic gone (probe runs 31 samples). BUT the 4 scroll-contract assertions FAIL ("scroll-up did not pause", "pill not visible", "N!=3", "jump did not resume"). Likely the probe's scroll *simulation* (`scroll_by`/`scroll_to` may not emit a `ListScrollEvent` with `is_scrolled=true` to fire the handler) OR view follow-mode wiring (`view.rs` `set_follow_mode`/`new_since_pause` look correct). Also the probe's exit logic is BUGGY (printed failures yet exited 0 — must exit 1 on non-empty `failures`). Iterate: edit → `cargo build -p lens-ui --bin focused_scroll_probe` → run with `dangerouslyDisableSandbox`.
2. **Task 14** — `ReconnectBreak` marker (`RowId::Marker` reserved for it; `StructureEntry::Marker` scaffolded). No scroll-contract dependency.
3. **Task 15** — `syncing…` debounce + release perf gate (O(visible)).
4. **Carried findings** (in the ledger's roll-up): confirmed **flaky Task-7 test** `spawned_session_retains_reader_factory` (mock-omnigent handshake — deterministic rewrite owed before CI-trustworthy); **Task-12 collapse nuance** (pending reasoning tail hidden when `Completed`'s `ActiveResponseChanged(None)` collapses its section — validate once Task 13 view works); **rusqlite-in-lens-ui layering** Minor (add `PersistError::is_busy()` to lens-core).

## Notes
- Real-window tests are `#[ignore]`d in the gate (CI has no WindowServer — correct); validate the probe binaries manually with the sandbox disabled.
- `lens-store` is an ungated production crate with an exhaustive `StreamUpdate` match — any new variant must add an arm (and `cargo build --workspace` should be run, not just the gate). See ledger pre-flight + gate-scope finding.
