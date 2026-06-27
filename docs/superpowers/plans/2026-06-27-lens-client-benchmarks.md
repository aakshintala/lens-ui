# Plan — lens-client benchmarks (Category 1 micro + Category 2 live overhead)

Closes a MANDATORY gap: `.agents/performance.md` requires `lens-client` to ship
benchmarks ("SSE/WS parse + codec throughput (`criterion`)"); none exist. Splits
the work into the two things "benchmark" means here — deterministic CPU-overhead
(gateable) vs. live omnigent round-trip overhead (informational).

## Why two categories

- **Category 1 — CPU typing-pass cost.** Pure functions over bytes, deterministic,
  sub-µs–µs. `criterion`, golden-corpus inputs, the thing the perf doc mandates and
  the thing that can become a regression gate. Proves/kills "trivial typing pass".
- **Category 2 — omnigent round-trip overhead.** Network + server bound, noisy,
  non-deterministic. Informational only, never a gate. Answers "how much of
  time-to-paint is omnigent vs. us" — the actual open question.

## Category 1 — `benches/sse_pipeline.rs` (criterion)

Pipeline mirrors the reader thread (`stream/reader.rs`): bytes → frames → typed
event → normalized events. All three stages are `pub(crate)`, so:

- **Exposure:** add a `bench` cargo feature; behind `#[cfg(feature = "bench")]`
  re-export a `#[doc(hidden)] pub mod bench_api` from `stream/mod.rs` exposing
  `SseParser`, `SseFrame`, `parse_event`, `Normalizer`. Public API stays clean;
  the surface only compiles under `--features bench`.
- **Dep:** `criterion = { version = "0.5", features = ["html_reports"] }` (dev).
  `[[bench]] name = "sse_pipeline", harness = false, required-features = ["bench"]`.
- **Inputs:** `include_bytes!` the golden corpus
  (`docs/spikes/captures/2026-06-26-sse/{happy_path,interrupt,reasoning_effort_high}.stream.sse`)
  — hermetic, no IO in the measured loop. Ground-truth bytes from the Plan 3 spike.
- **Benches** (each with `Throughput::Bytes` for MB/s):
  - `sse_frame_parse` — `SseParser::push(&[u8])` over a full corpus file (exercises
    the byte-buffer + split-UTF-8 path).
  - `event_decode` — `parse_event(&frame)` per frame (the serde typing cost;
    bench representative variants: item delta, snapshot, chrome, error).
  - `normalize` — `Normalizer::push(ev)` over the decoded stream.
  - `full_pipeline` — bytes → normalized events end-to-end (per-chunk reader cost).
- **Run:** `cargo bench -p lens-client --features bench` (release implicit).

## Category 2 — `tests/live_overhead.rs` (`#![cfg(feature = "live-tests")]`)

Matches the existing live-test convention (`LENS_OMNIGENT_URL` +
`LENS_OMNIGENT_SESSION_ID`, `--nocapture`, print don't assert). Reports a table:

- **REST round-trip:** N samples of `send_event` and a session read; p50/p90 wall.
- **Time-to-first-event:** subscribe → post → first typed event.
- **Inter-event gap vs. parse cost:** measure gaps between events over one turn;
  alongside, time `parse_event` on the same frames. Ratio = the I/O-bound proof.
- Soft-assert it completes; numbers are the deliverable. Run:
  `LENS_OMNIGENT_URL=… LENS_OMNIGENT_SESSION_ID=… cargo test -p lens-client
  --features live-tests --test live_overhead -- --nocapture`.

## Follow-up — grow the golden corpus

Current corpus is ~22KB across 3 short captures — thin for throughput signal and
light on variant diversity (compact/elicitation/sub-agent/terminal still deferred).
Capture **longer sessions** and ideally **a real work pass on Lens itself** (more
turns, tool calls, larger reasoning + output payloads) per the live-event-recapture
procedure, and add them as bench inputs. The harness loads any corpus file via
`include_bytes!`, so new captures drop in without code changes. Do this after the
harness lands so we benchmark against representative, not toy, streams.

## Out of scope (flagged, not gaps)

- **WS codec benches** — WS path not built in lens-client yet; benches land with it.
- **state-store / render benches** — those crates don't exist yet; benches ship
  with them per the perf doc's "every layer" rule.
- **CI regression gating** — no CI workflow exists (solo merge-to-main). Record a
  baseline now (`target/criterion` + a number in STATUS); wire the 90fps-line gate
  when CI lands.

## Process

- Build via `cursor-delegate` / `composer-2.5` (mechanical: bench harness + feature
  wiring). Keep `generated.rs` untouched.
- Gate: `cargo bench --features bench` runs clean; live harness compiles under
  `--features live-tests`; `clippy --all-targets` + `fmt` clean (benches included).
- Cross-family review (gpt-5.5 or codex/gemini-3.5) — different family than author.
- Capture the baseline parse number + omnigent-overhead ratio in STATUS / memory.
