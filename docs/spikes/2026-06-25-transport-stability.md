# Spike findings — omnigent transport-stability (§0.8 gate)

**Date:** 2026-06-25 · **Pin:** omnigent `0.3.0.dev0` (`36b2a11c`, source build).
**Question (capability §0.8):** is `omnigent server` solid enough to build Lens on
as a pure HTTP+SSE+WS client — or does the Rust-sidecar contingency reopen?

**Verdict: trending PASS. No instability observed; positive evidence on every
axis exercised. The Rust-sidecar contingency does not reopen.** Caveats below.

The harness itself was a throwaway Rust probe (`spikes/transport-stability/`,
since discarded). What follows is the durable result.

## Method
Opus-spec → `composer-2.5` build → `gpt-5.5` cross-family review → live runs
against the pinned source server. Probes: cold-start, RSS, SSE capture+taxonomy,
heartbeat, mid-stream-drop reconnect. (Server-crash reconnect / sustained-load
RSS were not separately driven — see caveats.)

## Findings

### 1. Server solidity — good
- Warm cold-start ~1.6s; ready ladder `/health`→`/api/version`→`/v1/info` all
  <5ms once up. (First boot right after a fresh install took ~31s — one-time.)
- Runs agents end-to-end via the canonical path (`omnigent run --harness
  claude-sdk` → real completion).
- Live SSE stream parses clean (0 errors) on a real heterogeneous stream.
- No crashes/hangs across many start/stop cycles.

### 2. Reconnect (the load-bearing gate piece) — green
Mid-stream drop on a live producing session, then the typed-client §7 protocol
(snapshot `?include_items` + `GET /items` + reopen stream + dedup): **zero
persisted-item loss.** Items that streamed *during* the disconnect were
recovered from `/items` on reconnect. Dedup by `sequence_number` applies to the
live-overlap only; `/items` rows (no `sequence_number`) merge by item `id`. The
probe was independently reviewed as sound — it cannot report a clean pass
without genuinely capturing a live item and executing the reconnect path.

### 3. Daemon/runner lifecycle — confirms server-lifecycle §3.1/§6
Three layers: **server** (state + API, no execution), **host daemon**
(registers host, launches runners), **runner** (executes the harness).
- `omnigent server stop` tears down the daemon with the server; `omnigent server
  start` brings the server back but **not** the daemon → `/v1/runners` empty.
- A bare-API `POST /v1/sessions {host_type:"external"}` does **not** auto-bind a
  runner. Working sessions carry an explicit `host_id` **and** `runner_id`, and
  the agent's harness must match an **online** runner's supported harnesses
  (e.g. a session pinned to `antigravity-native` fails against a runner serving
  `[claude-native, claude-sdk, codex, …]`). → Session→runner binding is
  explicit (server-lifecycle §6), not implicit on `host_type`.
- Empirically: a runner-less turn returns a typed `response.error
  {code:"runner_failed_to_start"}` → `session.status:"failed"`. Failures are
  legible on the wire (good for Lens's health surface).
- **Implication for Lens:** supervise the **daemon** (`-m
  omnigent.host._daemon_entry`), never bare `server start` — exactly as
  server-lifecycle §3.1 specifies. Now confirmed rather than assumed.

### 4. Wire/protocol confirmations (feed `lens-client`)
- SSE is live-tail/no-replay → **subscribe-first** is mandatory (POST the
  message only after the stream is attached, else early output is lost).
- SSE framing is `event: …\ndata: …\n\n`; stop-on-terminal must wait for a fully
  parsed terminal event, not a raw substring.
- The background server uses an **ephemeral port** (6767 when free, else random)
  — discover via `omnigent server status`, never hardcode.

## Caveats / not-yet-exercised
- **Server-crash reconnect (P7)** not separately driven. A server stop also
  kills the daemon/runner, so the session can't resume producing — but its
  persisted items survive (same snapshot+`/items` path proven in §2); recovery
  is read-only until the runner is relaunched (server-lifecycle §9.1).
- **RSS under sustained load** not stressed beyond a single session.
- The bring-up/binding order-sensitivity is a **Lens-implementation** concern
  (get §3.1/§6 right), not server instability.

## Process learnings (the delegation pipeline)
- `composer-2.5`: grounds static wire shapes correctly (verified by review),
  incorporates explicit review feedback accurately; **weak unprompted on
  temporal/stateful logic** (no-replay ordering, port-changes-on-restart,
  snapshot/items overlap) — needs those spelled out. Predictable, manageable.
- Cross-family review (`gpt-5.5`) caught exactly the falsely-green/false-fail
  risks a compile + the author could not. The diversity rule paid for itself.
- Opus-spec → composer-build → cross-family-review → live-run is a working loop.

## Disposition
- Throwaway harness **discarded** (its job was producing this evidence).
- The reconnect protocol + golden-SSE contract tests get built **properly** in
  `lens-client` (typed-client §7/§9) — clean captures to be taken there.
