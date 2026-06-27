# `lens-client` — implementation spec

**Status:** Accepted, 2026-06-25. 

**Design ground truth:** `typed-client.md`
(this doc does not restate the contract — it records the *build* decisions,
module structure, and order layered on top). **Pin:** omnigent `0.3.0.dev0`
(`36b2a11c`), frozen per `docs/adr/0001-omnigent-contract-pinning.md`.

## 1. Scope

Build the **entire** `lens-client` crate — the single typed seam over omnigent's
HTTP + SSE + WS contract — in dependency order, finished before the state-model
layer is started. `typed-client.md` owns *what* the crate does; this doc owns
*how* it's built and in *what order*.

## 2. Decisions

| #   | Decision                                                                                                                                                                                                                                                                                                                                                                                                            | Rationale                                                                                                                                                                                                                                                                                                                                                                                                                  |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | **Codegen: `typify`, one-shot scaffold.** `xtask codegen` extracts `components/schemas` (plain JSON Schema) from the vendored `openapi.json`, runs `typify`, writes a **committed** `generated.rs`. Re-run manually on re-vendor.                                                                                                                                                                                   | Sidesteps the OpenAPI **3.2.0** envelope + SSE `itemSchema` (we hand-write the event taxonomy anyway). Deterministic, reviewable diffs; no build-time fragility. (`generated.rs` is regenerated, never hand-edited — tweaks live in the hand-written wrapper layer.) `utoipa` was a red herring — it's Rust→OpenAPI, the wrong direction.                                                                                  |
| D2  | **Sync/blocking public API.** Methods are blocking `fn`; callers offload to background threads at the gpui seam (**one thread per active session**). SSE/WS: dedicated blocking OS thread → `std::sync::mpsc` → UI poller. **No async runtime, no tokio, no flume** in the crate.                                                                                                                                   | At our scale (tens–low-hundreds of active streams, self-bounded by 10-min auto-sleep — *no* hard cap) one blocking thread per stream is ample. tokio's headline benefit (I/O multiplexing) is bypassed by the blocking-reader-thread pattern (framework §2.1) regardless, so tokio would be cost without benefit. Sync→async is the *reversible* direction and is localized to this seam + its single consumer (`ActiveSession`). |
| D3  | **Local verification (no CI).** Golden-SSE captures → always-on `cargo test` (deterministic, no server). Live tests (taxonomy diff, endpoint reachability) gate behind `--features live-tests` + `LENS_OMNIGENT_URL`. A workspace `xtask` is the "local CI" home: `codegen`, `drift`, `live-test`.                                                                                                                  | We run everything locally. Keeps the default test loop fast and serverless; live checks are opt-in.                                                                                                                                                                                                                                                                                                                        |
| D4  | **Contract gate kept but understood as coarse on dev0.** `GET /api/version` returns `0.3.0.dev0` for *every* commit, so the runtime version gate only catches gross mismatches (e.g. a 0.2.0 server). Real drift protection on dev0 = **startup taxonomy diff** (§9.4) + **`xtask drift`** (vendored `openapi.json` vs sibling checkout). The gate regains precision when we pin a real `0.3.0` release (ADR-0001). | Honest about what the version string can and can't detect while upstream is unreleased. Full §3 surface is modeled (the whole crate).                                                                                                                                                                                                                                                                                      |

## 3. Workspace layout

```
crates/lens-client/
  src/
    lib.rs            # crate root, re-exports, PINNED_OMNIGENT_VERSION
    error.rs          # ClientError, Result alias
    ids.rs            # branded newtypes (macro-generated)
    connection.rs     # Connection, Auth, auth injection
    client.rs         # Client + subservice accessors; handshake/contract gate
    http.rs           # reqwest::blocking base, request builder, error mapping
    generated.rs      # typify output (committed) — DO NOT hand-edit
    sessions.rs       # Sessions subservice (REST §3, incl. SessionEventInput write path)
    resources.rs      # env-scoped fs/diff/search/shell, terminals REST, files
    hosts.rs          # hosts, runners
    agents.rs         # agents (read-only), policies
    info.rs           # /v1/info, /v1/me, /api/version, /health
    stream/           # SSE: parser, taxonomy (events.rs), blocking reader thread, normalization, dedup
    reconnect.rs      # no-replay protocol, three-bucket, stop-conditions
    terminal.rs       # WS attach (tungstenite) over the same blocking-thread pattern
  tests/
    golden/*.sse      # recorded captures (deterministic, no server)
    contract_*.rs     # golden parse tests (always-on)
    live_*.rs         # taxonomy-diff, reachability (feature = "live-tests")
crates/xtask/         # local "CI": codegen | drift | live-test
```

## 4. Build order (each unit lands independently, green)

1. **Scaffold** — `lib` / `error` / `ids` / `connection`. No network.
2. **Codegen** — `xtask codegen` + committed `generated.rs`. Validate `typify` maps the schemas cleanly (the one open implementation risk in D1).
3. **HTTP core + contract gate** — `http` / `client` / `info`; `Client::new` ready-ladder (`/health`→`/api/version`→`/v1/info`) + gate. First live call.
4. **REST surface** — `sessions` read → `sessions` write (`SessionEventInput`) → `resources`/terminals/comments → elicitations/labels/permissions/policies → `hosts`/runners/agents (5a–5e in `typed-client.md` §3).
5. **SSE** — parser + `ServerStreamEvent` taxonomy + blocking reader thread + normalization guarantees (§7a).
6. **Reconnect** — no-replay three-bucket protocol (§7), stop-immediately conditions.
7. **WS terminal** — attach client (§5). Independent of 5/6 after the reader-thread pattern exists.
8. **Verification consolidation** — golden captures per event family, `xtask drift`, gated live tests (§9).

Contract/unit tests are written *with* each unit, not bolted on at the end.

## 5. Local verification

- `cargo test` — golden-SSE parse + unit tests **+ the offline taxonomy-completeness
  test** (`taxonomy_drift`: the pinned openapi's `ServerStreamEvent` discriminator
  mapping must set-equal `MODELED_EVENT_TYPES ∪ DEFERRED_EVENT_TYPES`, and the two
  are disjoint — a new upstream event type fails here with no server). Always green.
- `xtask drift` **(built, Plan 3c)** — `cargo run -p xtask -- drift` diffs the vendored
  `openapi.json` vs the sibling pin (default `../omnigent/openapi.json`, override
  `--against <path>`): semantic path-set diff (excluding `/hooks/*` runner-callback
  routes per ADR-0001) + SSE discriminator-mapping diff + per-member property-name
  shape diff; non-zero exit on drift. The ADR-0001 "passive alarm," run locally.
  ⚠ Member-shape diff is property-**names** only (not requiredness/type/nullability/
  nested) — a deliberate scope bound; deepen to a canonicalized subtree compare if a
  field's *type* (vs presence) ever needs guarding.
- **Gated live checks** (`cargo test -p lens-client --features live-tests`, Plan 3c) —
  against a running daemon (`LENS_OMNIGENT_URL` + a claude-sdk `LENS_OMNIGENT_SESSION_ID`):
  `live_taxonomy` (every wire event type is modeled, or a *deferred* type legitimately
  surfacing as `Unknown` — a **modeled** type arriving as `Unknown` is drift) and
  `live_reachability` (every consumed read endpoint reachable: typed `Ok` or a typed
  domain error, never transport/decode). Both executed & green vs pinned `0.3.0.dev0`.

## 6. Recorded doc corrections / deferred items

- **`typed-client.md` §10** — public API is **blocking/sync**, not the sketched `async fn` (D2). Note added inline in §10.
- **`typed-client.md` §4** — removed a stale "~8 concurrent streams" cap; the active set self-bounds via auto-sleep with **no hard cap** (state-model §3.3). Corrected inline.
- **`app-architecture-and-state-model.md` §8** — corrected to the D2 shape:
  one `ActiveSession` background OS thread per Active session, sync channels to
  the foreground `SessionStore` replica, no tokio requirement on the core path.

## 7. Reversibility

D2 is a hinge, not a one-way door. The async/sync choice is contained to this
seam and its single consumer (`ActiveSession`). Sync→async is the easy
direction (wrap at the edge / swap the channel for one with an async receiver).
Revisit only if stream fan-out reaches the thousands — at which point the move is
to **fully** async I/O (drop blocking reader threads for an async reactor), a
crate-internal refactor behind stable public types, *not* a tokio actor bolt-on.
