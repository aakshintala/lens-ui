# Live rider results — Terminal Slice 5 sub-slice D

**Date:** 2026-07-23
**Server:** live omnigent **0.5.1** at `127.0.0.1:6767`, source checkout on tag `v0.5.1` @ `08285468`
(matches `vendor/omnigent-0.5.1/README.md` Source HEAD; installed binary embeds the same commit;
`GET /v1/info` → `server_version: 0.5.1`). Pinned contract confirmed — not a PyPI release build.
**Branch:** `terminal-slice-5-fleetstore` @ `9eadc45`
**Raw SSE capture:** `.superpowers/sdd/rider-sse-A.log`

## Status: rider leg 1 (wire/contract) **PASS**. GUI legs outstanding.

---

## What was run

A **wire-level rider** reproducing the production `/clear` rotation exactly. This is not a
simulation: the claude-native forwarder's `_post_clear_supersession` /
`_create_clear_replacement_session` (`omnigent/claude_native_forwarder.py:2065-2125`, `:3344-3440`)
perform precisely these four REST calls, and the rider issues the same ones from a different
client:

1. `POST /v1/sessions` `{agent_id}` → session **B**
2. `PATCH /v1/sessions/{B}` `{runner_id}` → bind B to A's runner
3. `POST /v1/sessions/{A}/resources/terminals/terminal_shell_main/transfer` `{target_session_id: B}`
4. `POST /v1/sessions/{A}/events` `{"type":"external_session_superseded","data":{"target_conversation_id": B}}`

Step 4 lands in the server's `post_event` handler (`server/routes/sessions.py:19226-19235`), which
calls the **same** `_publish_session_superseded` the real `/clear` path calls — so the emitted
event is byte-identical to production.

Session A was an ephemeral `rider-shell` bundle (bash terminal, `harness: claude-sdk`, no `-p`)
per memory `omnigent-terminal-attach-live-run` — zero LLM cost.

**Fidelity gap, stated honestly:** what is *not* exercised is Claude Code's own `/clear` hook
detection and the forwarder's decision to rotate. Those are claude-native-internal and have no
Lens surface. Everything from the server emission outward is the real thing.

---

## Findings

### 1. `session.superseded` IS emitted — D's no-fallback bet holds ✅

This is the one D has no fallback for (design §4.2 `map_item` deliberately not built). Captured
verbatim on A's live stream:

```
event: session.superseded
data: {"sequence_number": null, "type": "session.superseded",
       "conversation_id": "conv_4ae5a516b18a4f669744c626ef07f29b",
       "target_conversation_id": "conv_7422c24b1ef7423e8360919baf219dde",
       "reason": "clear"}
```

`target_conversation_id` is B, `reason` is `"clear"`. **§4.2 does not need reopening.**

Caveat inherited from the server, not introduced by us: this event is documented as
**transient / live-only, with no SSE replay** (`server/schemas.py:2960-2965`). A client that
connects *after* the rotation never sees it; the durable counterpart is the persisted notice
`message` item — which is exactly the `map_item` path D chose not to build. So D follows a
supersede **only while actively streaming A**. That is the accepted design position, now
confirmed against live behavior rather than assumed.

### 2. The terminal transfers live, same id, same PTY ✅

`POST …/transfer` returned the resource re-parented to B:

- `id: terminal_shell_main` — **same TerminalId**
- `session_id` → B
- `running: true`
- `tmux_socket: …omnigent-terminal-5314p19u/tmux.sock` — **the same socket as before the move**

Post-rotation ownership confirmed by GET on both sessions: B owns `terminal_shell_main`
(same socket, running); A no longer does. The server-side docstring — *"Move a terminal resource
to another session without closing it… the tmux pane keeps running"*
(`server/routes/sessions.py:17635-17640`) — is accurate.

This is the precondition D's retain-engine `Transfer` rests on, now proven live.

### 3. ⚠️ NEW — the live order is `resource.deleted` **then** `session.superseded`

Full event order captured on A across the rotation:

```
session.heartbeat
session.resource.created   (terminal_tui_main)
session.resource.created   (terminal_shell_main)
session.changed_files.invalidated
session.presence
session.heartbeat ×3
session.resource.deleted   (terminal_shell_main)   <-- FIRST
session.superseded         (target = B)            <-- SECOND
session.heartbeat
```

**The design did not document this interleaving.** It matters, and the news is good — traced
through the code, it lands D on its designed happy path rather than fighting it:

1. `resource.deleted{terminal_shell_main}` → poller → `ActorOutcome::TerminalResource(Deleted)`
   → `FleetStore::on_session_control` → `forward_terminal_resource` → `tab.on_host_event(ResourceDeleted)`.
   In the tab, `on_resource_signal` (`lens-terminal/src/lib.rs:1932`) consults the generation
   guard, which for a delete of the bound tid **with a key present** returns `AwaitReplacement`
   (`generation.rs:66-72`) → `enter_replacement_waiting()` (`lib.rs:1961`) — **transport-only
   teardown that RETAINS the frozen engine**, and arms the 30s replacement timeout.
2. `session.superseded{target: B}` → `on_supersede` → load B → `move_terminal_members` →
   `Transfer{new_session: B}` → the tab's changed-session branch **reuses the retained engine**.

So **scrollback survives *because of* the delete-first ordering, not despite it**: the delete is
what parks the tab in the engine-retaining `ReplacementWaiting` state that `Transfer` then reuses.

**Residual risk this exposes:** the load-B window is bounded by the 30s `REPLACEMENT_WAIT`
timeout. If GET → seed → `spawn_live_session` ever exceeds 30s, the timeout fires first, the tab
detaches, and scrollback is lost. 30s is generous headroom today, but this is the concrete
consequence of the deferred foreground-handshake cost (Task 6 Important 1) — it is a budget, not
just a hitch.

### 4. Wire shape carries an undocumented `sequence_number` — parses fine ✅

Both `session.superseded` and the resource events carry `"sequence_number": null`, which the
schema docstring's stated flat shape omits. `RawSuperseded`
(`lens-client/src/stream/event.rs:369-375`) is a plain `Deserialize` without
`deny_unknown_fields`, so the extra field is ignored. No action.

### 5. `resource.deleted` payload confirms the known granularity limit ✅

```json
{"type":"session.resource.deleted","resource_id":"terminal_shell_main",
 "resource_type":"terminal","session_id":"conv_4ae5…"}
```

Carries only the id — no `terminal_name` / `session_key`. Confirms memory
`terminal-resource-event-granularity` and the deterministic-id reconstruction D relies on.

---

## Outstanding rider legs (need the GUI app + a human watching)

1. **Visual scrollback survival.** Wire-level proof is done (same PTY under B, engine-retaining
   path traced). What remains is confirming in `lens-app` that pre-`/clear` output is still
   *rendered and scrollable* after the follow — i.e. the retained engine's screen state actually
   paints.
2. **`4404`-first real interleaving.** This rider produced the `deleted`-first order. The other
   race (transport `4404` before `resource.deleted`) was not observed and still needs forcing.
3. **Transfer `output_gap` visual.** Sub-slice A's review flagged `on_reconnect_success` setting
   `output_gap = true` on the Transfer reuse path, possibly spurious when B replays clear+redraw.
   Needs eyes on the window. This is also the measurement that sizes the deferred handshake fix.

## Cleanup

Rider runner torn down (`pkill -f "omnigent run ./rider-shell"`). The user's pre-existing server
(pid 46270, 15 live sessions, host daemon attached) was **left running and never restarted** —
`GET /v1/info` already confirmed it serves the pinned 0.5.1. Rider sessions A
(`conv_4ae5a516b18a…`) and B (`conv_7422c24b1ef7…`) remain as inert records.
