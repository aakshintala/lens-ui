# Handoff — Terminal Slice 1d: live-rider RUN (the only thing left)

**Slice 1d code is DONE, fully reviewed, and gate-green** on `terminal-ws` (unpushed).
The ONE remaining task is *running* the live rider against a real omnigent 0.5.1
terminal — which is blocked on **omnigent operational setup + one design question**,
NOT on any Lens code. This session built + reviewed all of 1d and attempted the live
run; it hit an omnigent wall and stopped for a fresh session to investigate.

## State: 1d is code-complete

- **17 commits** `f4d3080..39ee7b3` on `terminal-ws` (unpushed; user chose "stay on branch, don't merge").
- **Gate GREEN across all feature combos:** `cargo fmt --check`, `cargo clippy --workspace
  --all-targets -D warnings`, `cargo clippy -p lens-terminal --all-targets --features
  test-util,live-tests -D warnings`, `cargo build -p lens-client` (± `--features live-tests`),
  `cargo test -p lens-terminal` (46), `cargo test -p lens-client` (163),
  `cargo test -p lens-terminal --features live-tests,test-util --no-run`.
- **Ledger:** `.superpowers/sdd/progress.md` (full per-task record + roll-up). Plan:
  `docs/plans/2026-07-16-terminal-slice-1d-convergence.md`. Spec:
  `docs/specs/2026-07-16-terminal-workstream-design.md`.
- **What shipped:** bridge thread (Select multiplex; OutboundSaturated/AttachDisconnected/
  EngineStopped events) · `TerminalRuntime` off-foreground teardown (panic-free unique-Arc stop) ·
  `open()` C2 background discover/attach + foreground `cx.spawn` wake sampler (durable `policy_tx`
  survives reconnect) · close-code policy + single 30s `RetryWindow` + reconnect state machine
  (read-only downgrade → read-only reconnect; preflight GET; transient-retryable) · resize-before-
  input on connect+reconnect · retained-engine reconnect-seed acceptance · `TerminalInspect` ·
  standalone GPUI demo · live-rider harness.
- **Reviews earned their keep:** codex on T4 (2 Critical: reconnect race via background-spawned
  bridge → fixed by atomic foreground spawn; foreground-block → try_send; +4 Important). codex
  whole-branch (6 Important integration issues: engine-death stuck-Live; initial resize skipped;
  **rider false-green**; abort machinery leaking to production; inspect not durable across reconnect)
  — ALL folded in `0e111e8`. ~4 codex calls total; rest Opus-inline (Claude≠Composer satisfies the
  cross-family rule at zero codex cost).

## The remaining task: RUN `tests/terminal_live.rs` vs omnigent 0.5.1

The rider CODE is written + hardened (false-green fixed: unique `LENSMARK_<pid>` marker +
paint-after-marker correlation). It's a `harness=false` real-window binary, 4 phases:
P1 attach→Live · P2 send `echo LENSMARKER\r` + prove a PAINTED frame contains it · P3
`abort_for_test`→Reconnecting · P4 reattach Live + `output_gap`. Env-gated skip (exit 0) vs
fail (exit 1). Live-tests debug hooks on `TerminalTab`: `debug_send_input_for_test` (returns
bool), `debug_abort_attach_for_test`, `debug_latest_frame_for_test` (all `#[cfg(feature="live-tests")]`).

Run cmd:
```
LENS_OMNIGENT_URL=http://127.0.0.1:6767 LENS_OMNIGENT_SESSION_ID=<...> \
LENS_OMNIGENT_TERMINAL_NAME=lens-rider LENS_OMNIGENT_SESSION_KEY=rider \
cargo test -p lens-terminal --features live-tests,test-util --test terminal_live -- --nocapture
```

### What the live-run attempt found (the blockers — omnigent-side, NOT rider bugs)

1. **Rider reached P1 and reported correctly.** Failure = `terminal_live FAIL: P1 attach: lifecycle
   did not reach Live`. Root cause (server): `runner_unavailable — runner 'runner_token_…' is
   offline for conversation 'conv_…'`.
2. **Orphaned sessions.** All ~20 existing omnigent sessions are bound to their ORIGINAL runners
   (long gone). Bringing a fresh host online does NOT adopt them. A live terminal needs a session
   **dispatched to an online runner** — i.e. launched via `omnigent run --harness <X>` on the host.
3. **⚠ DESIGN QUESTION (the thing to investigate): agent-terminal vs shell-terminal.** The rider
   sends `echo LENSMARKER\r` assuming a **shell PTY that echoes**. But omnigent SESSION terminals run
   the **agent TUI** — sending that to e.g. claude-native types into its prompt box, and the `\r`
   risks submitting a **billable LLM turn**, not a shell echo. **Open question:** does
   `POST /v1/sessions/{id}/resources/terminals` (`TerminalCreate{terminal, session_key}`, lens-client
   `rest.rs:32` "launch or return existing") launch a **bare shell terminal** (separate from the
   agent) on the session's runner? If YES → the echo-marker approach is valid once a runner is online.
   If NO → the rider's P2 needs adapting (a non-submitting marker that only DISPLAYS in the agent's
   input, or observe existing terminal output) to work against agent-backed terminals.

### Recipe to complete the live run (next session)

1. Ensure server + an ONLINE host: `omnigent server start`; `omnigent host --server
   http://127.0.0.1:6767 --non-interactive &` (this session left both running — see below; verify
   `curl -s :6767/v1/hosts` shows a host `online`).
2. **Investigate blocker #3 first** (cheapest, zero-cost): read the omnigent source at
   `~/.local/share/uv/tools/omnigent/.../server/routes/terminal_attach.py` + the terminal-resource
   CRUD (`POST …/resources/terminals`) to determine whether it spawns a bare shell PTY vs the agent
   TUI. The Spike B doc `docs/spikes/2026-07-15-pty-attach-contract.md` says terminal ids like
   `terminal_bash_s1` are "minted by the terminal-resource CRUD" and are tmux-backed — suggests a
   bare shell terminal MAY be creatable. Confirm.
3. Then get a session on the online runner (launch an agent session — mind LLM cost; a no-`-p`
   `omnigent run --harness claude-sdk` starts the session+terminal WITHOUT an immediate LLM turn) OR,
   if #2 confirms bare-shell terminals, create one via REST directly.
4. Run the rider (cmd above). If P2 fails on the marker, apply the adaptation from #3.

## Infra THIS session left running (on the user's machine)

- omnigent **server** pid 46270 at `http://127.0.0.1:6767` (log `~/.omnigent/logs/server/local-server-*.log`).
- omnigent **host daemon** pid 47321 (`omnigent host --server http://127.0.0.1:6767 --non-interactive`,
  log `~/.omnigent/logs/lens-rider-host.log`), host_id `host_e0b4c26c6cc54febb978760194fe9795`, online.
- Stop both with **`omnigent server stop`** (stops server + local host daemon). The user was asked
  leave-vs-stop but interrupted to write this handoff — decide + clean up as desired.

## Deferred / roll-up (from the ledger — none block 1d)

- **Pre-existing 1b engine flake** `engine::handle::tests::hidden_tab_suppresses_publish_until_visible`
  — failed 1/4 under full parallel `cargo test --workspace` load, passes on re-run. A SEPARATE flake
  from the two the per-handle build-failure-injection fix covered. Undermines xtask-gate determinism.
  Harden its timing (1b scope).
- **`EngineHandle::spawn` readiness result (1b):** engine init failure currently flashes `Live` briefly
  before the bridge's new `EngineStopped` event drives Detached. A readiness result at spawn would avoid
  the flash. (1d handles the death-after-attach case.)
- **Process:** per-task `lens-terminal` clippy should include `--features test-util` (and `live-tests`)
  — its absence hid T3's `set_frame` `pub(crate)` breakage of `render_realwindow.rs` for ~6 tasks until
  T9's gate (which included the features) caught it. Only the xtask gate built the gated targets.
- **Thread-exhaustion foreground-panic** (extreme OS edge; whole codebase unguarded) — accepted.

## Do NOT redo

All 1d code + reviews are complete and committed. Do not re-review or rebuild the tasks. The next
session's job is ONLY: investigate the agent-terminal question, stand up a working omnigent terminal,
run the rider to a PASS, then (user's call) merge `terminal-ws` → main.
