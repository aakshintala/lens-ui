# lens-capture

CLI dev tool that wraps an interactive omnigent harness, subscribes to the
session SSE stream as soon as the harness creates it, and writes a golden
corpus on exit.

```sh
lens_capture omnigent claude     # zero flags — that's the whole thing
lens_capture omnigent codex
```

Captures auto-name into `docs/spikes/captures/live/<UTC-stamp>-<harness>.{stream.sse,
snapshot.json,items.json}` (e.g. `…/live/20260627-170641-claude.stream.sse`). Pass
`--out <prefix>` only to override the location.

### Race-free mode

When you already know the session id (`conv_...` — the same value omnigent shows in
its picker, a prior capture's summary, or `GET /v1/sessions`), subscribe before the
harness starts so no SSE events are missed:

```sh
lens_capture --session conv_abc123 omnigent claude
```

`--resume <id>` is appended to the harness command automatically unless `--resume` or
`-r` is already present.

**Environment**

- `LENS_OMNIGENT_URL` — base URL (required unless `--url` is passed)
- `LENS_OMNIGENT_TOKEN` — optional bearer token

**Output** (`<PREFIX>` defaults to `docs/spikes/captures/live/<UTC-stamp>-<harness>`):

- `<PREFIX>.stream.sse` — raw SSE bytes
- `<PREFIX>.snapshot.json` — final session snapshot
- `<PREFIX>.items.json` — final items page

**Bench wiring:** drop the `.stream.sse` under `docs/spikes/captures/...` and add
an entry to the `CORPORA` array in `crates/lens-client/benches/sse_pipeline.rs`.

**Known limitations (default mode is best-effort, fine for corpus growth):**

- *Subscribe-first is best-effort, not guaranteed.* Default mode detects the harness
  session by polling `GET /v1/sessions`, then subscribes. The stream endpoint does
  not replay, so any events emitted before the subscription lands are missed. In
  practice the session is created during the harness's idle startup (before you
  type the first prompt), so it's caught in time. If the harness creates the
  session lazily on the first message, send a throwaway first prompt (`hi`), let it
  complete, then do the real work on turn 2+. Use `--session conv_...` for
  race-free capture when you know the id and the harness supports `--resume`.
- *Session pick is a single-harness heuristic.* The first session id not present
  before spawn is assumed to be the harness's. Don't run two session-creating
  things at once against the same server during a capture.
- *Capture stops when the harness exits.* The `.stream.sse` is complete up to the
  last chunk written before exit (writes are unbuffered/write-through). A non-zero
  harness exit (incl. Ctrl-C quit) is not treated as a capture failure.
