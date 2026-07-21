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

## T-0 live rider (2026-07-21, executed slice)

The built T-0 stack was live-verified two ways:

1. **Integration rider** — `crates/lens-core/tests/t0_live_rider.rs` replays `turn2.stream.sse` and
   `interrupt-then-retry.stream.sse` through the real built pipeline (`lens_client::stream::decode_all`
   → `lens_core::reduce`) and asserts: items carry their **own** wire `response_id`; `active_response`
   mirrors `response.in_progress` mid-turn and clears on terminal `response.*`; interrupt→retry mints a
   **distinct** turn-B id; turn A's interrupted id never lands on a transcript item; and the delta path
   emits `ActiveResponseChanged` `Some(A) → None → Some(B)`. Both tests green.
2. **Fresh drift-drive** — `rider-refresh_items.no-created_at.json`: a fresh claude-sdk turn driven
   against the same live 0.5.1 server (build `08285468`), then `GET /v1/sessions/{id}/items`. Confirms
   the shapes T-0 was built against still hold today: `response_id` present on every item (resource=`conv_`,
   user=`turn_`, agent=`resp_`), `created_at` **null** on `/items` (§7).
