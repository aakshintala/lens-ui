# T-0 turn-identity live verification — omnigent 0.5.1 (08285468)

Driven 2026-07-21, claude-sdk harness, two real turns. See memory
[[t0-response-id-live-sourcing]] for the full analysis. Captures here are the raw evidence:

- `items_endpoint.no-created_at.json` — `GET /items`: `response_id` present, `created_at` **null**.
- `snapshot_include-items.has-created_at.json` — `GET /session?include_items`: `created_at`
  present (epoch **seconds**); `active_response_id` null (idle).
- `turn2.stream.sse` — live SSE: `response.in_progress.response.id` = the live turn id;
  `output_item.done.item.response_id` present on message; `session.status.response_id` = **null**.
- `liveness_poll.active_response_id-null-midturn.txt` — snapshot `active_response_id` polled
  during a running turn: **null** (in-process harnesses don't populate it).

Turn ids observed: turn-1 agent `resp_00b52ad7…`, turn-2 `resp_bcb93365…` (distinct per turn).

- `interrupt-then-retry.stream.sse` — interrupt turn A (`resp_0099878e`) → `response.cancelled`
  (carries that id) → retry turn B starts a NEW `response.in_progress` `resp_37ba30e3`
  (`previous_response_id: null`). Confirms: cancelled-turn retry mints a distinct response_id;
  terminal events carry the ending response's id.
