# lens-capture

CLI dev tool that wraps an interactive omnigent harness, subscribes to the
session SSE stream as soon as the harness creates it, and writes a golden
corpus on exit.

```sh
lens_capture omnigent claude
lens_capture --out docs/spikes/captures/2026-06-27-sessionstore/work omnigent codex
```

**Environment**

- `LENS_OMNIGENT_URL` — base URL (required unless `--url` is passed)
- `LENS_OMNIGENT_TOKEN` — optional bearer token

**Output** (prefix defaults to `./capture`):

- `<PREFIX>.stream.sse` — raw SSE bytes
- `<PREFIX>.snapshot.json` — final session snapshot
- `<PREFIX>.items.json` — final items page

**Bench wiring:** drop the `.stream.sse` under `docs/spikes/captures/...` and add
an entry to the `CORPORA` array in `crates/lens-client/benches/sse_pipeline.rs`.

**Known limitations (best-effort capture, fine for corpus growth):**

- *Subscribe-first is best-effort, not guaranteed.* The tool detects the harness
  session by polling `GET /v1/sessions`, then subscribes. The stream endpoint does
  not replay, so any events emitted before the subscription lands are missed. In
  practice the session is created during the harness's idle startup (before you
  type the first prompt), so it's caught in time. If the harness creates the
  session lazily on the first message, send a throwaway first prompt (`hi`), let it
  complete, then do the real work on turn 2+. A fully race-free design (create the
  session via the API → subscribe → attach the harness to that id) depends on the
  harness supporting attach-to-existing-session.
- *Session pick is a single-harness heuristic.* The first session id not present
  before spawn is assumed to be the harness's. Don't run two session-creating
  things at once against the same server during a capture.
- *Capture stops when the harness exits.* The `.stream.sse` is complete up to the
  last chunk written before exit (writes are unbuffered/write-through). A non-zero
  harness exit (incl. Ctrl-C quit) is not treated as a capture failure.
