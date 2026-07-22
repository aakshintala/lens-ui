# Terminal Slice 4 — Lifecycle Mechanisms Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `lens-terminal`'s lifecycle *mechanisms* correct — a full resource-generation guard, Sleep/Wake teardown-and-recreate, `ReplacementWaiting` exact-key successor adoption, an explicit Reattach action for `4405`/`ClientDetached`, and a non-panicking engine-spawn-failure policy — all host-agnostic module code the demo and deterministic tests drive.

**Architecture:** The module already owns confirmed-worker-exit teardown (`runtime.rs`), close-code policy (`policy.rs`), off-thread discover/attach (`policy::discover_and_attach`), and a reconnect loop (`lib.rs`). Slice 4 adds a **pure correlation reducer** (`generation.rs`) fed by two new normalized `TerminalHostEvent` signals; the tab consumes its verdicts to enter `ReplacementWaiting`/`Detached`/adopt. Sleep/Wake/Reattach are new arms on the existing `on_host_event` seam. All new outward failure paths resolve to `Lifecycle` values, never panics.

**Tech Stack:** Rust, gpui `Entity`/`Context`, `crossbeam-channel` + `async-channel`, vendored `libghostty-vt`. Tests: in-crate `#[gpui::test]` + real-`Application` harnesses (existing), offline unit tests, `stub_for_test` client, `terminal_live` opt-in rider.

## Global Constraints

Copied verbatim in intent from `docs/specs/2026-07-16-terminal-workstream-design.md` (§Threading, §Lifecycle, §Module ownership, Build sequence Slice 4) and `CLAUDE.md`:

- **Pure `lens-terminal` + demo + xtask only.** No `lens-ui`/`lens-core` edits — `terminal-ws → main` must stay mergeable after this slice.
- **The tab renders modeled values and never panics.** Every failure path resolves to one of the 7 frozen `Lifecycle` variants. New outward-facing fallibility (engine spawn) becomes a `Detached` variant, not an `.expect()`.
- **No Ghostty type ever escapes the engine boundary.** New seams carry only Lens-owned values.
- **Teardown = signal stop → worker drains + exits → off-foreground `join`.** Never `join` on the gpui foreground thread (Drop/`teardown_*` already spawn off-thread; reuse them).
- **`Lifecycle` variants stay payload-free + `Copy` permanently.** Detail rides `Presentation` (`detached_detail`, etc.).
- **`Ended` stays inert** — no 0.5.1/0.6.0 positive process-termination signal exists (verified). Never enter `Ended`; ambiguous disappearance is `Detached`.
- **Host-agnostic mechanisms.** The module owns Sleep/Wake/Reattach/adoption; a host merely *invokes* them via `TerminalHostEvent`. No host is required to exist.
- **`#[non_exhaustive]` seams grow additively.** New `TerminalHostEvent`/`DetachedDetail` variants must not break existing `match`es.
- **Every layer keeps its `Inspect` contract** with zero hot-path cost while disabled; extend inspect where Slice 4 adds observable state.
- **Verification gates:** `rustfmt` + `cargo clippy --workspace --all-targets -- -D warnings` (both configs) + all crate tests + `xtask gate` green. Frequent commits, TDD, ≥1 cross-family review at each seam (author = composer-2.5 → mandatory non-composer review, default `codex`/`gpt-5.6-sol` read-only).
- **Contract ground truth (verified 2026-07-22, memory `terminal-resource-event-granularity`):** `session.resource.created` carries the full resource (id + `terminal_name` + `session_key`); `session.resource.deleted` carries only `resource_id`. Deterministic id = `terminal_{name}_{key}`. On agent switch, delete is emitted **synchronously** before any successor create (structural). Delivery is best-effort (documented missed-event race).

---

## Design note — one deliberate deviation from spec §Identity (RATIFIED 2026-07-22; residual is the accepted no-token gap)

Spec §Identity says "same-ID recreation **outside** a positively identified reset → freeze → `Detached`." Encoding that literally makes a `resource.created` that matches our own attached identity **with no preceding delete** trigger `Detached`. But our own terminal's `created` echo can legitimately arrive on the host event stream just after we attach (stream lag), which would spuriously detach a healthy live tab.

**Decision (ratified):** treat a matching `created` as significant **only when a `deleted` was positively observed first** (→ adopt). A matching `created` with no prior delete → **`Unchanged`** (benign echo). Rationale: a false `Detached` on a self-echo is a *new*, user-visible spurious failure; the alternative failure is *narrow and pre-existing*.

**Honest residual (corrected after Grok-4.5 review — do NOT claim reconnect recovers this; severity sharpened by a source trace):** the reducer's live-stream guard reliably handles every **observed** signal (a `deleted` for our id → `ReplacementWaiting`/`Detached`; delete→create → adopt fresh engine). The unresolved case is a **missed `deleted`** (host-stream gap) followed by a retryable WS close (`Network`/`Internal`, not `4404`): `preflight_reconnect` only GETs *existence* — it does **not** detect a new generation (there is **no immutable generation token**; omnigent's own `session_resources.py:34` says durable resource-id generation "does not exist" yet, and the resource metadata carries nothing generation-distinguishing). So the reconnect path retains the **old** engine and resumes `Live` + `output_gap` on a possibly-new PTY generation → a silent scrollback-mix window.

**Why this is narrow + mild (source-verified 2026-07-22):**
- **The common replacement path is already correct.** When a terminal is genuinely torn down server-side (agent reset / session ended), the attach WS closes with **`4404`** (`omnigent/terminals/ws_bridge.py:80-83`, "sent on PTY EOF when the tmux session is genuinely gone") → client `TerminalNotFound` → `StopDetached(TerminalGone)` → clean `Detached`, **no** reconnect, **no** mix. The residual requires the replacement to coincide with a *transport-level* non-`4404` drop — a rare triple-coincidence (replaced ∧ missed live delete ∧ non-`4404` close).
- **Even then the symptom is mild:** the reconnect seed is a clear + current-screen redraw, so the **active viewport is correct**; only *scrollback* shows stale prior-generation lines, with the `output_gap` marker already displayed. Recoverable by manual recreate.

We accept it as the upstream no-token gap (ratified 2026-07-22). Task 5 adds a **documenting test** (Network-close + same-id GET retains engine) so the behavior is pinned, not implied. **When it closes:** practically narrowed at **Slice 5** (the `FleetStore`'s *persistent* host session-event subscription shrinks the missed-live-delete window — the module guard already consumes those signals); truly closed only by an **upstream omnigent durable-generation token** (filed as a SPEC-GAP). The reducer encodes the echo choice with a loud comment; the "full generation guard on the reconnect path" completion-matrix row is **explicitly deferred** with the SPEC-GAP citation, not "covered."

---

## File Structure

- **Create `crates/lens-terminal/src/generation.rs`** — the pure correlation reducer: `ResourceSignal`, `GenerationVerdict`, `GenerationGuard`. No gpui, no I/O. One responsibility: given our attached identity + target class + a normalized resource signal, decide the lifecycle consequence. Fully unit-testable offline.
- **Modify `crates/lens-terminal/src/lib.rs`** — new `TerminalHostEvent` variants + `DetachedDetail` variants; new `TerminalTab` fields (`generation`, `reconnect_epoch`); new arms in `on_host_event`; adoption + Sleep/Wake/Reattach handlers; reconnect-cancellation guard in `on_reconnect_success`; `mod generation;`.
- **Modify `crates/lens-terminal/src/engine/worker.rs`** — `spawn_worker` returns `Result<JoinHandle<()>, std::io::Error>` (remove the `.expect` at :396).
- **Modify `crates/lens-terminal/src/engine/forwarder.rs`** — `InputForwarder::spawn` returns `Result<Self, std::io::Error>` (remove the `.expect` at :58).
- **Modify `crates/lens-terminal/src/engine/handle.rs`** — `EngineHandle::spawn`/`spawn_from_parts` return `Result<Self, EngineSpawnError>`; define `EngineSpawnError`. Fix test call sites.
- **Modify `crates/lens-terminal/src/policy.rs`** — `discover_and_attach` maps engine-spawn failure to `DetachedDetail::EngineSpawnFailed`.
- **Modify `crates/lens-terminal/src/inspect.rs`** (or `engine/inspect.rs`) — surface `saw_delete`/`replacement_waiting` counters if an `Inspect` field is the cheapest way to assert generation state in tests; otherwise skip.
- **Modify `crates/lens-terminal-demo/src/main.rs`** — key/menu bindings that feed `Sleep`/`Wake`/`Reattach` + a scripted reset (feed `ResourceDeleted` then `ResourceCreated`) so the demo drives every mechanism.
- **Modify `crates/lens-terminal/tests/terminal_live.rs`** — opt-in `P7` Sleep→Wake reattach + `P8` Reattach-after-detach riders against a real omnigent shell.

---

## Task 1: Pure correlation reducer (`generation.rs`) + new seam types

**Files:**
- Create: `crates/lens-terminal/src/generation.rs`
- Modify: `crates/lens-terminal/src/lib.rs` (add `mod generation;`; extend `TerminalHostEvent` and `DetachedDetail`)
- Test: unit tests inside `generation.rs`

**Interfaces:**
- Produces (consumed by Tasks 3–6):
  - `pub(crate) enum ResourceSignal { Created { session_id: SessionId, terminal_id: TerminalId, terminal_name: String, session_key: String }, Deleted { terminal_id: TerminalId } }`
  - `pub(crate) enum GenerationVerdict { Unchanged, AwaitReplacement, AdoptSuccessor { session_id: SessionId, terminal_id: TerminalId }, Detach(DetachedDetail) }`
  - `pub(crate) struct GenerationGuard` with `fn new(tid: TerminalId, key: Option<TerminalKey>) -> Self`, `fn on_signal(&mut self, signal: &ResourceSignal) -> GenerationVerdict`, `fn is_dirty(&self) -> bool`.
  - `TerminalHostEvent::ResourceCreated { session_id: SessionId, terminal_id: TerminalId, terminal_name: String, session_key: String }` and `TerminalHostEvent::ResourceDeleted { terminal_id: TerminalId }` (normalized host signals).
  - `DetachedDetail::{ IdentityChanged, ReplacementTimedOut, EngineSpawnFailed }`.

- [ ] **Step 1: Add the new `DetachedDetail` variants**

In `lib.rs`, extend the enum (keep `Copy`; these are payload-free):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DetachedDetail {
    TerminalGone,
    ClientDetached,
    Unauthorized,
    RetriesExhausted,
    DiscoveryFailed,
    EngineStopped,
    /// A `resource.created` proved our attached id is a new generation, or a
    /// wake found the observed generation did not survive.
    IdentityChanged,
    /// `ReplacementWaiting` elapsed without an exact-key successor appearing.
    ReplacementTimedOut,
    /// The engine worker/forwarder thread could not be spawned (resource
    /// exhaustion). Never a panic — modeled as a lifecycle value.
    EngineSpawnFailed,
}
```

- [ ] **Step 2: Add the two normalized host-event signals**

In `lib.rs`, extend `TerminalHostEvent` (already `#[non_exhaustive]`):

```rust
    /// Normalized `session.resource.created` for a terminal in this session.
    /// The host forwards these verbatim; the tab filters to its own identity.
    ResourceCreated {
        session_id: SessionId,
        terminal_id: TerminalId,
        terminal_name: String,
        session_key: String,
    },
    /// Normalized `session.resource.deleted` (server carries only the id).
    ResourceDeleted { terminal_id: TerminalId },
```

- [ ] **Step 2b: Keep the same-crate match exhaustive (commit-green)**

`on_host_event` is an exhaustive `match` in the same crate, so adding variants without arms fails to compile — Task 1's commit must stay green. Add explicit no-op arms now; Task 3 replaces them with real wiring:

```rust
TerminalHostEvent::ResourceCreated { .. } | TerminalHostEvent::ResourceDeleted { .. } => {
    // Wired in Task 3 (correlation). No-op until then.
}
```

(Do the same for `Reattach` when Task 5 adds it — add a no-op arm in the same commit as the variant.)

- [ ] **Step 3: Write failing unit tests for the reducer**

Create `crates/lens-terminal/src/generation.rs` with a `#[cfg(test)] mod tests` covering the full table. Use real `TerminalId`/`SessionId`/`TerminalKey`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use lens_client::ids::{SessionId, TerminalId};

    fn key(name: &str, sk: &str) -> TerminalKey {
        TerminalKey { terminal_name: name.into(), session_key: sk.into() }
    }
    fn created(sid: &str, tid: &str, name: &str, sk: &str) -> ResourceSignal {
        ResourceSignal::Created {
            session_id: SessionId::new(sid),
            terminal_id: TerminalId::new(tid),
            terminal_name: name.into(),
            session_key: sk.into(),
        }
    }
    fn deleted(tid: &str) -> ResourceSignal {
        ResourceSignal::Deleted { terminal_id: TerminalId::new(tid) }
    }

    #[test]
    fn open_or_create_delete_then_matching_create_adopts() {
        let mut g = GenerationGuard::new(TerminalId::new("t1"), Some(key("bash", "s1")));
        assert_eq!(g.on_signal(&deleted("t1")), GenerationVerdict::AwaitReplacement);
        assert!(g.is_dirty());
        match g.on_signal(&created("sess", "t1", "bash", "s1")) {
            GenerationVerdict::AdoptSuccessor { session_id, terminal_id } => {
                assert_eq!(session_id.as_str(), "sess");
                assert_eq!(terminal_id.as_str(), "t1");
            }
            other => panic!("expected AdoptSuccessor, got {other:?}"),
        }
    }

    #[test]
    fn open_or_create_create_without_delete_is_benign_echo() {
        // Deliberate deviation from spec §Identity — see plan design note.
        let mut g = GenerationGuard::new(TerminalId::new("t1"), Some(key("bash", "s1")));
        assert_eq!(g.on_signal(&created("sess", "t1", "bash", "s1")), GenerationVerdict::Unchanged);
        assert!(!g.is_dirty());
    }

    #[test]
    fn open_or_create_wrong_key_create_is_unchanged() {
        let mut g = GenerationGuard::new(TerminalId::new("t1"), Some(key("bash", "s1")));
        assert_eq!(g.on_signal(&deleted("t1")), GenerationVerdict::AwaitReplacement);
        assert_eq!(g.on_signal(&created("sess", "t9", "zsh", "s2")), GenerationVerdict::Unchanged);
    }

    #[test]
    fn existing_delete_detaches_terminal_gone() {
        let mut g = GenerationGuard::new(TerminalId::new("t1"), None);
        assert_eq!(
            g.on_signal(&deleted("t1")),
            GenerationVerdict::Detach(DetachedDetail::TerminalGone)
        );
    }

    #[test]
    fn existing_ignores_create_for_our_id_without_delete() {
        // Existing never adopts; a bare create echo is benign (missed-event gap on next reconnect).
        let mut g = GenerationGuard::new(TerminalId::new("t1"), None);
        assert_eq!(g.on_signal(&created("sess", "t1", "bash", "s1")), GenerationVerdict::Unchanged);
    }

    #[test]
    fn unrelated_delete_is_unchanged() {
        let mut g = GenerationGuard::new(TerminalId::new("t1"), Some(key("bash", "s1")));
        assert_eq!(g.on_signal(&deleted("other")), GenerationVerdict::Unchanged);
        assert!(!g.is_dirty());
    }
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test -p lens-terminal generation:: 2>&1 | tail -20`
Expected: FAIL — `cannot find type GenerationGuard` / module not implemented.

- [ ] **Step 5: Implement the reducer**

Write the module body above the test mod in `generation.rs`:

```rust
//! Pure resource-generation correlation (Slice 4). No gpui, no I/O.
//!
//! Given our attached identity + target class, decide the lifecycle
//! consequence of a normalized `session.resource.created`/`.deleted` signal.
//! The `saw_delete`-before-`created` discriminator is the positive-reset test;
//! see the plan design note for why a create without a prior delete is benign.

use lens_client::ids::{SessionId, TerminalId};

use crate::{DetachedDetail, TerminalKey};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResourceSignal {
    Created {
        session_id: SessionId,
        terminal_id: TerminalId,
        terminal_name: String,
        session_key: String,
    },
    Deleted {
        terminal_id: TerminalId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GenerationVerdict {
    /// Nothing relevant changed.
    Unchanged,
    /// Positive reset for an `OpenOrCreate` key: enter `ReplacementWaiting`.
    AwaitReplacement,
    /// The exact-key successor arrived after a delete: adopt it (fresh engine).
    AdoptSuccessor {
        session_id: SessionId,
        terminal_id: TerminalId,
    },
    /// Identity gone/changed with no adoptable successor.
    Detach(DetachedDetail),
}

/// Per-attachment correlation state. Rebuilt on every fresh attach
/// (initial open, adoption, wake) so `saw_delete` never leaks across identities.
pub(crate) struct GenerationGuard {
    tid: TerminalId,
    /// `Some` => `OpenOrCreate` (adoptable by key); `None` => `Existing` (never adopts).
    key: Option<TerminalKey>,
    saw_delete: bool,
}

impl GenerationGuard {
    pub fn new(tid: TerminalId, key: Option<TerminalKey>) -> Self {
        Self { tid, key, saw_delete: false }
    }

    /// True once a `resource.deleted` for our id has been observed — used by
    /// Sleep/Wake to test whether the same observed generation survived.
    pub fn is_dirty(&self) -> bool {
        self.saw_delete
    }

    pub fn on_signal(&mut self, signal: &ResourceSignal) -> GenerationVerdict {
        match signal {
            ResourceSignal::Deleted { terminal_id } if *terminal_id == self.tid => {
                self.saw_delete = true;
                match self.key {
                    Some(_) => GenerationVerdict::AwaitReplacement,
                    None => GenerationVerdict::Detach(DetachedDetail::TerminalGone),
                }
            }
            ResourceSignal::Created {
                session_id,
                terminal_id,
                terminal_name,
                session_key,
            } => match &self.key {
                Some(k)
                    if k.terminal_name == *terminal_name && k.session_key == *session_key =>
                {
                    if self.saw_delete {
                        GenerationVerdict::AdoptSuccessor {
                            session_id: session_id.clone(),
                            terminal_id: terminal_id.clone(),
                        }
                    } else {
                        // DEVIATION (plan design note): a matching create with NO
                        // prior delete is our own echo / a missed-delete recreation.
                        // Detaching here would spuriously kill a healthy tab on a
                        // lagged self-echo; degrade to the accepted missed-event gap.
                        GenerationVerdict::Unchanged
                    }
                }
                _ => GenerationVerdict::Unchanged,
            },
            ResourceSignal::Deleted { .. } => GenerationVerdict::Unchanged,
        }
    }
}
```

Add `mod generation;` next to the other `mod` declarations in `lib.rs`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p lens-terminal generation:: 2>&1 | tail -20`
Expected: PASS (6 tests).

- [ ] **Step 7: Serde round-trip for the new `DetachedDetail` variants**

Add to the existing serde tests in `lib.rs` (mirroring the current `detached_detail` coverage): assert each new variant round-trips. Run `cargo test -p lens-terminal detached 2>&1 | tail -10`. Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/lens-terminal/src/generation.rs crates/lens-terminal/src/lib.rs
git commit -m "feat(terminal-4): pure resource-generation reducer + host-signal/detach seam types"
```

---

## Task 2: Engine-spawn-failure policy (fold the Slice-3 Minor)

**Files:**
- Modify: `crates/lens-terminal/src/engine/worker.rs:220-396` (`spawn_worker` → fallible)
- Modify: `crates/lens-terminal/src/engine/forwarder.rs:41-58` (`InputForwarder::spawn` → fallible)
- Modify: `crates/lens-terminal/src/engine/handle.rs:62-134` (`EngineHandle::spawn`/`spawn_from_parts` → `Result`; add `EngineSpawnError`)
- Modify: `crates/lens-terminal/src/policy.rs:147-153` (`discover_and_attach` maps to `DetachedDetail::EngineSpawnFailed`)
- Modify: call sites in `lib.rs` (`with_engine_for_test`, tests) and `engine/mod.rs` re-exports
- Test: unit test in `handle.rs` for the injected-failure → `Err` path; mapping test in `policy.rs`

**Interfaces:**
- Consumes: `DetachedDetail::EngineSpawnFailed` (Task 1).
- Produces: `EngineHandle::spawn(cfg) -> Result<EngineHandle, EngineSpawnError>`; `pub struct EngineSpawnError` (opaque, `Debug`/`Display`). `discover_and_attach` continues to return `Result<AttachedParts, DetachedDetail>`, now also mapping engine-spawn failure.

- [ ] **Step 1: Write the failing mapping test in `policy.rs`**

A real thread-spawn failure can't be triggered deterministically, so exercise the **real conversion** production uses (a `From<EngineSpawnError> for DetachedDetail` impl — Step 5), built from a synthetic `io::Error`. This is not tautological: it runs the same `From` that `discover_and_attach` calls, over a real `EngineSpawnError` value (Grok Nit 15).

```rust
#[test]
fn engine_spawn_error_converts_to_engine_spawn_failed() {
    let err = crate::engine::EngineSpawnError::from_io_for_test(
        std::io::Error::new(std::io::ErrorKind::Other, "no threads"),
    );
    let detail: DetachedDetail = err.into();
    assert_eq!(detail, DetachedDetail::EngineSpawnFailed);
}
```

Add `#[cfg(any(test, feature = "test-util"))] pub fn from_io_for_test(e: std::io::Error) -> Self { Self(e) }` to `EngineSpawnError` so the test can mint one.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p lens-terminal engine_spawn_error_maps 2>&1 | tail -10`
Expected: FAIL — `map_engine_spawn_error` not found.

- [ ] **Step 3: Make the worker + forwarder spawns fallible**

`worker.rs`: change the signature and return the `io::Error` instead of `.expect`:

```rust
pub(crate) fn spawn_worker(/* …unchanged args… */) -> std::io::Result<JoinHandle<()>> {
    // …unchanged stack-size comment…
    thread::Builder::new()
        .stack_size(WORKER_STACK_BYTES)
        .spawn(move || { /* …unchanged body… */ })
    // NOTE: no `.expect` — propagate the spawn error up as a lifecycle value.
}
```

`forwarder.rs`: `pub(crate) fn spawn(...) -> std::io::Result<Self>` — return `Ok(Self { … })`, propagate the builder error with `?`.

- [ ] **Step 4: Make `EngineHandle::spawn` return `Result` and define `EngineSpawnError`**

`handle.rs`:

```rust
#[derive(Debug)]
pub struct EngineSpawnError(std::io::Error);

impl std::fmt::Display for EngineSpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "failed to spawn engine thread: {}", self.0)
    }
}
impl std::error::Error for EngineSpawnError {}

impl EngineHandle {
    pub fn spawn(cfg: EngineConfig) -> Result<Self, EngineSpawnError> {
        let worker::WorkerChannels { cmd_tx, cmd_rx, presentation_tx, presentation_rx } =
            worker::worker_channels();
        Self::spawn_from_parts(cfg, cmd_tx, cmd_rx, presentation_tx, presentation_rx)
    }

    fn spawn_from_parts(/* …unchanged args… */) -> Result<Self, EngineSpawnError> {
        // …unchanged setup…
        let input_forwarder = InputForwarder::spawn(cmd_tx.clone(), Arc::clone(&access_epoch))
            .map_err(EngineSpawnError)?;
        // If the worker fails to spawn, the forwarder thread is already live — reclaim
        // it before returning Err, or it leaks until process exit (Grok Imp 9). This
        // runs off-foreground (discover_and_attach's background task), so joining is safe.
        let join = match worker::spawn_worker(/* …unchanged args… */) {
            Ok(join) => join,
            Err(e) => {
                input_forwarder.sever_and_join(); // confirm the forwarder exits
                return Err(EngineSpawnError(e));
            }
        };
        Ok(Self { /* …unchanged fields… */ })
    }
}
```

Verify `InputForwarder` exposes a synchronous stop+join (grep `forwarder.rs` for `sever_and_join`/`join`/`stop`; the tab teardown already relies on the forwarder stopping — reuse that method name). Re-export `EngineSpawnError` from `engine/mod.rs` and `lib.rs`. Update the **real** test constructors — `spawn_with_cmd_cap` and `spawn_with_cmd_cap_for_test` (there is **no** `spawn_from_parts_for_test` — Grok Minor 13) — to `.expect("spawn")` the `Result` inside `#[cfg(test)]` (panicking in a test is fine; the production path must not). Grep first: `rg 'spawn_from_parts|spawn_with_cmd_cap' crates/lens-terminal`.

- [ ] **Step 5: Update production call sites**

Define the conversion once (in `lib.rs` near `DetachedDetail`, or `engine/mod.rs`), used by both production and the Step-1 test:

```rust
impl From<engine::EngineSpawnError> for DetachedDetail {
    fn from(_: engine::EngineSpawnError) -> Self {
        DetachedDetail::EngineSpawnFailed
    }
}
```

`policy.rs::discover_and_attach` (currently `let engine = Arc::new(EngineHandle::spawn(cfg));`):

```rust
let engine = Arc::new(EngineHandle::spawn(cfg).map_err(DetachedDetail::from)?);
```

`lib.rs::with_engine_for_test` and every `EngineHandle::spawn(...)` in tests: append `.expect("spawn engine for test")`. Grep to catch all: `rg 'EngineHandle::spawn\(' crates/lens-terminal`.

- [ ] **Step 6: Run the mapping test + full crate build**

Run: `cargo test -p lens-terminal engine_spawn_error_maps 2>&1 | tail -10` → PASS.
Run: `cargo build -p lens-terminal --all-targets 2>&1 | tail -20` → builds clean.

- [ ] **Step 7: Run the existing engine + lifecycle suites (no regressions)**

Run: `cargo test -p lens-terminal --lib 2>&1 | tail -15`
Expected: PASS (all existing tests, now against the `Result` signature).

- [ ] **Step 8: Commit**

```bash
git add crates/lens-terminal/src/engine crates/lens-terminal/src/policy.rs crates/lens-terminal/src/lib.rs
git commit -m "fix(terminal-4): fallible engine spawn → Detached(EngineSpawnFailed), never panic (Slice-3 Minor)"
```

---

## Task 3: Wire the reducer into `on_host_event` + reconnect cancellation

**Files:**
- Modify: `crates/lens-terminal/src/lib.rs` — `TerminalTab` fields (`generation: Option<GenerationGuard>`, `reconnect_epoch: u64`); `on_host_event` arms for `ResourceCreated`/`ResourceDeleted`; set the guard in `on_attached`; bump/check `reconnect_epoch` in `on_reconnect_success` + teardown paths; `ReplacementWaiting` entry helper
- Test: in-crate `#[gpui::test]`-style tests using `with_engine_for_test`

**Interfaces:**
- Consumes: `GenerationGuard`, `ResourceSignal`, `GenerationVerdict` (Task 1); `on_attached`, `on_detach`, `teardown_runtime_full`, `on_reconnect_success` (existing).
- Produces (consumed by Task 4): `fn enter_replacement_waiting(&mut self, cx)` — freezes frame, tears down the dead engine, sets `Lifecycle::ReplacementWaiting`, arms the bounded timeout; `fn adopt_successor(&mut self, session_id, terminal_id, cx)` (stub in this task, filled in Task 4). `reconnect_epoch` cancellation contract.

- [ ] **Step 1: Add tab fields + guard construction**

Add to `TerminalTab`:
```rust
    /// Resource-generation correlation for the current attachment (None until first attach).
    generation: Option<generation::GenerationGuard>,
    /// Cancellation token for in-flight reconnect loops. Any transition OUT of the
    /// reconnect path (adopt / replacement / sleep / detach) bumps this; a stale
    /// `on_reconnect_success` whose captured epoch mismatches is dropped (attach closed off-thread).
    reconnect_epoch: u64,
```
Initialize both in `starting`, `with_engine_for_test` (`generation: None`, `reconnect_epoch: 0`).

In `on_attached` (after `self.current_tid = Some(resource.id.clone());`), rebuild the guard for the fresh identity and clear any stale gap marker (Grok Imp 5 — `on_attached` is the single fresh-engine attach path shared by open/adopt/wake; a fresh engine never carries a prior reconnect's gap):
```rust
let key = match &self.target {
    TerminalTarget::OpenOrCreate { key, .. } => Some(key.clone()),
    TerminalTarget::Existing { .. } => None,
};
self.generation = Some(generation::GenerationGuard::new(resource.id.clone(), key));
self.presentation.output_gap = false; // fresh engine — no carried-over gap
self.adopt_in_flight = false;         // settle any in-flight adopt (Task 4)
```
(The reconnect path sets `output_gap = true` in `on_reconnect_success`, which is separate and unaffected. The Reattach-fresh fallback in Task 5 re-sets it to `true` *after* `on_attached` returns.)

- [ ] **Step 2: Write failing tests for correlation transitions**

Add to the `lib.rs` test module (offline; these transitions never touch REST). Model on the existing `with_engine_for_test` tests. Note `with_engine_for_test` builds an `OpenOrCreate` tab keyed `main:k` with `current_tid = None` — set `current_tid` + guard explicitly in the test, or add a small test helper `set_attached_identity_for_test(tid, key)`.

```rust
#[gpui::test]
fn open_or_create_delete_enters_replacement_waiting(cx: &mut TestAppContext) {
    let (engine, tab) = live_tab_for_test(cx, /*open_or_create*/ true, "t1", "main", "k");
    tab.update(cx, |tab, cx| {
        tab.on_host_event(TerminalHostEvent::ResourceDeleted { terminal_id: TerminalId::new("t1") }, cx);
        assert_eq!(tab.lifecycle, Lifecycle::ReplacementWaiting);
        assert!(tab.runtime.is_none(), "dead engine must be released on reset");
    });
    let _ = engine;
}

#[gpui::test]
fn existing_delete_detaches_terminal_gone(cx: &mut TestAppContext) {
    let (_e, tab) = live_tab_for_test(cx, /*open_or_create*/ false, "t1", "main", "k");
    tab.update(cx, |tab, cx| {
        tab.on_host_event(TerminalHostEvent::ResourceDeleted { terminal_id: TerminalId::new("t1") }, cx);
        assert_eq!(tab.lifecycle, Lifecycle::Detached);
        assert_eq!(tab.presentation.detached_detail, Some(DetachedDetail::TerminalGone));
    });
}

#[gpui::test]
fn create_echo_without_delete_keeps_live(cx: &mut TestAppContext) {
    let (_e, tab) = live_tab_for_test(cx, true, "t1", "main", "k");
    tab.update(cx, |tab, cx| {
        tab.on_host_event(TerminalHostEvent::ResourceCreated {
            session_id: SessionId::new("s"), terminal_id: TerminalId::new("t1"),
            terminal_name: "main".into(), session_key: "k".into(),
        }, cx);
        assert_eq!(tab.lifecycle, Lifecycle::Live);
    });
}
```

`live_tab_for_test` is a small helper: builds via `with_engine_for_test`, overrides `target` (Existing vs OpenOrCreate), sets `current_tid` + `generation`. Add it to the test module.

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p lens-terminal replacement_waiting 2>&1 | tail -20`
Expected: FAIL — `on_host_event` still no-ops the new variants; `ReplacementWaiting` never set.

- [ ] **Step 4: Implement the `on_host_event` arms + `enter_replacement_waiting`**

Replace the `TerminalHostEvent::Sleep | TerminalHostEvent::Wake => {}` line: keep Sleep/Wake as a `todo`-free stub for now (Task 5), and add:

```rust
TerminalHostEvent::ResourceCreated { session_id, terminal_id, terminal_name, session_key } => {
    self.on_resource_signal(
        generation::ResourceSignal::Created { session_id, terminal_id, terminal_name, session_key },
        cx,
    );
}
TerminalHostEvent::ResourceDeleted { terminal_id } => {
    self.on_resource_signal(generation::ResourceSignal::Deleted { terminal_id }, cx);
}
```

Add the dispatcher:
```rust
// NB: compute the verdict in a scoped block so the `&mut self.generation` borrow
// is released BEFORE the self-mutating transition calls (holding it across
// `self.enter_replacement_waiting(...)` would not compile).
fn on_resource_signal(&mut self, signal: generation::ResourceSignal, cx: &mut Context<Self>) {
    let verdict = {
        let Some(guard) = self.generation.as_mut() else { return; };
        match self.lifecycle {
            // Live-ish guarded states: act on the verdict.
            Lifecycle::Live | Lifecycle::Reconnecting | Lifecycle::ReplacementWaiting => {
                Some(guard.on_signal(&signal))
            }
            // Sleeping: feed the guard so `is_dirty()` records a delete arriving during
            // Sleep (Wake detaches on it — Task 5), but drive no transition here.
            Lifecycle::Sleeping => {
                let _ = guard.on_signal(&signal);
                None
            }
            // Starting/Ended/Detached: not guarded; ignore.
            _ => None,
        }
    };
    match verdict {
        Some(generation::GenerationVerdict::AwaitReplacement) => self.enter_replacement_waiting(cx),
        Some(generation::GenerationVerdict::AdoptSuccessor { session_id, terminal_id }) => {
            self.adopt_successor(session_id, terminal_id, cx)
        }
        Some(generation::GenerationVerdict::Detach(detail)) => self.on_detach(detail, cx),
        Some(generation::GenerationVerdict::Unchanged) | None => {}
    }
}

fn enter_replacement_waiting(&mut self, cx: &mut Context<Self>) {
    // Positive reset: the old terminal is gone. Cancel any in-flight reconnect,
    // freeze the final frame (render state already holds it), and release the
    // dead engine + scrollback off-foreground. A fresh engine is built on adopt.
    self.reconnect_epoch = self.reconnect_epoch.wrapping_add(1);
    self.clear_input_composition_state();
    self.teardown_runtime_full(cx); // full engine reclaim (confirmed worker exit)
    self.input_enabled = false;
    self.presentation.output_gap = false; // frozen ReplacementWaiting frame carries no stale gap marker
    self.lifecycle = Lifecycle::ReplacementWaiting;
    self.presentation.lifecycle = Lifecycle::ReplacementWaiting;
    self.arm_replacement_timeout(cx); // Task 4
    cx.emit(TerminalEvent::PresentationChanged);
    cx.notify();
}
```

Add stubs `fn adopt_successor(&mut self, _sid: SessionId, _tid: TerminalId, _cx: &mut Context<Self>) {}` and `fn arm_replacement_timeout(&mut self, _cx: &mut Context<Self>) {}` — filled in Task 4. (The `AdoptSuccessor` verdict is only reachable after a delete → `ReplacementWaiting`, so the stub never fires in this task's tests.)

- [ ] **Step 5: Add the reconnect-cancellation guard**

The existing comment at `on_reconnect_success` ("no events arrive during the reconnect window") is now false — a `ResourceDeleted` can fire mid-reconnect. Capture the epoch when scheduling and re-check on **every** exit from the reconnect loop — success **and** `Fatal` **and** `RetriesExhausted` (Grok Critical 1: a stale in-flight reconnect that then 404s or exhausts retries would otherwise call `on_detach` and clobber `ReplacementWaiting`/`Sleeping`/adopted-`Live`). In `schedule_reconnect`, snapshot `let epoch = self.reconnect_epoch;` and move it into the spawn. Guard all three arms:

```rust
// success:
Ok((resource, attach)) => {
    let _ = weak.update(cx, |tab, cx| {
        if tab.reconnect_epoch != epoch {
            tab.close_attach_off_foreground(attach, cx); // superseded — never resurrect
            return;
        }
        tab.on_reconnect_success(resource, attach, cx);
    });
    break;
}
// Fatal:
Err(ReconnectOutcome::Fatal(detail)) => {
    let _ = weak.update(cx, |tab, cx| {
        if tab.reconnect_epoch == epoch { tab.on_detach(detail, cx); } // else superseded — no-op
    });
    break;
}
```

Also guard the `RetriesExhausted` branch (the `Ok(None)` from `retry.next_delay`): only `on_detach(RetriesExhausted)` when `tab.reconnect_epoch == epoch`; otherwise just `break`. And short-circuit the loop top: if a `weak.update`-read shows `reconnect_epoch != epoch`, `break` before sleeping/attempting.

Bump `reconnect_epoch` in `on_detach`, `enter_replacement_waiting` (done above), Sleep/Wake/adopt (Tasks 4–5). Extract the "close attach off the fg" idiom (used twice in `on_reconnect_success`) into `fn close_attach_off_foreground(&self, attach: AttachHandle, cx)`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p lens-terminal 'replacement_waiting|detaches_terminal_gone|create_echo' 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 7: Regression — reconnect cancellation**

Add a test: drive a tab into `Reconnecting` (feed a `BridgeEvent::Closed(CloseCause::Network)` via `apply_bridge_event`), then feed `ResourceDeleted{ours}` → assert `ReplacementWaiting` and that a subsequent stale `on_reconnect_success` call (simulated) does **not** flip back to `Live`. Run it. Expected: PASS.

- [ ] **Step 7b: Acceptance — resize during reconnect (Grok Imp 7 / completion matrix)**

The matrix assigns "resize end-to-end during-reconnect" to Slice 4. Verify the newest-size-before-input ordering survives the cancellation guard: drive a tab into `Reconnecting`, apply a resize (the tab's resize path forwards to the retained engine's geometry), then simulate a successful reconnect and assert the engine's geometry is the newest size and `apply_newest_size_before_input` ran before `input_enabled` flipped true. If Slice 1d already covers the base ordering, this test just re-confirms it under Slice 4's epoch guard (document which). Run. Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/lens-terminal/src/lib.rs
git commit -m "feat(terminal-4): correlate resource signals → ReplacementWaiting/Detached + reconnect cancellation"
```

---

## Task 4: Successor adoption + bounded `ReplacementWaiting` timeout

**Files:**
- Modify: `crates/lens-terminal/src/lib.rs` — fill `adopt_successor` + `arm_replacement_timeout`; add `REPLACEMENT_WAIT`
- Test: in-crate tests (offline application-path + timeout via the reconnect-style injected delay)

**Interfaces:**
- Consumes: `discover_and_attach` (Task 2 signature), `on_attached` (existing), `enter_replacement_waiting`/`reconnect_epoch` (Task 3).
- Produces: adoption reuses `on_attached` for the fresh engine so Task 5/6 inherit a single attach-application path.

**Adoption design:** the successor's `(session_id, terminal_id)` come straight from the `AdoptSuccessor` verdict. Adopt = off-thread `discover_and_attach(client, TerminalTarget::Existing { session_id, terminal_id }, options)` (GET-before-attach; deterministic id means the GET lands on the fresh instance) → on success `on_attached(parts)` (fresh engine, resets the guard, `Live`) with `output_gap` left **false** (adoption is a clean fresh terminal, not a gap). Guard the application with `reconnect_epoch`.

- [ ] **Step 1: Write the failing timeout test**

```rust
#[gpui::test]
fn replacement_waiting_times_out_to_detached(cx: &mut TestAppContext) {
    let (_e, tab) = live_tab_for_test(cx, true, "t1", "main", "k");
    tab.update(cx, |tab, cx| {
        tab.on_host_event(TerminalHostEvent::ResourceDeleted { terminal_id: TerminalId::new("t1") }, cx);
        assert_eq!(tab.lifecycle, Lifecycle::ReplacementWaiting);
    });
    // Drive the injected clock past REPLACEMENT_WAIT (mirror the retry-window test's
    // injected `now`); assert Detached(ReplacementTimedOut).
    advance_replacement_timeout(cx, &tab);
    tab.update(cx, |tab, _| {
        assert_eq!(tab.lifecycle, Lifecycle::Detached);
        assert_eq!(tab.presentation.detached_detail, Some(DetachedDetail::ReplacementTimedOut));
    });
}
```

The timeout mechanism mirrors `schedule_reconnect`: an off-thread `background_executor().spawn(async { sleep(REPLACEMENT_WAIT) })` then `weak.update` that, **iff still `ReplacementWaiting` and `reconnect_epoch` unchanged**, calls `on_detach(DetachedDetail::ReplacementTimedOut, cx)`. For determinism, gate the sleep behind the same injected-delay seam the reconnect tests use (or expose a `#[cfg(test)] fn fire_replacement_timeout_now`). Prefer a test hook `fire_replacement_timeout_now(&mut self, cx)` that runs the timeout body directly — avoids real sleeps.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lens-terminal replacement_waiting_times_out 2>&1 | tail -15`
Expected: FAIL — `arm_replacement_timeout` is a stub.

- [ ] **Step 3: Implement `arm_replacement_timeout`**

```rust
/// Bounded wait for the exact-key successor. Unbounded waiting would hang the
/// tab forever when the new agent never relaunches our key (the delete fires
/// regardless of whether a successor ever appears — memory
/// `terminal-resource-event-granularity`).
const REPLACEMENT_WAIT: Duration = Duration::from_secs(30);

fn arm_replacement_timeout(&mut self, cx: &mut Context<Self>) {
    let epoch = self.reconnect_epoch;
    cx.spawn(async move |weak, cx| {
        cx.background_executor().spawn(async move { std::thread::sleep(REPLACEMENT_WAIT) }).await;
        let _ = weak.update(cx, |tab, cx| {
            if tab.reconnect_epoch == epoch && tab.lifecycle == Lifecycle::ReplacementWaiting {
                tab.on_detach(DetachedDetail::ReplacementTimedOut, cx);
            }
        });
    })
    .detach();
}
```

Add the `#[cfg(test)] fn fire_replacement_timeout_now` that runs the inner body with the current epoch, for the deterministic test.

- [ ] **Step 4: Implement `adopt_successor`**

Add a `adopt_in_flight: bool` field (init `false` in `starting`/`with_engine_for_test`) so a duplicate/redelivered matching `created` while an adopt is already running doesn't spawn a second `discover_and_attach` (Grok Minor 12 — epoch already prevents a *wrong* landing, this prevents wasted work). Set it `true` at the top of `adopt_successor`, and clear it in every settle path (`on_attached`, the `Err`/superseded arms). Guard the entry: `if self.adopt_in_flight { return; }`.

```rust
fn adopt_successor(&mut self, session_id: SessionId, terminal_id: TerminalId, cx: &mut Context<Self>) {
    if self.adopt_in_flight { return; }
    self.adopt_in_flight = true;
    // Cancel the replacement timeout; a successor is in hand.
    self.reconnect_epoch = self.reconnect_epoch.wrapping_add(1);
    let epoch = self.reconnect_epoch;
    let client = Arc::clone(&self.client);
    let options = self.options.clone();
    let target = TerminalTarget::Existing { session_id, terminal_id };
    cx.spawn(async move |weak, cx| {
        let outcome = cx
            .background_executor()
            .spawn(async move { discover_and_attach(client, target, options) })
            .await;
        let _ = weak.update(cx, |tab, cx| {
            if tab.reconnect_epoch != epoch {
                // Superseded (another reset, sleep, or detach) — drop the result.
                tab.adopt_in_flight = false;
                if let Ok(parts) = outcome { tab.close_parts_off_foreground(parts, cx); }
                return;
            }
            match outcome {
                Ok(parts) => tab.on_attached(parts, cx), // fresh engine, guard rebuilt, Live, clears adopt_in_flight
                Err(detail) => tab.on_detach(detail, cx), // must clear adopt_in_flight — see below
            }
        });
    })
    .detach();
}
```

Add `close_parts_off_foreground` (tears down an unused `AttachedParts.runtime` off-thread via `teardown_blocking`). Also add `self.adopt_in_flight = false;` to `on_detach` (it is the terminal settle point for the `Err` arm and any other failure), so the flag never latches.

- [ ] **Step 5: Run the timeout test + build**

Run: `cargo test -p lens-terminal replacement_waiting_times_out 2>&1 | tail -15` → PASS.
Run: `cargo build -p lens-terminal --all-targets 2>&1 | tail -10` → clean.

- [ ] **Step 6: Adoption application test (offline)**

Test the application step without live REST: construct an `AttachedParts` from a fresh `with_engine_for_test`-style engine (reuse the helper that builds `AttachedParts` for `on_attached` tests if one exists; otherwise assert the epoch-supersession branch closes parts). At minimum, assert that a stale-epoch adoption result does **not** flip to `Live`. Run. Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/lens-terminal/src/lib.rs
git commit -m "feat(terminal-4): adopt exact-key successor (fresh engine) + bounded ReplacementWaiting timeout"
```

---

## Task 5: Sleep / Wake / Reattach host actions

**Files:**
- Modify: `crates/lens-terminal/src/lib.rs` — `on_host_event` Sleep/Wake arms + a new `TerminalHostEvent::Reattach` variant + handler
- Test: in-crate tests via `live_tab_for_test`

**Interfaces:**
- Consumes: `teardown_runtime_full`, `on_detach`, `discover_and_attach`, `on_attached`, `GenerationGuard::is_dirty`, `reconnect_epoch`.
- Produces: full Sleep/Wake/Reattach behavior; adds `TerminalHostEvent::Reattach`.

**Behavior (spec §Deliberate Sleep/wake, §Lifecycle):**
- **Sleep:** close WS + release engine + full scrollback (`teardown_runtime_full`), retain the final frame (render state already holds it), `Lifecycle::Sleeping`, `input_enabled=false`, bump `reconnect_epoch`, **no** `output_gap` marker. Record nothing extra — the retained `GenerationGuard.is_dirty()` continues to accumulate signals arriving during Sleep.
- **Wake:** if `generation.is_dirty()` (a delete arrived during Sleep) → `Detach(IdentityChanged)`. Else off-thread `discover_and_attach(Existing{current_session, current_tid})`; `TerminalGone`/`Unauthorized` → `Detach`; success → `on_attached` (fresh engine, `Live`), **no** `output_gap`. Wake never adopts a successor (stricter than reconnect — spec).
- **Reattach** (4405/`ClientDetached` explicit action): only meaningful from `Detached` with `reattach_available`. The engine was retained (`teardown_transport_off_foreground` keeps it). Re-attach the transport to the retained engine like `on_reconnect_success` but user-triggered; set `output_gap=true` (output during detach may be missing). If the engine is gone, fall back to a fresh `discover_and_attach`.

- [ ] **Step 1: Add the `Reattach` variant**

In `TerminalHostEvent`:
```rust
    /// Explicit user reattach after a `ClientDetached` (4405) detach (the only
    /// detach that sets `reattach_available`). Reuses the retained engine when
    /// present; else a fresh attach.
    Reattach,
```

- [ ] **Step 2: Write failing Sleep + Wake tests**

```rust
#[gpui::test]
fn sleep_releases_engine_and_freezes(cx: &mut TestAppContext) {
    let (engine, tab) = live_tab_for_test(cx, true, "t1", "main", "k");
    let weak_engine = Arc::downgrade(&engine);
    drop(engine); // the tab's runtime holds the only other Arc
    tab.update(cx, |tab, cx| {
        tab.on_host_event(TerminalHostEvent::Sleep, cx);
        assert_eq!(tab.lifecycle, Lifecycle::Sleeping);
        assert!(!tab.presentation.output_gap, "Sleep adds no gap marker");
        assert!(tab.runtime.is_none());
    });
    // teardown is off-foreground; drain then assert the engine Arc was reclaimed.
    cx.run_until_parked();
    assert!(weak_engine.upgrade().is_none(), "Sleep must release the engine + scrollback");
}

#[gpui::test]
fn wake_after_delete_during_sleep_detaches(cx: &mut TestAppContext) {
    let (_e, tab) = live_tab_for_test(cx, true, "t1", "main", "k");
    tab.update(cx, |tab, cx| {
        tab.on_host_event(TerminalHostEvent::Sleep, cx);
        // Delete arrives while sleeping — marks the guard dirty.
        tab.on_host_event(TerminalHostEvent::ResourceDeleted { terminal_id: TerminalId::new("t1") }, cx);
        tab.on_host_event(TerminalHostEvent::Wake, cx);
        assert_eq!(tab.lifecycle, Lifecycle::Detached);
        assert_eq!(tab.presentation.detached_detail, Some(DetachedDetail::IdentityChanged));
    });
}
```

Note: `on_resource_signal` must accept signals while `Sleeping` (mark dirty) **without** driving a transition — update its guard so `Sleeping` only records `is_dirty` and returns (do not enter `ReplacementWaiting` from Sleep). Adjust the early-return in `on_resource_signal` to: if `Sleeping`, feed the guard (to update `saw_delete`) but ignore the verdict.

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p lens-terminal 'sleep_releases|wake_after_delete' 2>&1 | tail -20`
Expected: FAIL — Sleep/Wake still stubbed.

- [ ] **Step 4: Implement Sleep/Wake/Reattach**

```rust
TerminalHostEvent::Sleep => self.on_sleep(cx),
TerminalHostEvent::Wake => self.on_wake(cx),
TerminalHostEvent::Reattach => self.on_reattach(cx),
```

```rust
fn on_sleep(&mut self, cx: &mut Context<Self>) {
    // Only sleep from a live-ish state. Sleeping from Detached(ClientDetached) would
    // `teardown_runtime_full` the engine that the 4405 reattach path relies on (Grok Imp 8).
    if !matches!(self.lifecycle, Lifecycle::Live | Lifecycle::Reconnecting | Lifecycle::ReplacementWaiting) {
        return;
    }
    self.reconnect_epoch = self.reconnect_epoch.wrapping_add(1);
    self.clear_input_composition_state();
    self.teardown_runtime_full(cx); // release engine + scrollback (confirmed worker exit)
    self.input_enabled = false;
    self.presentation.output_gap = false; // deliberate Sleep — frozen frame carries no gap marker
    self.lifecycle = Lifecycle::Sleeping;
    self.presentation.lifecycle = Lifecycle::Sleeping;
    cx.emit(TerminalEvent::PresentationChanged);
    cx.notify();
}

fn on_wake(&mut self, cx: &mut Context<Self>) {
    if self.lifecycle != Lifecycle::Sleeping { return; }
    if self.generation.as_ref().map(|g| g.is_dirty()).unwrap_or(false) {
        self.on_detach(DetachedDetail::IdentityChanged, cx);
        return;
    }
    let (Some(session), Some(tid)) = (self.current_session.clone(), self.current_tid.clone()) else {
        self.on_detach(DetachedDetail::TerminalGone, cx);
        return;
    };
    // Fresh attach against the same identity (Sleep released the engine).
    self.reconnect_epoch = self.reconnect_epoch.wrapping_add(1);
    let epoch = self.reconnect_epoch;
    let client = Arc::clone(&self.client);
    let options = self.options.clone();
    let target = TerminalTarget::Existing { session_id: session, terminal_id: tid };
    cx.spawn(async move |weak, cx| {
        let outcome = cx.background_executor()
            .spawn(async move { discover_and_attach(client, target, options) }).await;
        let _ = weak.update(cx, |tab, cx| {
            // Re-check at APPLY time, not just before spawning (Grok Critical 2): a
            // `ResourceDeleted` arriving while the attach was in flight marks the guard
            // dirty (Task 3 Sleeping arm) without bumping the epoch, so the epoch check
            // alone would let Wake land Live on a generation that did not survive —
            // violating spec §Deliberate Sleep/wake ("reattaches only if the same
            // observed generation survived; else Detached").
            let dirty = tab.generation.as_ref().map(|g| g.is_dirty()).unwrap_or(false);
            if tab.reconnect_epoch != epoch || tab.lifecycle != Lifecycle::Sleeping || dirty {
                if let Ok(parts) = outcome { tab.close_parts_off_foreground(parts, cx); }
                if dirty && tab.lifecycle == Lifecycle::Sleeping {
                    tab.on_detach(DetachedDetail::IdentityChanged, cx);
                }
                return;
            }
            match outcome {
                Ok(parts) => tab.on_attached(parts, cx), // no gap marker on deliberate wake
                Err(detail) => tab.on_detach(detail, cx),
            }
        });
    })
    .detach();
}

fn on_reattach(&mut self, cx: &mut Context<Self>) {
    // Reattach is the explicit action for a `ClientDetached` (4405) detach ONLY —
    // that path retains the engine (`teardown_transport_off_foreground`) and sets
    // `reattach_available`. Other detaches (`RetriesExhausted`, `TerminalGone`, …)
    // do a full teardown and do NOT set `reattach_available`, so they are gated out
    // here (Grok Imp 10 — the earlier "exhausted-retry" wording was wrong).
    if self.lifecycle != Lifecycle::Detached || !self.presentation.reattach_available { return; }
    // Retained engine present (ClientDetached path) → re-attach transport only.
    if self.runtime.as_ref().map(|r| r.engine_ref().is_some()).unwrap_or(false) {
        self.lifecycle = Lifecycle::Reconnecting;
        self.presentation.lifecycle = Lifecycle::Reconnecting;
        self.presentation.detached_detail = None;
        self.policy.retry.reset();
        cx.emit(TerminalEvent::PresentationChanged);
        cx.notify();
        self.schedule_reconnect(Duration::ZERO, cx); // sets output_gap on success
        return;
    }
    // Engine gone (rare — ClientDetached retains it, but a later drop is possible)
    // → fresh attach against the same identity, WITH a gap marker (output during the
    // detach may be missing). Distinct from Wake (no gap) — do NOT share on_wake's
    // Sleeping-specific dirty re-check here.
    let (Some(session), Some(tid)) = (self.current_session.clone(), self.current_tid.clone()) else {
        return; // nothing to reattach to
    };
    self.reconnect_epoch = self.reconnect_epoch.wrapping_add(1);
    let epoch = self.reconnect_epoch;
    self.lifecycle = Lifecycle::Reconnecting;
    self.presentation.lifecycle = Lifecycle::Reconnecting;
    self.presentation.detached_detail = None;
    cx.emit(TerminalEvent::PresentationChanged);
    cx.notify();
    let client = Arc::clone(&self.client);
    let options = self.options.clone();
    let target = TerminalTarget::Existing { session_id: session, terminal_id: tid };
    cx.spawn(async move |weak, cx| {
        let outcome = cx.background_executor()
            .spawn(async move { discover_and_attach(client, target, options) }).await;
        let _ = weak.update(cx, |tab, cx| {
            if tab.reconnect_epoch != epoch {
                if let Ok(parts) = outcome { tab.close_parts_off_foreground(parts, cx); }
                return;
            }
            match outcome {
                Ok(parts) => {
                    tab.on_attached(parts, cx);          // forces output_gap = false…
                    tab.presentation.output_gap = true;  // …but reattach DID miss output.
                    cx.emit(TerminalEvent::PresentationChanged);
                }
                Err(detail) => tab.on_detach(detail, cx),
            }
        });
    })
    .detach();
}
```

- [ ] **Step 5: Run Sleep/Wake tests**

Run: `cargo test -p lens-terminal 'sleep_releases|wake_after_delete' 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Reattach test**

Drive a tab to `Detached(ClientDetached)` via `apply_bridge_event(BridgeEvent::Closed(CloseCause::TerminalDetached))` (retains engine, `reattach_available=true`), then `on_host_event(Reattach)` → assert `Reconnecting` and (after `run_until_parked` against the stub, which will fail the attach) that it does not panic and returns to a retry/detached path. Assert the pre-reattach transition to `Reconnecting` deterministically. Run. Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/lens-terminal/src/lib.rs
git commit -m "feat(terminal-4): Sleep/Wake teardown-recreate + explicit Reattach (4405) host actions"
```

---

## Task 6: Demo driving + live rider + inspect + gate

**Files:**
- Modify: `crates/lens-terminal-demo/src/main.rs` — keybindings feeding `Sleep`/`Wake`/`Reattach` + a scripted reset (`ResourceDeleted` then `ResourceCreated` for the same key) to exercise adoption in the demo
- Modify: `crates/lens-terminal/tests/terminal_live.rs` — `P7` Sleep→Wake, `P8` Reattach riders (opt-in env-gated, like existing P-cases)
- Modify: inspect surface if generation state needs asserting in-app
- Test: `xtask gate`

**Interfaces:**
- Consumes: every `TerminalHostEvent` from Tasks 1–5.

- [ ] **Step 1: Demo host-action bindings**

In the demo, bind keys (document them in the demo's on-screen help / README): e.g. `Cmd+S` → `Sleep`, `Cmd+Shift+S` → `Wake`, `Cmd+R` → `Reattach`, and `Cmd+Shift+R` → scripted reset (emit `ResourceDeleted { current_tid }` then, after a short delay, `ResourceCreated { same key }`) so a human can watch `Live → ReplacementWaiting → Live` (adopt) and the timeout path (`Wait without the create`). Route each through `tab.update(cx, |t, cx| t.on_host_event(ev, cx))`. Keep it host-agnostic — the demo is just a host that invokes.

- [ ] **Step 2: Build + manually smoke the demo**

Run: `cargo build -p lens-terminal-demo 2>&1 | tail -10` → clean.
Manually (via `!`, real window): launch the demo against a live shell, exercise Sleep (screen freezes, `Sleeping`), Wake (reattaches), reset (adopt), reset-without-successor (times out → `Detached`). Record the observed transitions in the handoff. (Real-window bins frame-starve headless — memory `terminal-realwindow-harness-pitfalls`; run foreground.)

- [ ] **Step 3: Live riders (opt-in)**

Add `P7`/`P8` to `terminal_live.rs` gated behind an env flag (mirror `LENS_LIVE_MOUSE_REPORT`): `P7` attaches a real terminal, `Sleep`, asserts engine released + `Sleeping`, `Wake`, asserts re-attached + `Live` + fresh redraw; `P8` forces a detach, `Reattach`, asserts `Live`. Document that generation-guard **adoption** has no cheap live trigger (needs a real agent switch / `reset-state`), so it stays deterministic-test + demo-covered — an honest limitation for the handoff.

- [ ] **Step 4: Run the live riders (foreground)**

Run (only if a live omnigent is available; otherwise document skip): `LENS_LIVE_TERMINAL=1 cargo test -p lens-terminal --features live-tests --test terminal_live 2>&1 | tail -20`
Expected: `P7`/`P8` PASS or documented-skip.

- [ ] **Step 5: Full gate**

Run: `cargo run -p xtask -- gate 2>&1 | tail -30`
Expected: `gate: all checks passed` — all crate tests, both clippy configs (`-D warnings`), both real-window harnesses, benches compile, no contract drift. If the pre-existing `render_realwindow`/`engine::handle` load flakes trip, re-run the affected component isolated (memories `worker-stall-gate-busy-spin-flake`, Slice-3 handoff) and record.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-terminal-demo crates/lens-terminal/tests/terminal_live.rs
git commit -m "feat(terminal-4): demo drives Sleep/Wake/Reattach/reset + opt-in live riders; gate green"
```

---

## Self-Review

**1. Spec coverage (Slice 4 bullet + completion matrix rows):**
- Generation guard on the **live stream + wake** (duplicate `resource.created` / delete correlation) → Tasks 1, 3, 5, with the ratified echo-avoidance deviation.
- Generation guard on the **reconnect path** ("full" guard, completion-matrix row) → **DEFERRED, not covered.** Grok Critical 3 / Imp 6: `preflight_reconnect` cannot detect a new generation without an immutable token (spec §Open contract gaps: "Never invent a client-side generation ID"). A *missed* delete + retryable close silently retains the old engine. The reliable observed-signal cases are covered; this residual is the accepted upstream no-token race. **Handoff must record it as deferred-with-SPEC-GAPS-citation, not delivered.** Closing it later needs an upstream terminal-generation token (or lens-client resource-event-history consultation, which is out of the Slice-3+4 "pure lens-terminal + demo" merge-safety scope).
- Sleep/wake teardown/recreate (confirmed-exit) → Task 5 (reuses `teardown_runtime_full`).
- `ReplacementWaiting` exact-key successor adoption (fresh engine) → Tasks 3–4.
- `4405`/`1008` close-code lifecycle refinements → Task 5 (`Reattach`); `1008` downgrade already in `policy.rs` (no new work — noted).
- **Resize end-to-end during-reconnect** (completion-matrix row assigned to Slice 4) → Task 3 Step 7b acceptance test (Grok Imp 7): a resize while `Reconnecting` updates the retained engine's geometry, and `on_reconnect_success`'s existing `apply_newest_size_before_input` sends the newest `{cols,rows}` before input re-enables. Verify it holds under the new cancellation guard.
- `Ended` inert → untouched by construction (no task enters it); called out in Global Constraints.
- Folded Slice-3 Minor (worker `.expect()` → policy) → Task 2.
- Host-agnostic (host invokes) → all mechanisms on `on_host_event`; demo is the invoking host (Task 6).
- Inspect + benches per-slice → Task 6 (inspect if needed); no new hot path, so no new bench required — **confirm** during execution that generation state adds zero hot-path cost.
- Pure `lens-terminal` + demo → no `lens-ui`/`lens-core` touched; merge-safe.

**2. Placeholder scan:** No "TBD"/"handle edge cases"/"similar to Task N". Test code is concrete; gpui integration steps name exact methods/fields/line refs. The one intentional stub (`adopt_successor`/`arm_replacement_timeout` in Task 3) is explicitly filled in Task 4.

**3. Type consistency:** `GenerationGuard::new(TerminalId, Option<TerminalKey>)`, `on_signal(&ResourceSignal) -> GenerationVerdict`, `is_dirty()` used identically across Tasks 1/3/5. `TerminalHostEvent::{ResourceCreated,ResourceDeleted,Reattach}` and `DetachedDetail::{IdentityChanged,ReplacementTimedOut,EngineSpawnFailed}` defined in Task 1/5, consumed consistently. `EngineHandle::spawn -> Result<Self, EngineSpawnError>` (Task 2) flows into `discover_and_attach` reused by adoption/wake. `reconnect_epoch` cancellation contract is uniform across reconnect/adopt/wake/sleep.

**4. Grok-4.5 review (2026-07-22) — disposition:** C1 (guard all reconnect exit arms) → Task 3 Step 5; C2 (Wake dirty re-check at apply) → Task 5; C3 (reconnect ≠ Identity recovery) → design-note rewrite + Self-Review deferral; Imp 4 (commit-green no-op arms) → Task 1 Step 2b; Imp 5 (`output_gap`) → `on_attached`/`enter_replacement_waiting`/`on_sleep`/reattach-fresh; Imp 6 (reconnect guard deferred) → Self-Review §1; Imp 7 (resize-during-reconnect) → Task 3 Step 7b; Imp 8 (Sleep accept-states) → Task 5 `on_sleep`; Imp 9 (forwarder orphan) → Task 2; Imp 10 (Reattach comment) → Task 5; Minors 12/13, Nit 15 → Tasks 2/4. All folded. Review artifact: `docs/plans/2026-07-22-terminal-slice-4-review-grok.md`.

**Open items to confirm during execution (not blockers):**
- Whether `live_tab_for_test` / an `AttachedParts` test builder already exists to reuse — grep before adding.
- Exact injected-clock seam for the replacement timeout (prefer a `#[cfg(test)]` direct-fire hook over real sleeps).
- `output_gap` policy is now fixed: `on_attached` always sets it `false` (fresh engine); reconnect/reattach set it `true` post-hoc. Keep that invariant — do **not** re-add a `gap` param to `on_attached`.
- Confirm `InputForwarder` exposes a synchronous sever+join for the Task-2 orphan fix (grep `forwarder.rs`).
