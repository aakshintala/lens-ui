# D30 tool-fold golden fixtures

Captured live from omnigent 0.5.1 (source HEAD 08285468) on 2026-07-12, session
`conv_0c5f615af67c4644bf872b5324e7d8bd`, a `claude-sdk` shell-tool turn
(`echo LENS_D30_MARKER`). Drives the D30 scaffold-id two-id-space fold.

- `tool_fold.stream.sse` — raw `GET /v1/sessions/{id}/stream` bytes. The live
  tool call splits across TWO `fc_*` ids sharing one `call_id`:
  `fc_1a365818d5b6` (in_progress) + `fc_7ad94742b335` (completed),
  `call_id=toolu_013u7owVfBjDQpv6ourvP6DY`; output `fco_a51bcc02b848`.
- `tool_fold.items.json` — `GET …/items` canonical rows. The SAME call_id
  appears under a DIFFERENT store id: `fc_9bb8ae52357c40a…` (+ `fco_7bd2ed51…`).

The D30 fold reconciles the live provisional `fc_*` rows into the `/items`
canonical rows by `call_id` → one `function_call` row (canonical id,
`provisional=0`), live `fc_*` ids gone. See the golden-replay test in
`src/actor/runloop.rs`.
