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

## Outstanding rider legs — RESOLVED 2026-07-23 by `terminal_live` P9/P10 (`003e242`)

The three legs listed here originally were framed as "needs the GUI app + a human watching."
That framing was wrong: **there is no production terminal surface until Slice 6**, so the GUI
app cannot host them. They were instead built as two new automated phases in
`crates/lens-terminal/tests/terminal_live.rs`, which already opens a real GPUI window hosting a
production `TerminalTab`. Both **PASS twice consecutively** against the same live omnigent 0.5.1.

### P9 — cross-session Transfer (`LENS_LIVE_TRANSFER=1`) ✅

Drives the *same* four-call rotation this rider proved (minus the `external_session_superseded`
POST, which is lens-ui's input and has no consumer in lens-terminal), then asserts:

- the attach **survives the transfer untouched** — see finding 6 below;
- `resource.deleted` + `Transfer{new_session: B}` return the tab to `Live` **against B**, with
  `presentation.output_gap` set and the pre-rotation marker still on screen.

This closes legs 1 and 3 together, and *mechanically* rather than by eye: marker survival is
scrollback survival, and `output_gap` is asserted rather than judged. It is also the only proof
that cross-session engine reuse actually lands — the in-crate deterministic test
(`transfer_reuses_retained_engine_and_retargets_session`) can only show the reuse branch is
*entered*, because its offline attach against B then fails by construction.

### P10 — `4404` before the host signals (`LENS_LIVE_4404_FIRST=1`) ✅

Leg 2, forced. Deletes the terminal resource outright and forwards **nothing**, asserting the tab
reaches `ReplacementWaiting` **holding its frozen engine** on the close code alone — nothing else
could have moved it. Then relaunches the same key and forwards the late `resource.deleted` +
`resource.created`, which must still drive `adopt()` to `Live`. (Same-session adopt is a fresh
attach by design, so scrollback is not expected to survive that branch.)

### 6. NEW — a `…/transfer` is attach-transparent (no `4404`) ✅

P10's first draft assumed the transfer itself would provoke the `4404` and **failed**: after the
rotation the tab stayed `Live` with a healthy bridge. A transfer rebinds the resource's owning
session and closes nothing — the terminal WS is bound to the terminal/tmux socket, not the
session. Two consequences:

- The host's forwarded `resource.deleted` is the **sole** trigger for the supersede follow. There
  is no transport-level backstop; if the host drops that event the tab silently stays attached to
  a terminal that now belongs to another session. P9 now asserts this transparency directly, so a
  server-side change here fails the rider instead of silently altering the contract.
- Deleting the resource is the only cheap live source of a `4404`, which is what P10 uses.

### 7. NEW — delete+create returns the SAME deterministic id ✅

P10's second draft asserted the successor would carry a new `TerminalId` and **failed**: the
recreate returned `terminal_shell_main` again, even though the delete demonstrably took (the
`4404` had already parked the tab). Ids are `terminal_{name}_{key}` by construction — confirming
memory `terminal-resource-event-granularity` from the *other* direction, and explaining why the
generation guard keys on the delete/create **events** rather than on an id change. An
id-comparison guard would have been a no-op.

## Cleanup

Rider runner torn down (`pkill -f "omnigent run ./rider-shell"`) after both the wire rider and the
P9/P10 runs. The user's pre-existing server (pid 46270, 15 live sessions, host daemon attached)
was **left running and never restarted** — `GET /v1/info` already confirmed it serves the pinned
0.5.1. The rider sessions each run created (`conv_4ae5a516b18a…`, `conv_7422c24b1ef7…`, and the
`conv_a1ac…`/`conv_e90d…`/`conv_4711…`/`conv_6145…`/`conv_274b…` chain P9 minted) remain as inert
records; each P9 run creates one by design, so run these against a scratch rider agent.

## Full-slice status after this

Every live rider leg sub-slice A and D wanted is now automated and green. Remaining before merge:
nothing rider-shaped. The deferred **foreground blocking handshake** fix (Task 6 Important 1) is
still outstanding and is now better motivated — see finding 3: the load-B window is bounded by the
30s `REPLACEMENT_WAIT`, so the handshake cost is a budget, not just a hitch.
