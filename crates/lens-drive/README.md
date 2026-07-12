# lens-drive

Headless JSON-lines driver for the `lens-core` session actor. Connects to a live
omnigent server, subscribes to the session SSE stream, and drives the actor via
newline commands from stdin or a script file.

## Bright line

**lens-drive dumps state as JSON; it never renders.** No markdown, no transcript
layout, no virtualization. The moment it grows a rendering concern, that work
belongs in `lens-ui`, not here.

## Usage

```bash
lens-drive --base-url http://localhost:8080 --session conv_abc123
lens-drive --base-url http://localhost:8080 --session conv_abc123 --script commands.txt
lens-drive --base-url http://localhost:8080 --session conv_abc123 --data-dir /tmp/lens-drive-data
```

### Commands (stdin or `--script`)

| Line | Action |
| --- | --- |
| `send <text>` | Optimistic user message |
| `sleep` | Durable sleep (actor stops when quiescent) |
| `reconnect` | Respawn via `FleetScheduler::reconnect` (prints `live_status`) |
| `stop` | Stop the actor and exit |
| `snapshot` | `Promote` → print current `SessionState` as JSON |

### Output (stdout, one JSON object per line)

- `{"kind":"outcome","outcome":…}` — actor outcomes (`SendAccepted`, `Parked`, …)
- `{"kind":"state","state":…}` — compact `SessionState` on each `Rebased` update
- `{"kind":"reconnect","live_status":"idle"|"running"|"failed"|null}` — D26 re-read

### Environment

- `LENS_OMNIGENT_URL` — default base URL when `--base-url` is omitted
- `LENS_OMNIGENT_TOKEN` — optional bearer token

Connect + resolve the `conv_…` session id the same way as `lens-capture` (pass
`--session` with the id from omnigent's session picker or `GET /v1/sessions`).
