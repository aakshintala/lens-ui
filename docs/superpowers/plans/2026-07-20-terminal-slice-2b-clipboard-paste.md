# Terminal Slice 2b — OSC-52 Clipboard-Write Policy + Cmd+V Paste Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Register the deferred `on_clipboard_write` effect with a decoded-byte cap applied *before* cloning, route program clipboard writes through an async foreground permission policy, and add `Cmd+V` paste (bracketed engine-side, read-only-gated, multiline-warned, capped) — all behind an injectable session-scoped `ClipboardPolicy` seam.

**Architecture:** 2b lands **serially, after 2d**, on `terminal-ws` — it edits 2d's committed code (no worktree, no merge). The OSC-52 callback runs on the engine thread; **its result is ignored by OSC-52 (never backpressure)**, so it only caps + forwards an owned `EnginePresentationEvent::ClipboardWrite` (variant already declared by 2d). The **policy decision happens on the foreground, asynchronously** — the drain emits a typed `ClipboardWriteRequest`, the host answers via the existing `HostRequestResponse` inbound seam, and on Allow the foreground performs the system-clipboard write. `Cmd+V` is intercepted in `handle_key_down` (Cmd currently routes to the key encoder — it must not), gated read-only, warned on multiline, capped, then sent as a new never-drop / epoch-revocable `EngineCommand::Paste` that the worker bracketed-encodes against the terminal's live mode 2004.

**Tech Stack:** `libghostty-vt` (`Terminal::on_clipboard_write`, `ClipboardWrite`/`ClipboardContent`, `terminal::Mode::BRACKETED_PASTE`, `paste::encode`), gpui (`ClipboardItem`, `cx.read_from_clipboard`/`write_to_clipboard`, `EventEmitter`), `crossbeam-channel`, `base64` (dev-only, OSC-52 test vectors).

## Serial position (2b is THIRD — lands after 2d; selection/copy re-cut to 2c)

Prerequisite: **2a + 2d are committed on `terminal-ws`** (`5e6f28b`+). 2b edits their real code directly. Per the **2026-07-20 spec amendment** (`docs/specs/2026-07-17-terminal-slice-2-interaction-design.md`), local selection + `Cmd+C` copy **moved from 2b into 2c** (they share 2c's mouse-capture stack + XTSHIFTESCAPE arbitration). **This slice does NOT build selection, copy, mouse capture, hit-testing, or pixel→cell.** 2b = **OSC-52 output-clipboard write policy + `Cmd+V` paste only** (both keyboard/output-only, zero mouse dependency).

| Surface 2d already declared (2b fills) | Where |
| --- | --- |
| `EnginePresentationEvent::ClipboardWrite { location, contents }` | `engine/presentation.rs:26` |
| `ClipboardLocation { Standard, Selection, Primary }` / `ClipboardMimePart { mime, data }` | `engine/presentation.rs:8,15` |
| `HostRequestId` / `HostRequestDecision { Allow, Deny }` / `TerminalHostEvent::HostRequestResponse` | `lib.rs:321,325,344` |
| `next_host_request_id` + `drain_presentation_events` + `on_host_event` (no-op) | `lib.rs:405,1046,510` |
| `collect_presentation_drain` (ignores `ClipboardWrite`) + `PresentationDrainResult` | `engine/presentation.rs:76,58` |
| Bare title closure holding the sole `presentation_tx` clone + **the 2b re-thread note** | `engine/vt.rs:100-103` |

## Global Constraints

- **Per-task `lens-terminal` clippy MUST include `--features test-util` (and `live-tests` when touching live hooks):** `cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings`. Workspace gate: `cargo clippy --workspace --all-targets -- -D warnings`. **Never pipe the gate / clippy / test output through `tail`.** `cargo test` takes ONE positional filter.
- **Effect callbacks never block:** inside `on_clipboard_write`, map location + **cap-check on summed decoded bytes → copy into owned → non-blocking `try_send` → wake → return `Ok(())`**; never `recv`, never await host policy, never take a GPUI lock. **OSC 52 ignores the callback result** (`vendor/.../terminal.rs:1345-1348`) — do **not** claim `Busy`/`Denied` backpressure; the callback always returns `Ok(())` after forwarding (or after an over-cap drop).
- **cap-BEFORE-clone (security-relevant ordering):** `MAX_OSC52_CLIPBOARD_BYTES = 1 << 20` (1 MiB). Collect the callback's contents as **borrowed `(&str, &str)` refs** (no data copy), sum `data.len()` across MIME parts, and if the total exceeds the cap **drop with no owned allocation** (record `osc52_over_cap_drops`, return `Ok(())`). Only under the cap do you allocate owned `ClipboardMimePart`s. `ClipboardContent.data` is **already base64-decoded** by the binding — the cap is on decoded bytes.
- **Preserve location + all MIME representations** — build `Vec<ClipboardMimePart>` from every `contents()` item; never flatten to one `Vec<u8>`; carry `ClipboardLocation` (Standard/Selection/Primary) through unchanged.
- **OSC-52 reads never reach the callback** — the binding never delivers read requests to `on_clipboard_write` (`vendor/.../terminal.rs:1676-1677`). Keep the deny as a **by-construction invariant + a test** asserting an OSC-52 `?` query emits **no** event. Do not add a read path.
- **Policy is session-scoped behind `ClipboardPolicy`** — the paste multiline "don't warn again" flag and the OSC-52 "allow/deny for session" decision live in foreground per-tab state (reset per process). The trait is the seam; `lens-ui` injects a persisted impl later. The spec's "persisted" wording is a **documented deferral** — 2b ships the in-memory `SessionClipboardPolicy`.
- **Async foreground policy, never in the callback:** the drain consults `ClipboardPolicy::osc52_session_decision(location)`; a remembered decision applies directly, otherwise it mints a `HostRequestId`, stashes the write in a **bounded** `pending_clipboard_writes` map (`PENDING_HOST_REQUESTS_CAP = 64`, drop-oldest), and emits `TerminalEvent::ClipboardWriteRequest`. On `HostRequestResponse` Allow the foreground performs `cx.write_to_clipboard`.
- **Paste is user input → never-drop + epoch-revocable + read-only-gated:** `EngineCommand::Paste` rides the **same `InputForwarder` never-drop path as `Key`** (`is_stale` + `send_stale_revoke_ack` + `enqueue_input` epoch stamping all gain a `Paste` arm); a read-only downgrade revokes a queued paste via `access_epoch`. `Cmd+V` is suppressed entirely when `write_input_allowed()` is false.
- **`Cmd+V` must be intercepted before the key encoder:** `keydown_should_enqueue` returns `true` for **any** `platform` (Cmd) modifier (`key_map.rs:13`), so Cmd+V would otherwise be encoded as a key to the PTY. Intercept `platform && key == "v"` (no ctrl/alt/function) at the top of `handle_key_down` (and the test entry points), route to paste, `cx.stop_propagation()`, and return **before** the key path.
- **`MAX_PASTE_BYTES = 1 << 20` (1 MiB), reject-not-truncate:** an over-cap paste is **rejected with a visible marker** (`presentation.input_discarded = true` + `PresentationChanged`), never silently truncated (a mid-sequence cut could sever an escape sequence — Design Point 5 never-silent-drop).
- **Bracketed encoding is engine-side against the live mode:** the worker reads `terminal.mode(Mode::BRACKETED_PASTE)` (mode 2004, `vendor/.../terminal.rs:951`) then `paste::encode(&mut work, bracketed, &mut buf)` — the same "modes as-of stream position" discipline the single ordered stream gives keys. `paste::encode` strips control bytes → spaces, wraps `ESC[200~…ESC[201~` when bracketed, newlines→CR when not; **output ≤ input + 12 bytes**, so a `data.len() + 16` buffer never needs growing.
- **No Ghostty type escapes the engine boundary** — Lens-owned `ClipboardLocation`/`ClipboardMimePart`/`PasteInput` only.
- **`#[gpui::test]` / `NoopTextSystem` false-greens painted-window paths** ([[gpui-test-noop-text-system]]). 2b's correctness is **hermetic** (engine-thread golden via `vt_write` + drain logic via `#[gpui::test]` emit-capture); real Cmd+V-through-a-window is an **optional end-of-slice check**, not a per-task gate. Any real-window run: read [[terminal-realwindow-harness-pitfalls]] first, user heads-up.
- Ground truth: `docs/specs/2026-07-17-terminal-slice-2-interaction-design.md` (§2b + **2026-07-20 amendment** + command seam); **2a/2d committed code** on `terminal-ws`; real APIs in `lib.rs`, `engine/{vt,worker,handle,forwarder,command,presentation,inspect}.rs`, `input_gate.rs`, `vendor/libghostty-rs/libghostty-vt/src/{terminal,paste}.rs`.

## Deferred (out of this plan)

- **Selection, `Cmd+C` copy, mouse capture, pixel→cell hit-testing, motion coalescing, XTSHIFTESCAPE** — all **Slice 2c** (2026-07-20 re-cut). Do not build any of them here.
- **Persisted** clipboard/paste preferences — `lens-ui` injects a persisted `ClipboardPolicy`; 2b ships the in-memory impl + the seam.
- **Primary-selection / middle-click paste** — not a macOS gesture; not in scope.
- **Progress / desktop notifications** — deferred at 2d (no binding payload accessor); unchanged.
- **Bracketed-mode-gated warn nuance** (review finding I6): the parent design warns on multiline paste **only when bracketed paste is inactive** (mode 2004 already protects against injection when active). The foreground has **no live mode-2004 snapshot** today (2c builds the first foreground mouse-/terminal-mode snapshot), so 2b uses the **safe over-approximation: always warn on multiline** (suppressible via "don't warn again"). This is an intentional, documented deviation — suppressing the warn while bracketed paste is active is deferred until a foreground mode snapshot exists. Note this in `docs/STATUS.md` at slice end.
- **Menu Edit→Paste (`OsAction::Paste`)** (review finding M4): the app-menu paste command is a distinct gpui path from the Cmd+V keystroke; 2b intercepts only the keystroke (`handle_key_down`). Wiring the menu action to the same `handle_paste` is deferred to `lens-ui` menu integration (the standalone demo has no app menu). Not a Cmd+V bypass — the keystroke path is fully covered.

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/lens-terminal/src/engine/presentation.rs` | **Modify.** Add `MAX_OSC52_CLIPBOARD_BYTES`; `map_clipboard_location`; extend `PresentationDrainResult` with `clipboard_writes`; collect `ClipboardWrite` in `collect_presentation_drain`. |
| `crates/lens-terminal/src/engine/vt.rs` | **Modify.** Clone `presentation_tx` for a second closure; register `on_clipboard_write` (cap-before-clone); add `encode_paste`. |
| `crates/lens-terminal/src/engine/inspect.rs` | **Modify.** Counters: `osc52_forwarded`, `osc52_over_cap_drops`, `clipboard_writes_allowed`, `clipboard_writes_denied`, `pastes_sent`, `paste_over_cap_rejects`, `paste_warn_prompts` + `record_*` methods + `EngineInspect` fields + `snapshot()`. |
| `crates/lens-terminal/src/engine/command.rs` | **Modify.** Add `PasteInput { bytes, access_epoch, ack }`. |
| `crates/lens-terminal/src/engine/worker.rs` | **Modify.** Add `EngineCommand::Paste(PasteInput)` variant + `handle_command` arm (mirror `Key`). |
| `crates/lens-terminal/src/engine/forwarder.rs` | **Modify.** `is_stale` + `send_stale_revoke_ack` gain `Paste` arms. |
| `crates/lens-terminal/src/engine/handle.rs` | **Modify.** `enqueue_input` stamps `Paste` epoch; `record_clipboard_write_allowed/denied` + `record_paste_*` pass-throughs on the handle inspect arc. |
| `crates/lens-terminal/src/clipboard_policy.rs` | **Create.** `ClipboardPolicy` trait + in-memory `SessionClipboardPolicy`. |
| `crates/lens-terminal/src/lib.rs` | **Modify.** `TerminalEvent::{ClipboardWriteRequest, ClipboardWriteNotice, PasteWarnRequest}`; `HostRequestDecision::{AllowSession, DenySession}`; tab fields (`clipboard_policy`, `pending_clipboard_writes`, `pending_pastes`); fill drain clipboard arm; fill `on_host_event`; Cmd+V intercept + `handle_paste`/`reject_over_cap_paste`/`dispatch_paste`; `MAX_PASTE_BYTES`/`PENDING_HOST_REQUESTS_CAP`; pure `paste_needs_warn`/`is_paste_keystroke`. |
| `crates/lens-terminal/src/engine/mod.rs` | **Modify.** `pub use` `MAX_OSC52_CLIPBOARD_BYTES` if needed; `mod clipboard_policy` is at crate root (in `lib.rs`). |
| `crates/lens-terminal/benches/engine.rs` | **Modify.** `osc52_callback_throughput` + `paste_encode_throughput` benches. |
| `crates/lens-terminal/Cargo.toml` | **Modify.** Add `base64 = "0.22"` to `[dev-dependencies]`. |
| `crates/lens-terminal-demo/src/main.rs` | **Modify.** Handle `ClipboardWriteRequest` + `PasteWarnRequest` explicitly — **Deny/decline by default**, driving an in-memory `SessionClipboardPolicy`. |
| `docs/superpowers/plans/2026-07-20-terminal-slice-2b-clipboard-paste.md` | This plan. |

---

### Task 1: OSC-52 `on_clipboard_write` — cap-before-clone + registration + engine-thread golden

**Files:**
- Modify: `crates/lens-terminal/src/engine/presentation.rs` (`MAX_OSC52_CLIPBOARD_BYTES`, `map_clipboard_location`)
- Modify: `crates/lens-terminal/src/engine/vt.rs` (`new_shared`: clone `presentation_tx`, register `on_clipboard_write`)
- Modify: `crates/lens-terminal/src/engine/inspect.rs` (`osc52_forwarded`, `osc52_over_cap_drops`)
- Modify: `crates/lens-terminal/Cargo.toml` (`base64` dev-dep)
- Test: `crates/lens-terminal/src/engine/vt.rs` (`#[cfg(test)]` engine-thread golden via a real `VtEngine` + `presentation_rx`)

**Interfaces:**
- Consumes: `EnginePresentationEvent::ClipboardWrite { location: ClipboardLocation, contents: Vec<ClipboardMimePart> }`, `ClipboardLocation`, `ClipboardMimePart` (all declared in `presentation.rs`); the presentation channel `Sender` already threaded into `new_shared`.
- Produces:
  - `pub const MAX_OSC52_CLIPBOARD_BYTES: usize = 1 << 20;` (in `presentation.rs`)
  - `fn map_clipboard_location(loc: libghostty_vt::terminal::ClipboardLocation) -> ClipboardLocation` (a **private fn in `vt.rs`**, next to `on_clipboard_write` — keeps `presentation.rs` Ghostty-free, review finding I7)
  - `InspectShared::record_osc52_forwarded(&self)` / `record_osc52_over_cap_drop(&self)` + `EngineInspect.osc52_forwarded` / `.osc52_over_cap_drops`
  - A registered `on_clipboard_write` closure on every `VtEngine` that caps + forwards `ClipboardWrite`.

- [ ] **Step 1: Add the cap constant (`presentation.rs`)**

Only the constant lives in `presentation.rs` — it stays **Ghostty-free** (zero `libghostty_vt` imports today; keep it that way, review finding I7). The `libghostty_vt::terminal::ClipboardLocation` → Lens `ClipboardLocation` mapper is defined next to the callback in `vt.rs` (Step 5), not here.

```rust
/// Max **decoded** clipboard bytes (summed across MIME parts) an OSC 52 write may
/// carry before it is dropped. Applied BEFORE any owned allocation. 1 MiB is well
/// above real copy sizes; the bound stops a hostile program forcing large owned copies.
pub const MAX_OSC52_CLIPBOARD_BYTES: usize = 1 << 20;
```

- [ ] **Step 2: Add the inspect counters (`inspect.rs`)**

Add `osc52_forwarded: AtomicU64` + `osc52_over_cap_drops: AtomicU64` fields to `InspectShared` (init `AtomicU64::new(0)`), the matching `u64` fields to `EngineInspect`, read them in `snapshot()`, and add two enabled-gated recorders modeled on `record_hyperlink_open` (no ring event needed — counter only):

```rust
pub fn record_osc52_forwarded(&self) {
    if !self.enabled.load(Ordering::Relaxed) { return; }
    self.osc52_forwarded.fetch_add(1, Ordering::Relaxed);
}
pub fn record_osc52_over_cap_drop(&self) {
    if !self.enabled.load(Ordering::Relaxed) { return; }
    self.osc52_over_cap_drops.fetch_add(1, Ordering::Relaxed);
}
```

- [ ] **Step 3: Write the failing engine-thread golden tests (`vt.rs` `#[cfg(test)]`)**

Add `base64 = "0.22"` to `[dev-dependencies]` first. These drive a real `VtEngine` (single-thread, no worker) — feed OSC 52 bytes via `engine.feed(...)`, read the `presentation_rx` the test owns. Build the engine with a test presentation channel:

```rust
#[cfg(test)]
mod clipboard_tests {
    use super::*;
    use crate::engine::presentation::{
        ClipboardLocation, EnginePresentationEvent, MAX_OSC52_CLIPBOARD_BYTES,
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    fn osc52(pc: &str, decoded: &[u8]) -> Vec<u8> {
        let mut v = Vec::from(format!("\x1b]52;{pc};").as_bytes());
        v.extend_from_slice(STANDARD.encode(decoded).as_bytes());
        v.push(0x07); // BEL terminator
        v
    }

    fn engine_with_rx() -> (VtEngine, crossbeam_channel::Receiver<EnginePresentationEvent>) {
        let (tx, rx) = crossbeam_channel::bounded(crate::engine::presentation::PRESENTATION_CHANNEL_CAP);
        let cfg = EngineConfig { cols: 40, rows: 8, max_scrollback: 0, cell_w_px: 8, cell_h_px: 16 };
        let engine = VtEngine::new(&cfg, |_| {}, tx).expect("engine");
        (engine, rx)
    }

    #[test]
    fn osc52_write_under_cap_emits_clipboard_event_with_location_and_data() {
        let (mut engine, rx) = engine_with_rx();
        engine.feed(&osc52("c", b"hello-copy"));
        match rx.try_recv().expect("clipboard event") {
            EnginePresentationEvent::ClipboardWrite { location, contents } => {
                assert_eq!(location, ClipboardLocation::Standard);
                assert_eq!(contents.len(), 1);
                assert_eq!(contents[0].data, "hello-copy");
            }
            other => panic!("expected ClipboardWrite, got {other:?}"),
        }
    }

    #[test]
    fn osc52_write_over_cap_drops_before_clone_no_event() {
        let (mut engine, rx) = engine_with_rx();
        let big = vec![b'x'; MAX_OSC52_CLIPBOARD_BYTES + 1];
        engine.feed(&osc52("c", &big));
        assert!(rx.try_recv().is_err(), "over-cap OSC 52 must emit no event");
    }

    #[test]
    fn osc52_write_cap_minus_one_emits() {
        let (mut engine, rx) = engine_with_rx();
        let below = vec![b'z'; MAX_OSC52_CLIPBOARD_BYTES - 1];
        engine.feed(&osc52("c", &below));
        assert!(matches!(rx.try_recv(), Ok(EnginePresentationEvent::ClipboardWrite { .. })));
    }

    #[test]
    fn osc52_write_at_cap_emits() {
        let (mut engine, rx) = engine_with_rx();
        let at = vec![b'y'; MAX_OSC52_CLIPBOARD_BYTES];
        engine.feed(&osc52("c", &at));
        assert!(matches!(rx.try_recv(), Ok(EnginePresentationEvent::ClipboardWrite { .. })));
    }

    #[test]
    fn osc52_read_query_emits_no_event() {
        let (mut engine, rx) = engine_with_rx();
        engine.feed(b"\x1b]52;c;?\x07"); // read request — binding never delivers reads
        assert!(rx.try_recv().is_err(), "OSC 52 read must not produce a host event");
    }
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test -p lens-terminal --lib clipboard_tests`
Expected: FAIL — `on_clipboard_write` not registered yet, no events on `rx`.

- [ ] **Step 5: Register `on_clipboard_write` in `VtEngine::new_shared` (`vt.rs`)**

**Ordering is load-bearing (review finding C1).** The existing code at `vt.rs:103` does `let title_tx = presentation_tx;` and then the `on_title_changed(move |term| …)` closure **moves** `title_tx`. You MUST take the clipboard clone **before** that move. Concretely:

1. At `vt.rs:103`, immediately after `let title_tx = presentation_tx;` and **before** the `terminal.on_title_changed(…)?;` call, insert the clipboard-side clones:

```rust
let title_tx = presentation_tx;         // existing line (vt.rs:103)
let clip_tx = title_tx.clone();          // NEW — MUST precede the on_title_changed move below
let waker_for_clip = waker.clone();
let inspect_for_clip = inspect.clone();
```

`crossbeam_channel::Sender` is `Clone`; only the ordering matters — a clone taken *after* `on_title_changed` consumes `title_tx` is a use-after-move compile error.

2. Add the private location mapper near the callback (keeps `presentation.rs` Ghostty-free, finding I7):

```rust
fn map_clipboard_location(
    loc: libghostty_vt::terminal::ClipboardLocation,
) -> ClipboardLocation {
    use libghostty_vt::terminal::ClipboardLocation as L;
    match loc {
        L::Standard => ClipboardLocation::Standard,
        L::Selection => ClipboardLocation::Selection,
        L::Primary => ClipboardLocation::Primary,
    }
}
```

3. **After** the existing `terminal.on_title_changed(…)?;` registration and before `Ok(Self { .. })`, register the capped, non-blocking clipboard closure using the `clip_tx`/`waker_for_clip`/`inspect_for_clip` you cloned in step 1:

```rust
// --- 2b: OSC 52 clipboard-write effect (result IGNORED by OSC 52; cap + forward only) ---
terminal.on_clipboard_write(move |_term, write| {
    let location = map_clipboard_location(write.location());
    // cap BEFORE clone: borrow (&mime,&data) refs (no data copy), sum decoded bytes.
    let parts: Vec<(&str, &str)> = write.contents().map(|c| (c.mime, c.data)).collect();
    let total: usize = parts.iter().map(|(_, d)| d.len()).sum();
    if total > MAX_OSC52_CLIPBOARD_BYTES {
        if let Some(insp) = inspect_for_clip.as_ref() {
            insp.record_osc52_over_cap_drop();
        }
        return Ok(()); // OSC 52 ignores the result; drop with no owned allocation
    }
    let contents: Vec<ClipboardMimePart> = parts
        .into_iter()
        .map(|(mime, data)| ClipboardMimePart { mime: mime.to_owned(), data: data.to_owned() })
        .collect();
    match clip_tx.try_send(EnginePresentationEvent::ClipboardWrite { location, contents }) {
        Ok(()) => {
            if let Some(insp) = inspect_for_clip.as_ref() { insp.record_osc52_forwarded(); }
        }
        Err(TrySendError::Full(_)) => {
            if let Some(insp) = inspect_for_clip.as_ref() { insp.record_presentation_channel_full_drop(); }
        }
        Err(TrySendError::Disconnected(_)) => {}
    }
    if let Some(w) = waker_for_clip.as_ref()
        && let Ok(guard) = w.lock()
        && let Some(f) = guard.as_ref()
    {
        f();
    }
    Ok(())
})?;
```

Add the imports the closure needs to `vt.rs`: `ClipboardMimePart`, `MAX_OSC52_CLIPBOARD_BYTES` from `super::presentation` (`map_clipboard_location` is now a local `fn` in `vt.rs`; `TrySendError` is already imported for the title path). The closure signature is `move |_term, write|` — the binding calls `func(&term, ClipboardWrite)` (verified `vendor/.../terminal.rs:1686`); the trait is `FnMut(&Terminal, ClipboardWrite<'_>) -> Result<(), ClipboardWriteError>`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p lens-terminal --lib clipboard_tests`
Expected: PASS (5 tests — under-cap, cap−1, at-cap, over-cap-drop, read-query-no-event).

- [ ] **Step 7: Gate + commit**

Run: `cargo fmt -p lens-terminal && cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings && cargo test -p lens-terminal --lib`
Expected: clean, all lib tests pass.

```bash
git add crates/lens-terminal/src/engine/{presentation,vt,inspect}.rs crates/lens-terminal/Cargo.toml
git commit -m "feat(terminal-2b): OSC 52 on_clipboard_write — cap-before-clone + forward"
```

---

### Task 2: Foreground clipboard policy — `ClipboardPolicy` seam, drain → `ClipboardWriteRequest`, `on_host_event` completion

**Files:**
- Create: `crates/lens-terminal/src/clipboard_policy.rs` (trait + `SessionClipboardPolicy`)
- Modify: `crates/lens-terminal/src/lib.rs` (`mod clipboard_policy;`; `HostRequestDecision::{AllowSession, DenySession}`; `TerminalEvent::ClipboardWriteRequest`; tab fields; drain clipboard arm; `on_host_event`; `PENDING_HOST_REQUESTS_CAP`; `record_clipboard_write_*` via handle)
- Modify: `crates/lens-terminal/src/engine/presentation.rs` (`PresentationDrainResult.clipboard_writes`; collect in `collect_presentation_drain`)
- Modify: `crates/lens-terminal/src/engine/inspect.rs` + `handle.rs` (`clipboard_writes_allowed`/`denied` counters + handle pass-throughs)
- Test: `crates/lens-terminal/src/clipboard_policy.rs` (pure policy) + `crates/lens-terminal/src/lib.rs` (`#[gpui::test]` drain emit + `on_host_event`)

**Interfaces:**
- Consumes: `EnginePresentationEvent::ClipboardWrite`, `ClipboardLocation`, `ClipboardMimePart`, `HostRequestId`, `TerminalHostEvent::HostRequestResponse`, `engine.presentation_rx()`, `engine.take_latest_title()`.
- Produces:
  - `pub trait ClipboardPolicy { fn paste_warn_suppressed(&self) -> bool; fn suppress_paste_warn(&mut self); fn osc52_session_decision(&self, location: &ClipboardLocation) -> Option<HostRequestDecision>; fn remember_osc52(&mut self, location: ClipboardLocation, decision: HostRequestDecision); }`
  - `pub struct SessionClipboardPolicy { .. }` implementing it (`Default`).
  - `TerminalEvent::ClipboardWriteRequest { id: HostRequestId, location: ClipboardLocation, contents: Vec<ClipboardMimePart> }`
  - `HostRequestDecision::{AllowSession, DenySession}` (existing `Allow`/`Deny` retained)
  - `PresentationDrainResult.clipboard_writes: Vec<(ClipboardLocation, Vec<ClipboardMimePart>)>`
  - `TerminalTab::{clipboard_policy, pending_clipboard_writes}` fields + `const PENDING_HOST_REQUESTS_CAP: usize = 64`
  - `EngineHandle::record_clipboard_write_allowed()/denied()` (pass-through to the inspect arc)

- [ ] **Step 1: Write the failing pure-policy tests (`clipboard_policy.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::presentation::ClipboardLocation;
    use crate::HostRequestDecision;

    #[test]
    fn session_policy_defaults_to_no_suppression_no_decision() {
        let p = SessionClipboardPolicy::default();
        assert!(!p.paste_warn_suppressed());
        assert_eq!(p.osc52_session_decision(&ClipboardLocation::Standard), None);
    }

    #[test]
    fn remembering_osc52_allow_is_returned_for_same_location_only() {
        let mut p = SessionClipboardPolicy::default();
        p.remember_osc52(ClipboardLocation::Standard, HostRequestDecision::Allow);
        assert_eq!(
            p.osc52_session_decision(&ClipboardLocation::Standard),
            Some(HostRequestDecision::Allow)
        );
        assert_eq!(p.osc52_session_decision(&ClipboardLocation::Primary), None);
    }

    #[test]
    fn suppress_paste_warn_sticks() {
        let mut p = SessionClipboardPolicy::default();
        p.suppress_paste_warn();
        assert!(p.paste_warn_suppressed());
    }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p lens-terminal --lib clipboard_policy`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement `clipboard_policy.rs` + register the module**

```rust
//! Session-scoped clipboard/paste policy seam. In-memory here; `lens-ui` injects
//! a persisted impl later (spec 2026-07-20 amendment — persistence deferred).

use std::collections::HashMap;

use crate::HostRequestDecision;
use crate::engine::presentation::ClipboardLocation;

/// Foreground policy for permissioned clipboard writes (OSC 52) + paste warnings.
pub trait ClipboardPolicy {
    fn paste_warn_suppressed(&self) -> bool;
    fn suppress_paste_warn(&mut self);
    /// A remembered `Allow`/`Deny` for this location this session, if any.
    fn osc52_session_decision(&self, location: &ClipboardLocation) -> Option<HostRequestDecision>;
    fn remember_osc52(&mut self, location: ClipboardLocation, decision: HostRequestDecision);
}

/// Default in-memory policy: everything resets on process exit.
#[derive(Default)]
pub struct SessionClipboardPolicy {
    paste_warn_suppressed: bool,
    osc52: HashMap<ClipboardLocation, HostRequestDecision>,
}

impl ClipboardPolicy for SessionClipboardPolicy {
    fn paste_warn_suppressed(&self) -> bool { self.paste_warn_suppressed }
    fn suppress_paste_warn(&mut self) { self.paste_warn_suppressed = true; }
    fn osc52_session_decision(&self, location: &ClipboardLocation) -> Option<HostRequestDecision> {
        self.osc52.get(location).cloned()
    }
    fn remember_osc52(&mut self, location: ClipboardLocation, decision: HostRequestDecision) {
        self.osc52.insert(location, decision);
    }
}
```

`ClipboardLocation` must be `Hash + Eq` for the map key — it already derives `PartialEq, Eq`; **add `Hash`** to its derive in `presentation.rs`. Add `mod clipboard_policy;` + `pub use clipboard_policy::{ClipboardPolicy, SessionClipboardPolicy};` to `lib.rs`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lens-terminal --lib clipboard_policy`
Expected: PASS (3 tests).

- [ ] **Step 5: Extend the typed seams + drain result (`lib.rs`, `presentation.rs`, `inspect.rs`)**

- `lib.rs`: `HostRequestDecision` gains `AllowSession, DenySession` (keep `Allow, Deny`). `TerminalEvent` gains `ClipboardWriteRequest { id: HostRequestId, location: ClipboardLocation, contents: Vec<ClipboardMimePart> }` **and** `ClipboardWriteNotice { location: ClipboardLocation, bytes: usize }` (the parent completion-matrix "**copy notice**" row — emitted whenever a clipboard write is actually performed, so the host can surface a toast/status; review finding I2). Import `ClipboardLocation`/`ClipboardMimePart` from `engine::presentation`.
- `presentation.rs`: `PresentationDrainResult` gains `pub clipboard_writes: Vec<(ClipboardLocation, Vec<ClipboardMimePart>)>` (add to the `Default` impl `Vec::new()`); in `collect_presentation_drain`, replace the `ClipboardWrite { .. } => {}` arm with one that pushes `(location, contents)` onto `clipboard_writes`. **Update the existing `presentation_inspect_drain_counters_zero_when_disabled` test literal** (`presentation.rs:347`) to add `clipboard_writes: Vec::new()`.
- `inspect.rs` + `handle.rs`: add `clipboard_writes_allowed`/`clipboard_writes_denied` counters (enabled-gated `record_*` like `record_hyperlink_open`) + `EngineHandle::record_clipboard_write_allowed()/denied()` pass-throughs (mirror how the drain reaches inspect via the handle arc — see `EngineHandle::record_presentation_drain_inspect`).

- [ ] **Step 6: Write the failing drain + `on_host_event` tests (`lib.rs` `#[gpui::test]`)**

Use the existing `#[gpui::test]` pattern in `lib.rs` (a `TestAppContext`, `cx.new(..)` a tab). Two pieces of boilerplate the sketches below rely on — wire them concretely (review finding I5):
- **Event capture:** subscribe with a shared collector before triggering the drain, mirroring `tests/presentation_realwindow.rs:104`:
  ```rust
  let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::<TerminalEvent>::new()));
  let sink = events.clone();
  let _sub = cx.update(|cx| cx.subscribe(&tab, move |_, ev: &TerminalEvent, _| sink.borrow_mut().push(ev.clone())));
  // ... trigger drain, then read events.borrow() ...
  ```
  **Hold the `_sub`** for the test's lifetime — a dropped `Subscription` silently detaches ([[terminal-realwindow-harness-pitfalls]]).
- **Inspect counters:** the `record_*` recorders no-op when inspect is disabled (`inspect.rs`), so any `clipboard_writes_*` / `pastes_sent` assertion must first enable inspect via the existing `set_inspect_enabled(true)` seam (pattern at `lib.rs:1818`). Snapshot via the handle's inspect accessor.
- Clipboard R/W in `#[gpui::test]` is real (TestAppContext backs it — `gpui .../test_context.rs`); `Context` derefs to `App`, so `cx.read_from_clipboard()`/`write_to_clipboard(..)` inside an entity update are valid and **not** a NoopTextSystem false-green (only font/shape/paint are faked).

Assert:

```rust
// (sketch — match the file's existing gpui-test helpers for tab construction + event capture)
#[gpui::test]
async fn clipboard_write_with_no_session_decision_emits_request(cx: &mut TestAppContext) {
    // enqueue a ClipboardWrite on the tab's presentation channel, run drain,
    // assert exactly one TerminalEvent::ClipboardWriteRequest with the same location+contents,
    // and that pending_clipboard_writes now holds its id.
}

#[gpui::test]
async fn remembered_deny_suppresses_request_and_records_denied(cx: &mut TestAppContext) {
    // policy.remember_osc52(Standard, Deny); enqueue a Standard ClipboardWrite; drain;
    // assert NO ClipboardWriteRequest emitted; clipboard_writes_denied incremented.
}

#[gpui::test]
async fn host_allow_writes_clipboard_and_evicts_pending(cx: &mut TestAppContext) {
    // drain → request → on_host_event(HostRequestResponse{ id, Allow }) →
    // cx.read_from_clipboard() returns the written text; pending map no longer holds id.
}

#[gpui::test]
async fn pending_clipboard_writes_are_bounded_drop_oldest(cx: &mut TestAppContext) {
    // emit PENDING_HOST_REQUESTS_CAP + 1 writes with no responses;
    // assert the map size == cap and the oldest id was evicted.
}
```

- [ ] **Step 7: Run to verify fail**, then implement, then pass.

Run: `cargo test -p lens-terminal --lib clipboard`
Expected: FAIL first.

- [ ] **Step 8: Implement the tab fields + drain arm + `on_host_event` (`lib.rs`)**

- Add fields to `TerminalTab`: `clipboard_policy: Box<dyn ClipboardPolicy>` (both constructors init `Box::new(SessionClipboardPolicy::default())`), `pending_clipboard_writes: std::collections::VecDeque<(HostRequestId, ClipboardLocation, Vec<ClipboardMimePart>)>` (a `VecDeque` gives drop-oldest cheaply). Add `const PENDING_HOST_REQUESTS_CAP: usize = 64;`.
- In `drain_presentation_events`, after the hyperlink loop, iterate `result.clipboard_writes`:

```rust
for (location, contents) in result.clipboard_writes {
    match self.clipboard_policy.osc52_session_decision(&location) {
        Some(HostRequestDecision::Allow | HostRequestDecision::AllowSession) => {
            self.write_clipboard_contents(&location, &contents, cx);
            if let Some(e) = self.engine_handle() { e.record_clipboard_write_allowed(); }
        }
        Some(HostRequestDecision::Deny | HostRequestDecision::DenySession) => {
            if let Some(e) = self.engine_handle() { e.record_clipboard_write_denied(); }
        }
        None => {
            let id = HostRequestId(self.next_host_request_id);
            self.next_host_request_id = self.next_host_request_id.wrapping_add(1);
            if self.pending_clipboard_writes.len() >= PENDING_HOST_REQUESTS_CAP {
                self.pending_clipboard_writes.pop_front();
            }
            self.pending_clipboard_writes.push_back((id, location.clone(), contents.clone()));
            cx.emit(TerminalEvent::ClipboardWriteRequest { id, location, contents });
        }
    }
}
```

- `write_clipboard_contents(&self, location: &ClipboardLocation, contents: &[ClipboardMimePart], cx: &mut Context<Self>)`: pick the `text/plain` part if present else the first part; `cx.write_to_clipboard(gpui::ClipboardItem::new_string(part.data.clone()))`; then emit the copy notice `cx.emit(TerminalEvent::ClipboardWriteNotice { location: location.clone(), bytes: contents.iter().map(|p| p.data.len()).sum() })` (this is the single notice-emit site, so both the drain remembered-Allow path and the `on_host_event` Allow path surface it). An empty `contents` (or no part) writes nothing and emits no notice.
- `engine_handle(&self) -> Option<&EngineHandle>` helper if one isn't already present (`self.runtime.as_ref()?.engine.as_ref()`).
- Fill `on_host_event` (currently a no-op at `lib.rs:510`): on `TerminalHostEvent::HostRequestResponse { id, decision }`, find + remove the matching `(id, location, contents)` entry in `pending_clipboard_writes`; if found:
  - `Allow | AllowSession` → `self.write_clipboard_contents(&location, &contents, cx)` (which emits the `ClipboardWriteNotice`), `record_clipboard_write_allowed`; `AllowSession` also `self.clipboard_policy.remember_osc52(location, HostRequestDecision::Allow)`.
  - `Deny | DenySession` → `record_clipboard_write_denied`; `DenySession` → `remember_osc52(location, HostRequestDecision::Deny)`.
  - If no clipboard entry matches, leave for Task 4's paste-pending lookup (added there) / ignore (OpenUrl responses carry no pending state). Keep `Sleep`/`Wake` arms as they are (or `todo`-free no-ops matching current behavior).

- [ ] **Step 9: Run to verify pass + gate + commit**

Run: `cargo test -p lens-terminal --lib && cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings && cargo fmt -p lens-terminal --check`
Expected: clean.

```bash
git add crates/lens-terminal/src/clipboard_policy.rs crates/lens-terminal/src/lib.rs crates/lens-terminal/src/engine/{presentation,inspect,handle}.rs
git commit -m "feat(terminal-2b): foreground OSC 52 policy — ClipboardPolicy seam + ClipboardWriteRequest + on_host_event"
```

---

### Task 3: `EngineCommand::Paste` — worker bracketed encode + never-drop/epoch wiring + golden

**Files:**
- Modify: `crates/lens-terminal/src/engine/command.rs` (`PasteInput`)
- Modify: `crates/lens-terminal/src/engine/worker.rs` (`EngineCommand::Paste` variant + `handle_command` arm)
- Modify: `crates/lens-terminal/src/engine/forwarder.rs` (`is_stale` + `send_stale_revoke_ack` `Paste` arms)
- Modify: `crates/lens-terminal/src/engine/handle.rs` (`enqueue_input` stamps `Paste` epoch)
- Modify: `crates/lens-terminal/src/engine/vt.rs` (`encode_paste`)
- Test: `crates/lens-terminal/src/engine/vt.rs` (bracketed golden) + `forwarder.rs` (stale-paste revoke) + `handle.rs` (integration bracketed egress via a real worker, bounded-ack)

**Interfaces:**
- Consumes: `InputAck`, `EgressKind::Input`, `try_emit_user_input`, `access_epoch`, `Mode::BRACKETED_PASTE`, `paste::encode`.
- Produces:
  - `pub(crate) struct PasteInput { pub bytes: Vec<u8>, pub access_epoch: u64, pub ack: Option<Sender<InputAck>> }` (in `command.rs`)
  - `EngineCommand::Paste(PasteInput)` (in `worker.rs`)
  - `VtEngine::encode_paste(&mut self, data: &[u8]) -> Result<Vec<u8>, EngineError>`

- [ ] **Step 1: Add `PasteInput` (`command.rs`)** — mirror `KeyInput`'s epoch/ack shape:

```rust
#[derive(Debug)]
pub(crate) struct PasteInput {
    pub bytes: Vec<u8>,
    pub access_epoch: u64,
    pub ack: Option<Sender<InputAck>>,
}
```

- [ ] **Step 2: Write the failing `encode_paste` golden (`vt.rs` `#[cfg(test)]`)**

```rust
#[test]
fn encode_paste_wraps_bracketed_when_mode_2004_enabled() {
    let cfg = EngineConfig { cols: 40, rows: 8, max_scrollback: 0, cell_w_px: 8, cell_h_px: 16 };
    let (tx, _rx) = crossbeam_channel::bounded(1);
    let mut engine = VtEngine::new(&cfg, |_| {}, tx).expect("engine");
    engine.feed(b"\x1b[?2004h"); // enable bracketed paste
    let out = engine.encode_paste(b"ab").expect("encode");
    assert_eq!(out, b"\x1b[200~ab\x1b[201~");
}

#[test]
fn encode_paste_plain_when_bracketed_disabled_and_strips_esc() {
    let cfg = EngineConfig { cols: 40, rows: 8, max_scrollback: 0, cell_w_px: 8, cell_h_px: 16 };
    let (tx, _rx) = crossbeam_channel::bounded(1);
    let mut engine = VtEngine::new(&cfg, |_| {}, tx).expect("engine");
    let out = engine.encode_paste(b"a\x1bb").expect("encode"); // ESC stripped -> space
    assert_eq!(out, b"a b");
}
```

- [ ] **Step 3: Run to verify fail**

Run: `cargo test -p lens-terminal --lib encode_paste`
Expected: FAIL — `encode_paste` not defined.

- [ ] **Step 4: Implement `encode_paste` (`vt.rs`)** — output ≤ input+12, so `len+16` never grows:

```rust
/// Encode paste bytes against the live bracketed-paste mode (mode 2004).
pub(crate) fn encode_paste(&mut self, data: &[u8]) -> Result<Vec<u8>, EngineError> {
    use libghostty_vt::terminal::Mode;
    let bracketed = self.terminal.mode(Mode::BRACKETED_PASTE)?;
    let mut work = data.to_vec(); // paste::encode mutates in place
    let mut buf = vec![0u8; data.len() + 16]; // bracket wrapper is 12 bytes; strip/CR are 1:1
    let n = libghostty_vt::paste::encode(&mut work, bracketed, &mut buf)?;
    buf.truncate(n);
    Ok(buf)
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p lens-terminal --lib encode_paste`
Expected: PASS (2 tests).

- [ ] **Step 6: Wire the never-drop / epoch path (`worker.rs`, `forwarder.rs`, `handle.rs`)**

- `worker.rs`: add `Paste(super::command::PasteInput)` to `EngineCommand`; in `handle_command`, add an arm mirroring `Key` — epoch-check (`cmd_epoch != current_epoch` → `(Vec::new(), false)`), else `engine.encode_paste(&input.bytes)`, re-check epoch after encode, `try_emit_user_input(egress.as_ref(), EgressKind::Input, &bytes)`, `record_user_egress_accepted/rejected`, and `ack` the `InputAck { encoded, accepted }`.
- `forwarder.rs`: `is_stale` gains `EngineCommand::Paste(p) => p.access_epoch < current_epoch`; `send_stale_revoke_ack` gains a `Paste` arm that acks `{ encoded: Vec::new(), accepted: false }` when `p.ack` is `Some`.
- `handle.rs`: `enqueue_input`'s `match &mut cmd` gains `EngineCommand::Paste(input) => input.access_epoch = epoch`.

- [ ] **Step 7: Write the failing stale-paste + integration tests**

- `forwarder.rs`: extend `is_stale_drops_prior_epoch_key_and_focus_not_scroll` (or add a test) asserting a `Paste` with a prior epoch is stale and its ack is revoked (`accepted == false`).
- `handle.rs`: an integration test that spawns a real `EngineHandle`, sets egress via a test receiver, feeds `\x1b[?2004h`, enqueues `Paste` with an ack, and asserts the egress receives `\x1b[200~..\x1b[201~` (bounded-ack synchronization — recv_timeout on the ack, then recv the egress frame; **no sleeps**, per Cross-cutting DP1).

- [ ] **Step 8: Run to verify fail → implement → pass + gate + commit**

Run: `cargo test -p lens-terminal --lib && cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings`
Expected: clean.

```bash
git add crates/lens-terminal/src/engine/{command,worker,forwarder,handle,vt}.rs
git commit -m "feat(terminal-2b): EngineCommand::Paste — engine bracketed encode + never-drop/epoch wiring"
```

---

### Task 4: Foreground paste — Cmd+V intercept, read-only gate, multiline warn, cap

**Files:**
- Modify: `crates/lens-terminal/src/lib.rs` (`TerminalEvent::PasteWarnRequest`; `MAX_PASTE_BYTES`; `pending_pastes`; Cmd+V intercept in `handle_key_down` + test entry points; `handle_paste`/`dispatch_paste`; pure `paste_needs_warn`; `on_host_event` paste-pending arm; `record_paste_*`)
- Modify: `crates/lens-terminal/src/engine/inspect.rs` + `handle.rs` (`pastes_sent`, `paste_over_cap_rejects`, `paste_warn_prompts` counters + pass-throughs)
- Test: `crates/lens-terminal/src/lib.rs` (pure `paste_needs_warn`; `#[gpui::test]` paste routing)

**Interfaces:**
- Consumes: `write_input_allowed()`, `engine.enqueue_input(EngineCommand::Paste(..))`, `cx.read_from_clipboard()`, `ClipboardItem::text()`, `HostRequestResponse`.
- Produces:
  - `TerminalEvent::PasteWarnRequest { id: HostRequestId, line_count: usize }`
  - `const MAX_PASTE_BYTES: usize = 1 << 20;`
  - `fn paste_needs_warn(text: &str, suppressed: bool) -> bool` (pure: `!suppressed && text.contains('\n')`)
  - `TerminalTab::pending_pastes: VecDeque<(HostRequestId, Vec<u8>)>`

- [ ] **Step 1: Write the failing pure + routing tests (`lib.rs`)**

```rust
#[test]
fn paste_needs_warn_only_on_multiline_and_not_suppressed() {
    assert!(paste_needs_warn("a\nb", false));
    assert!(!paste_needs_warn("ab", false));
    assert!(!paste_needs_warn("a\nb", true));
}
```

Plus `#[gpui::test]` routing tests (match the file's tab-construction + the Step 6/I5 subscribe + `set_inspect_enabled(true)` boilerplate from Task 2):
- **`real_cmd_v_keystroke_routes_to_paste_not_key_encoder`** (drives the intercept, not `debug_paste_for_test` — finding I4): set clipboard = "hi"; build a `gpui::Keystroke` with `modifiers.platform = true`, `key == "v"`; call `debug_handle_key_down_for_test` (the production `handle_key_down`); assert `pastes_sent == 1` **and no key was encoded** (`keys_encoded == 0` / no `Key` egress). This is the test that would catch a missing/broken `is_paste_keystroke` guard — without it Cmd+V hits the key encoder (`keydown_should_enqueue` returns `true` for any `platform` mod, `key_map.rs:12-14`).
- `cmd_v_single_line_dispatches_paste` — clipboard = "hello"; `debug_paste_for_test` → `pastes_sent == 1`, no `PasteWarnRequest`.
- `cmd_v_multiline_unsuppressed_emits_warn_no_dispatch` — clipboard = "a\nb" → exactly one `PasteWarnRequest { line_count: 2 }`, `pastes_sent == 0`, one entry in `pending_pastes`.
- `paste_warn_allow_session_suppresses_and_dispatches` — after `HostRequestResponse{ id, AllowSession }`, `paste_warn_suppressed()` is true and the paste dispatched.
- `over_cap_paste_rejected_before_pending` — clipboard > `MAX_PASTE_BYTES` (multiline, to prove it is rejected **before** the warn/pending branch) → `presentation.input_discarded == true`, `pastes_sent == 0`, `paste_over_cap_rejects == 1`, **`pending_pastes` empty** (finding I1).
- `read_only_tab_ignores_cmd_v` — read-only tab → nothing read/dispatched.

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p lens-terminal --lib paste`
Expected: FAIL.

- [ ] **Step 3: Add the seams + counters** — `TerminalEvent::PasteWarnRequest { id, line_count }`; `const MAX_PASTE_BYTES: usize = 1 << 20;`; `pending_pastes: VecDeque<(HostRequestId, Vec<u8>)>` field (both ctors init empty); inspect counters `pastes_sent`/`paste_over_cap_rejects`/`paste_warn_prompts` + `EngineHandle::record_paste_sent()/record_paste_over_cap_reject()/record_paste_warn_prompt()` pass-throughs.

- [ ] **Step 4: Implement the Cmd+V intercept + paste flow (`lib.rs`)**

```rust
fn is_paste_keystroke(ks: &gpui::Keystroke) -> bool {
    ks.modifiers.platform
        && !ks.modifiers.control
        && !ks.modifiers.alt
        && !ks.modifiers.function
        && ks.key == "v"
}

fn paste_needs_warn(text: &str, suppressed: bool) -> bool {
    !suppressed && text.contains('\n')
}
```

At the **top** of `handle_key_down` (before `keydown_should_enqueue`):

```rust
if is_paste_keystroke(ks) {
    self.handle_paste(cx);
    cx.stop_propagation();
    return;
}
```

Do the same guard in `debug_key_down_for_test` and `debug_handle_key_down_for_test` so harness paths match production. Add a `#[cfg(any(test, feature = "test-util"))] pub fn debug_paste_for_test(&mut self, cx: &mut Context<Self>)` that calls `handle_paste` directly (tests set the clipboard via `cx.write_to_clipboard` first).

```rust
fn handle_paste(&mut self, cx: &mut Context<Self>) {
    if !self.write_input_allowed() {
        return; // read-only: paste suppressed
    }
    let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else { return };
    if text.is_empty() { return; }
    // Cap BEFORE the warn/pending branch so an over-cap payload is never stashed in
    // pending_pastes (a count-capped-but-not-byte-capped leak otherwise; finding I1).
    if text.len() > MAX_PASTE_BYTES {
        self.reject_over_cap_paste(cx);
        return;
    }
    if paste_needs_warn(&text, self.clipboard_policy.paste_warn_suppressed()) {
        let line_count = text.lines().count();
        let id = HostRequestId(self.next_host_request_id);
        self.next_host_request_id = self.next_host_request_id.wrapping_add(1);
        if self.pending_pastes.len() >= PENDING_HOST_REQUESTS_CAP {
            self.pending_pastes.pop_front();
        }
        self.pending_pastes.push_back((id, text.into_bytes()));
        if let Some(e) = self.engine_handle() { e.record_paste_warn_prompt(); }
        cx.emit(TerminalEvent::PasteWarnRequest { id, line_count });
        return;
    }
    self.dispatch_paste(text.into_bytes(), cx);
}

/// Visible reject-with-marker for an over-cap paste (never silent truncation, DP5).
fn reject_over_cap_paste(&mut self, cx: &mut Context<Self>) {
    if let Some(e) = self.engine_handle() { e.record_paste_over_cap_reject(); }
    if !self.presentation.input_discarded {
        self.presentation.input_discarded = true;
        cx.emit(TerminalEvent::PresentationChanged);
        cx.notify();
    }
}

fn dispatch_paste(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
    if bytes.len() > MAX_PASTE_BYTES {
        // Defensive: handle_paste already caps; a pending-paste confirm is pre-capped.
        self.reject_over_cap_paste(cx);
        return;
    }
    let Some(engine) = self.engine_handle() else { return };
    let ok = engine
        .enqueue_input(EngineCommand::Paste(engine::command::PasteInput {
            bytes,
            access_epoch: 0, // stamped by enqueue_input
            ack: None,
        }))
        .is_ok();
    if ok {
        if let Some(e) = self.engine_handle() { e.record_paste_sent(); }
        self.clear_input_discarded(cx);
    }
}
```

Extend `on_host_event`'s `HostRequestResponse` arm (added in Task 2): after the clipboard lookup, if the id matches a `pending_pastes` entry, remove it and on `Allow`/`AllowSession` → (`AllowSession` also `self.clipboard_policy.suppress_paste_warn()`) `dispatch_paste(bytes, cx)`; on `Deny`/`DenySession` discard.

- [ ] **Step 5: Run to verify pass + gate + commit**

Run: `cargo test -p lens-terminal --lib && cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings && cargo fmt -p lens-terminal --check`
Expected: clean.

```bash
git add crates/lens-terminal/src/lib.rs crates/lens-terminal/src/engine/{inspect,handle}.rs
git commit -m "feat(terminal-2b): Cmd+V paste — intercept, read-only gate, multiline warn, cap"
```

---

### Task 5: Demo host policy + benches + inspect exposure + live-rider hook

**Files:**
- Modify: `crates/lens-terminal-demo/src/main.rs` (handle `ClipboardWriteRequest` + `PasteWarnRequest`; in-memory `SessionClipboardPolicy`; Deny/decline default)
- Modify: `crates/lens-terminal/benches/engine.rs` (`osc52_callback_throughput`, `paste_encode_throughput`)
- Modify: `crates/lens-terminal/tests/terminal_live.rs` (paste round-trip rider note/step)
- Test: bench compiles under `--features bench`; a `#[gpui::test]` asserting the inspect snapshot exposes the new counters after a drive.

**Interfaces:**
- Consumes: `TerminalEvent::{ClipboardWriteRequest, PasteWarnRequest}`, `HostRequestDecision::{Deny, Allow}`, `VtEngine::encode_paste`, the OSC-52 callback.
- Produces: demo host that **declines by default**; two benches; a live rider step.

- [ ] **Step 1: Extend the demo host loop (`main.rs`)** — replace the `_ =>` catch-all with explicit arms that **decline by default** (never auto-allow), matching the 2d OpenUrl pattern:

```rust
TerminalEvent::ClipboardWriteRequest { id, location, contents } => {
    let bytes: usize = contents.iter().map(|p| p.data.len()).sum();
    eprintln!("demo: ClipboardWriteRequest id={id:?} loc={location:?} bytes={bytes} — DENY by policy");
    let id = *id;
    this.update(cx, |tab, cx| {
        tab.on_host_event(
            TerminalHostEvent::HostRequestResponse { id, decision: HostRequestDecision::Deny },
            cx,
        );
    });
}
TerminalEvent::PasteWarnRequest { id, line_count } => {
    eprintln!("demo: PasteWarnRequest id={id:?} lines={line_count} — DENY (multiline) by policy");
    let id = *id;
    this.update(cx, |tab, cx| {
        tab.on_host_event(
            TerminalHostEvent::HostRequestResponse { id, decision: HostRequestDecision::Deny },
            cx,
        );
    });
}
```

```rust
TerminalEvent::ClipboardWriteNotice { location, bytes } => {
    // Copy notice — a program wrote the system clipboard (only fires after an Allow).
    eprintln!("demo: ClipboardWriteNotice loc={location:?} bytes={bytes}");
}
```

Keep a `_ =>` catch-all for future `#[non_exhaustive]` variants (still Deny/log). *(An env-gated `LENS_DEMO_ALLOW_CLIPBOARD=1` may flip the request arms to `Allow`/`AllowSession` to exercise the write + notice path manually — optional; default stays Deny.)*

- [ ] **Step 2: Add the benches (`benches/engine.rs`)** — `osc52_callback_throughput` (feed a fixed OSC-52 write repeatedly through a `VtEngine`, draining `rx`) and `paste_encode_throughput` (`encode_paste` over a representative payload, bracketed on). Guard behind the existing `bench` feature; run `cargo build -p lens-terminal --benches --features bench` to confirm they compile.

- [ ] **Step 3: Inspect exposure test** — a `#[gpui::test]` (or lib test) that enables inspect, drives one OSC-52 write (forwarded) + one dispatched paste, and asserts `snapshot().osc52_forwarded == 1` and `pastes_sent == 1`. (Guards against a later refactor dropping a `record_*` call — the same class of seam-test as 2d's.)

- [ ] **Step 4: Live-rider hook (`terminal_live.rs`)** — add a documented paste round-trip step (type nothing; set the clipboard, Cmd+V, assert the shell echoes the pasted text) as a manual/gated rider leg; OSC-52 has no hermetic live counterpart (program-driven) so leave it to manual observation. Match the file's existing env-gate/skip convention.

- [ ] **Step 5: Full gate + commit**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings && cargo test -p lens-terminal --lib && cargo build -p lens-terminal --benches --features bench && cargo build -p lens-terminal-demo`
Expected: all clean.

```bash
git add crates/lens-terminal-demo/src/main.rs crates/lens-terminal/benches/engine.rs crates/lens-terminal/tests/terminal_live.rs crates/lens-terminal/src/lib.rs
git commit -m "feat(terminal-2b): demo clipboard/paste host policy + benches + inspect exposure + live rider"
```

---

## Cross-cutting design points (baked in — keep honest during execution)

1. **Two-suite test seam** (spec DP1). OSC-52 + paste correctness is **hermetic**: (a) engine-thread golden via `engine.feed(vt_bytes)` reading a test-owned `presentation_rx` (OSC-52) / a real worker with a bounded ack (paste egress); (b) pure functions (`paste_needs_warn`, `SessionClipboardPolicy`, cap arithmetic). **No sleeps, no frame-polling** for synchronization.
2. **Callback never blocks** (spec): `on_clipboard_write` copies into owned data *after* the cap check, `try_send`s, wakes, returns `Ok(())`. Never `recv`/await/lock.
3. **cap-BEFORE-clone** is the security-relevant ordering — borrowed refs + sum, drop over-cap *before* any `to_owned()`. Reviewer must confirm no owned allocation precedes the cap check.
4. **Never-drop, epoch-revocable paste** (spec DP5): paste rides the `Key` forwarder path; a read-only downgrade revokes a queued paste; over-cap is *reject-with-marker*, never silent truncation.
5. **Async foreground policy** — the decision is on the foreground, not the engine; the callback is policy-free.

## Self-Review

- **Spec coverage (§2b + 2026-07-20 amendment + parent completion matrix):** OSC-52 registration + cap-before-clone (T1) · MIME/location preserved (T1) · read-denial by construction + test (T1) · foreground async policy + `ClipboardWriteRequest` + `ClipboardPolicy` seam + session decision (T2) · **copy notice `ClipboardWriteNotice` on every performed write (T2, parent matrix row)** · cap−1/cap/cap+1 tests (T1) · `Cmd+V` paste bracketed engine-side (T3) · read-only gate + multiline warn + "don't warn again" (T4) · payload cap, rejected-before-pending (T4) · inspect + benches (T5) · demo host policy Deny-by-default (T5) · live rider (T5). **Selection/copy/mouse explicitly deferred to 2c**; **bracketed-active warn-suppression + menu-paste explicitly deferred with STATUS note** (Deferred section). No unmapped §2b requirement remains for this slice.

## Cross-family review disposition (grok-4.5, 2026-07-20, source-verified)

Verdict **SHIP-WITH-FIXES** — all findings folded into this plan before execution: **C1** clone-before-title-move ordering (Task 1 Step 5) · **I1** paste capped before pending/warn (Task 4) · **I2** `ClipboardWriteNotice` copy notice added (Task 2 + demo) · **I3** cap−1 golden (Task 1) · **I4** real Cmd+V keystroke test driving the intercept (Task 4) · **I5** subscribe + `set_inspect_enabled` boilerplate (Task 2 Step 6) · **I6** always-warn documented deviation (Deferred) · **I7** location mapper moved to `vt.rs`, `presentation.rs` stays Ghostty-free. Confirmed-correct (do not "fix"): callback arity `|_term, write|`, decoded `&str` contents, `ClipboardContents` Clone, `paste::encode` ≤ input+12, gpui clipboard + `Context`→`App` deref, exhaustiveness site list complete, `HostRequestDecision` not `#[non_exhaustive]` (no match breaks). Full review: `.superpowers/sdd/grok-2b-plan-review.md`.
- **Type consistency:** `ClipboardLocation`/`ClipboardMimePart`/`EnginePresentationEvent::ClipboardWrite` reused from 2d (not redefined); `HostRequestDecision` extended (not replaced); `PasteInput` mirrors `KeyInput`; `PresentationDrainResult.clipboard_writes` consumed only in the drain; `record_*` names consistent across `inspect.rs`/`handle.rs`/call sites.
- **No placeholders:** every code step carries real code; test bodies are concrete except the `#[gpui::test]` tab-construction boilerplate, which must follow the existing helpers in `lib.rs` (noted explicitly rather than reinvented).
