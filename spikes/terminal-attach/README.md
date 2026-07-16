# terminal-attach (Spike B1)

Throwaway capture harness for omnigent's terminal WebSocket attach contract.
Dumps every frame to JSONL under `docs/spikes/captures/2026-07-15-pty-attach/`.

Contract reference: `docs/spikes/2026-07-15-pty-attach-contract.md`.

## Build

```bash
cargo build -p terminal-attach
```

## Environment

| Variable | Required | Description |
|----------|----------|-------------|
| `OMNIGENT_BASE_URL` | yes | REST base URL, e.g. `http://127.0.0.1:8000` |
| `OMNIGENT_TOKEN` | no | Bearer token for REST + WS (if your deployment needs it) |
| `OMNIGENT_SESSION_ID` | conditional | Existing session id. Required unless both session **and** terminal ids are set. |
| `OMNIGENT_TERMINAL_ID` | no | Existing terminal id. When **both** session and terminal are set, REST create is skipped. |
| `OMNIGENT_TERMINAL_NAME` | no | `terminal` field for POST create (default: `shell`) |
| `OMNIGENT_TERMINAL_SESSION_KEY` | no | `session_key` field for POST create (default: `main`) |

## CLI

```
terminal-attach [--transport <pty|control|default>] [--scenario <name>]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--transport` | `control` | WS `transport` query param. `default` omits the param (server default). |
| `--scenario` | `all` | `attach`, `input`, `resize`, `reconnect`, or `all` (sequential). |

## Scenarios

Each scenario writes `docs/spikes/captures/2026-07-15-pty-attach/{scenario}.frames.jsonl`.
One JSON object per line:

```json
{"ts_offset_ms":0,"direction":"in","kind":"binary","len":42,"hex":"1b5b...","utf8_lossy":"..."}
```

| Scenario | Behavior |
|----------|----------|
| `attach` | Connect; capture ~3s of server seed / screen redraw. |
| `input` | Send `printf 'MARKER_A\n'; ls -la\n` as a binary frame; capture echo + output. |
| `resize` | Send `{"type":"resize","cols":120,"rows":40}` text frame; capture reflow. |
| `reconnect` | Start a ~3s emitting loop, hard-close ~1s in, wait 1s, re-attach same terminal; markers `BEFORE_DROP` / `AFTER_RECONNECT`. |

Close codes **4404** / **4405** / **4500** are logged with branch hints.

## Example

```bash
export OMNIGENT_BASE_URL=http://127.0.0.1:8000
export OMNIGENT_SESSION_ID=conv_abc123

cargo run -p terminal-attach -- --scenario attach --transport control
cargo run -p terminal-attach -- --scenario all --transport pty
```

## OpenAPI gaps (resolve live)

Pinned `vendor/omnigent-0.5.1/openapi.json` documents route semantics but:

- `POST /v1/sessions/{session_id}/resources/terminals` has **no `requestBody` schema** (response schema is also `{}`).
- Harness sends `{"terminal":"<name>","session_key":"<key>"}` per route description; confirm field names and whether `launch_args` / `ensure_native_terminal` markers are needed for your agent.
- `GET …/resources/terminals` response is untyped `{}`; list envelope is likely `SessionResourcePaginatedList` (`data` array) per `SessionResourceObject` in components — not wired on the route.
