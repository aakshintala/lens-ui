# Lens — STATUS archive

Full dated session entries, append-only (newest at the bottom). The live
forward-looking state is in `STATUS.md`; detailed logs roll here as they age.

---

### 2026-06-24 — grilling pass, doc walkthrough, first renders

**Grounding.** Pulled omnigent 0.2.0 source into `../omnigent/`; ground-truthed
every contract claim. Render workflow set to **local-only** (`docs/design/renders/`).

**Factual corrections (verified against source).**
- Harnesses **16** (added `copilot`, `opencode-native`, `antigravity-native`, `qwen-native`).
- Cost is **server-computed** (`total_cost_usd`) — no Lens price table.
- Session status has **5** values — added `Launching`.
- **Fork** is `POST /fork`, not a `SessionEventInput` arm.
- **switch-agent** = `POST /switch-agent` (not `PUT /agent`); guards: owner-only +
  idle-only (409) + no sub-agents + no no-op; runner resources reset.
- `child_session.updated` carries **partial deltas** (merge, not replace).
- `X-Forwarded-Email` is **trusted-proxy** auth (OIDC cookie is the real remote credential).

**Cross-cutting decisions — all resolved (ledger was drifting).**
- **A** task=session; **Group** is the grouping (no `Task` entity); single-root
  file tree default; "task" retired as a term.
- **B** sub-agent home = tray "Sub-agents" segment; children never board cards.
- **C/F/G** ratified (ring buffer; collapsible working area + ⌘D; multi-window).
- **I** two-axis cost: cumulative per-card/project (server USD) + time-windowed
  global (today/7d/30d) via new `cost_samples` table.
- **J** switch-agent in-place handoff + verified guards.

**Session lifecycle — reshaped (was "Sleeping = client detach").**
- **Sleep** = `stop_session` (reclaim harness/PTY) + dim, stays visible;
  auto after ~10-min quiet (terminal-aware, skips pinned/needs-input).
- **Archive** = `stop_session` + hide (no longer UI-only).
- **Delete** = server delete. **No stream cap** (self-bounds via auto-sleep).
- Wake = resume + re-bind runner.

**New decisions (walkthrough).**
- **Status→wave** mapping pinned; **Ready** = idle + unviewed completion;
  **Scheduled** reserved but cut from v1 (it's the `/loop` state).
- **Concierge** is local-server-only → **local server is always-on baseline
  infra**; renders as a **floating pinnable chat panel** (⌘⇧C).
- **Keybindings:** `^\`` toggle terminal, **⌘⇧C** Concierge, **⌘⇧1-9** board-switch.
- **Bridge Inbox UI** (closes decision H): **pinned "Needs you" band + reverse-chron
  stream**; card = `kind · from→to · status · body · actions`.
- **Card design:** icon tile · status · title · `harness·model` · activity line ·
  `📁 repo ⑂ branch` rows · host+cost+ctx bar; **tinted group bodies**.
- **Residency + notifications:** resident menu-bar app; native needs-input
  notifications + `lens://` deep-links; ⌘W hides, ⌘Q quits; background poll
  throttles (not pauses).

**Renders (local):** `board-home.html`, `focused-session.html`, `bridge-inbox.html`.

**Cleanup:** scrubbed internal lineage (Cairn/MessageCenter/"older spec"/predecessor/
infinite canvas), kept the real reference apps (Arbor/Paneflow/gpui-flow) + Polly;
fixed editing-artifact typos.

**Docs touched:** all 11 in `docs/design/` + README.

---

### 2026-06-26 — Plan 3b-2b: §7 no-replay reconnect state machine (executed & complete)

**What.** Made the SSE reader thread reconnect-safe end-to-end inside the crate.
On a transport drop / clean EOF it backs off, re-reads the session snapshot +
`/items`, re-opens the live stream, and emits synthetic lifecycle markers on the
existing mpsc channel so the consumer stays purely event-driven and never sees
raw reconnect mechanics.

**Execution.** Subagent-driven, same session: `cursor-delegate`/composer-2.5 build
per task (red→green→commit), Opus controller per-task review, one consolidated
gpt-5.5 cross-family review at the end (`[[review-spend-policy]]`). Commits
`3d4048b..6d4dde3` — 6 code tasks + 1 review fix wave + an xtask fmt-housekeeping
commit (`b838a66`, pre-existing drift unblocking the workspace `cargo fmt --check`
gate) + docs. 119 lib tests, clippy `--all-targets`/fmt clean, `generated.rs`
untouched, no `Value` to consumers.

**Shipped.**
- T1: 4 synthetic `ServerStreamEvent` variants (`Reconnecting{attempt}`,
  `Reconnected{gap:Option<u64>}`, `SnapshotRestored(Box<SessionSnapshot>)`,
  `Disconnected{reason}`) + `DisconnectReason` (5 variants); `PartialEq` on
  `SessionSnapshot`/`ModelUsage`/`SkillRef`.
- T2: `Normalizer::reset_seen_items` (dedup-reset seam for history replay).
- T3: `SseFrame::sequence_number()` raw-JSON peek (`Option<u64>`).
- T4: `reconnect` module — `Reopen` trait + `HttpReopener` + `BACKOFF_MS`
  `[100,200,400,800,1600,3000,3000]` + `items_to_replay`; `ItemList::into_items`;
  `GetOpts`/`ItemsPage` `to_query` → `pub(crate)`.
- T5: the state machine in `stream::reader` — generic `run<Re:Reopen>` + injected
  `sleep`; `stop_reason` (401→Unauthorized, 403→Forbidden, 404→NotFound); clean-EOF
  flush vs transport-drop no-flush; overlap seq-dedup window; 4 order-asserting
  reconnect tests + 2 updated §7a tests.
- T6: `Sessions::stream` builds the real `HttpReopener` (StubReopener bridge deleted).
- T7: §7 reconciled (this) — `DisconnectReason` table, `gap:None` v1, frame-seq
  peek, single-page items, `items→open_stream` ordering, fallible spawn.

**Cross-family review (gpt-5.5) — 1 Critical, 3 Important, 1 Minor, all valid:**
- **CRITICAL:** `reconnect` opened the new body *before* fetching `/items`, then
  `continue`d the backoff on a retryable `/items` error — dropping the already-opened
  no-replay body (lost live frames). The author + green tests both missed it.
  Fixed: `snapshot → items → open_stream` (open_stream last fallible); markers
  emitted only after all three succeed; + a regression test
  (`retryable_items_failure_does_not_drop_the_reopened_body`).
- **IMPORTANT (user-decided):** failed-status path emitted `Reconnected` then
  `Disconnected{SessionFailed}` (contradictory; plan pseudocode had mandated it) →
  now emits `SnapshotRestored → Disconnected{SessionFailed}` only. And the
  pre-existing (Plan 3a) `spawn` `.expect` panic → `EventStream::spawn` now returns
  `Result` via new `ClientError::ThreadSpawn`.
- **MINOR:** removed unused `_last_seen_seq` param.
- Re-review done by Opus controller (fix == reviewer's prescribed remedy + regression
  test); 2nd paid cross-family pass forgone per budget policy.

**Deferred (flagged, safe fallbacks):** `gap==Some(0)` contiguity proof (v1 always
`None`); `/items` pagination/backfill (reducer merges by id); gated live reconnect
smoke test (no scripted server-kill harness this session). ⚠ `live_stream` NOT run
(no server) — unit coverage only.

**Minor rollup (final-triage, deferred):** reconnect.rs test-module redundant
re-imports (clippy-clean); `MockReopen` redundant `open_stream_always_503` branch;
`happy_idle_snapshot()` duplicated across the two reader test modules. None ship-blocking.

**Next:** Plan 3c — contract-drift CI (outstanding B6).
