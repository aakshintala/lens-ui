//! `lens-terminal` — deep terminal module wrapping the vendored Ghostty VT
//! engine behind a small, Lens-owned host interface.
//!
//! # Slice 0 — surface freeze
//!
//! This module freezes the **public type *names* and seam invariants** that
//! `lens-ui` (and the standalone demo) bind to. Per the workstream design
//! (`docs/specs/2026-07-16-terminal-workstream-design.md`, "Build sequence"),
//! **internal representations stay evolvable**: `Frame` fields, event payloads,
//! and options fields fill in as their producing + consuming slices (1a–1d)
//! converge. That deliberately avoids premature layer-boundary binding.
//!
//! ## Frozen seam invariants
//!
//! - [`open`] returns immediately in [`Lifecycle::Starting`]; discovery /
//!   create / attach run off-thread. **Failures become lifecycle values, never
//!   constructor errors.**
//! - **No Ghostty type ever escapes this crate's engine boundary.** The public
//!   surface exposes only Lens-owned values ([`Frame`], [`Presentation`],
//!   lifecycle).
//! - Exactly **one** typed inbound seam ([`TerminalHostEvent`] via
//!   [`TerminalTab::on_host_event`]) and **one** typed outbound stream
//!   ([`TerminalEvent`], emitted via gpui's [`gpui::EventEmitter`]).
//! - Accessors [`TerminalTab::focus_handle`] (host-driven focus, direct — not a
//!   callback) and [`TerminalTab::presentation`] (latest atomic
//!   title/lifecycle/access/progress).

use std::collections::{HashSet, VecDeque};
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use lens_client::WsOutbound;

mod bridge;
mod clipboard_policy;
mod engine;
mod hit_test;
mod input_gate;
mod inspect;
mod policy;
mod render;
mod runtime;

pub use input_gate::write_input_allowed;

/// Test-only view onto the private `render` module for the real-window harness
/// (`tests/render_realwindow.rs`). Gated on `test-util` because integration
/// tests link the crate's **normal** build (not `cfg(test)`); the harness runs
/// with `--features test-util`. Kept out of the default public API.
#[cfg(any(test, feature = "test-util"))]
pub mod render_test_api {
    pub use crate::render::fixtures::{
        ascii_frame, dense_wide_emoji_frame, mixed_ascii_wide_frame, pathological_wide_emoji_frame,
        sgr_frame,
    };
    pub use crate::render::metrics::{
        CellMetrics, MenloGateResult, menlo_gate_ok, per_row_alignment_ok,
    };
    pub use crate::render::paint::{RenderStats, paint_frame};
    pub use crate::render::state::TabRenderState;
}

/// Fixtures for Criterion `Frame`-construction benches (`bench` feature). Only
/// the builders — never `paint_frame` (I12: paint stays out of the public API).
#[cfg(feature = "bench")]
pub mod render_bench_api {
    pub use crate::render::fixtures::{ascii_frame, dense_wide_emoji_frame};
}

/// Engine input-path helpers for Criterion benches (`bench` feature).
#[cfg(feature = "bench")]
pub mod engine_bench_api {
    use crossbeam_channel::bounded;

    use crate::engine::command::{KeyAction, KeyInput, KeyMods, LensKey, MouseEventKind};
    use crate::engine::worker::EngineCommand;
    use crate::{EngineError, EngineHandle, FeedError, VtEngine};

    pub fn encode_arrow_up_press(engine: &mut VtEngine) -> Result<Vec<u8>, EngineError> {
        engine.encode_key(&KeyInput {
            action: KeyAction::Press,
            key: LensKey::ArrowUp,
            mods: KeyMods::default(),
            utf8: None,
            composing: false,
            access_epoch: 0,
            ack: None,
        })
    }

    pub fn encode_paste_bench(engine: &mut VtEngine, data: &[u8]) -> Result<Vec<u8>, EngineError> {
        engine.encode_paste(data)
    }

    pub fn encode_mouse_move_bench(
        engine: &mut VtEngine,
        px_x: f32,
        px_y: f32,
    ) -> Result<Vec<u8>, EngineError> {
        use crate::engine::command::MouseReportEv;
        engine.encode_mouse_report(&MouseReportEv {
            action: MouseEventKind::Move,
            button: None,
            wheel: None,
            mods: KeyMods::default(),
            px_x,
            px_y,
            any_button_pressed: false,
        })
    }

    pub fn feed_app_cursor_mode_then_arrow_up(handle: &EngineHandle) -> Result<(), FeedError> {
        handle.feed(b"\x1b[?1h".to_vec())?;
        let (ack_tx, ack_rx) = bounded(1);
        handle.enqueue_input(EngineCommand::Key(KeyInput {
            action: KeyAction::Press,
            key: LensKey::ArrowUp,
            mods: KeyMods::default(),
            utf8: None,
            composing: false,
            access_epoch: 0,
            ack: Some(ack_tx),
        }))?;
        ack_rx.recv().map_err(|_| FeedError::Stopped)?;
        Ok(())
    }
}

use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle, IntoElement,
    KeyDownEvent, KeyUpEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render,
    ScrollWheelEvent, Subscription, UTF16Selection, Window,
};
use lens_client::Client;
use lens_client::ids::{SessionId, TerminalId};
use lens_client::{AttachOptions, TerminalResource, attach};
use serde::{Deserialize, Serialize};

use bridge::{BridgeEvent, spawn_bridge};
pub use clipboard_policy::{ClipboardPolicy, SessionClipboardPolicy};
#[cfg(any(test, feature = "test-util"))]
use engine::command::InputAck;
use engine::command::{KeyAction, KeyInput, LensKey, ScrollDelta, WheelInput};
use engine::key_map::{gpui_mods_to_key_mods, keydown_should_enqueue, keystroke_to_lens};
use engine::worker::EngineCommand;
use policy::{
    AttachedParts, PolicyAction, PolicyState, discover_and_attach, identity_title_from_resource,
    preflight_reconnect,
};

// ---------------------------------------------------------------------------
// Public identity + access values (frozen names).
// ---------------------------------------------------------------------------

/// What the tab attaches to. Identity is separate from access ([`AccessIntent`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalTarget {
    /// Attach only to this exact named resource; never adopt a different
    /// resource or relaunch a process.
    Existing {
        session_id: SessionId,
        terminal_id: TerminalId,
    },
    /// Discover-or-create the exact logical key **during initial opening only**
    /// (not a perpetual keep-alive).
    OpenOrCreate {
        session_id: SessionId,
        key: TerminalKey,
    },
}

/// Logical terminal identity within a session. The server derives an opaque
/// [`TerminalId`] deterministically from `(terminal_name, session_key)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalKey {
    pub terminal_name: String,
    pub session_key: String,
}

/// Requested access. Server authorization remains authoritative — a caller may
/// force read-only but never *assert* authoritative write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessIntent {
    /// Prefer write for the owner, read-only for other viewers.
    Automatic,
    /// Force read-only regardless of ownership.
    ReadOnly,
}

/// Effective, server-resolved access — modeled **separately** from
/// [`Lifecycle`] because a `Live` tab can be write or read-only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessMode {
    Write,
    ReadOnly,
}

/// Open-time configuration. Holds **only** access intent, a scrollback limit,
/// and initial user preferences.
///
/// `#[non_exhaustive]` + [`Default`] + `with_*` setters so later slices can add
/// preference fields **without** breaking a `lens-ui` struct literal (external
/// crates construct via `TerminalOpenOptions::default().with_access(..)`, never
/// a field-literal). Fields evolve as later slices land — the *type name* and
/// this construction contract are what freeze.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TerminalOpenOptions {
    pub access: AccessIntent,
    /// Bounded scrollback cap in **lines** (`libghostty-vt` caps by line, not
    /// byte — see the design's "Scrollback, memory, resize"). `None` = engine
    /// default.
    pub scrollback_lines: Option<usize>,
    // Initial user preferences (mouse/paste/etc.) land with Slice 2.
}

impl Default for TerminalOpenOptions {
    fn default() -> Self {
        Self {
            access: AccessIntent::Automatic,
            scrollback_lines: None,
        }
    }
}

impl TerminalOpenOptions {
    /// Set the access intent.
    #[must_use]
    pub fn with_access(mut self, access: AccessIntent) -> Self {
        self.access = access;
        self
    }

    /// Set the scrollback cap in lines (`None` = engine default).
    #[must_use]
    pub fn with_scrollback_lines(mut self, lines: Option<usize>) -> Self {
        self.scrollback_lines = lines;
        self
    }
}

// ---------------------------------------------------------------------------
// Lifecycle (frozen: exactly these 7 variant names).
// ---------------------------------------------------------------------------

/// The tab's modeled lifecycle. The tab renders these values and **never
/// panics**; every failure path resolves to one of these variants.
///
/// A **pure, `Copy`, payload-free state tag** — the seven variants stay
/// payload-free **permanently**. Details that would otherwise ride a variant
/// (an `Ended` exit code, a `Detached` reason) live in the evolvable
/// [`Presentation`] snapshot instead, so adding one never rewrites a caller's
/// `match` arms or drops `Copy`. Cross-family review (2026-07-16) flagged that
/// unit variants plus `Copy` are incompatible with growing payloads, so we
/// commit to payload-free variants and route detail through `Presentation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lifecycle {
    /// Returned synchronously by [`open`]; discovery/attach in flight.
    Starting,
    /// Attached and streaming.
    Live,
    /// Transient transport loss; retained frame frozen while retrying.
    Reconnecting,
    /// Positively-identified reset; waiting to adopt the exact-key successor.
    ReplacementWaiting,
    /// Deliberate Sleep — engine + scrollback released, final viewport retained.
    Sleeping,
    /// Positively-reported process termination (may show an exit code).
    Ended,
    /// Terminal still exists elsewhere / identity changed / retries exhausted;
    /// an explicit user reattach or recreate is offered. Never a guessed `Ended`.
    Detached,
}

/// Why a tab entered [`Lifecycle::Detached`]. Presentation detail — not a lifecycle variant payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DetachedDetail {
    TerminalGone,
    ClientDetached,
    Unauthorized,
    RetriesExhausted,
    DiscoveryFailed,
    EngineStopped,
}

// ---------------------------------------------------------------------------
// Presentation snapshot (returned by `TerminalTab::presentation`).
// ---------------------------------------------------------------------------

/// Latest atomic presentation. `lens-ui` composes/truncates the visible title;
/// this carries the modeled inputs, not chrome.
#[derive(Clone, Debug)]
pub struct Presentation {
    pub lifecycle: Lifecycle,
    pub access: AccessMode,
    /// Stable routing/identity title = `terminal_name:session_key`. Never
    /// derived from terminal output.
    pub identity_title: String,
    /// Sanitized, bounded OSC 0/2 text. Optional, cosmetic, never
    /// identity/routing/authorization.
    pub reported_title: Option<String>,
    /// OSC progress — presentation state, not an OS side effect.
    pub progress: Option<Progress>,
    /// Set after a reconnect gap (Slice 1d policy); false at open.
    pub output_gap: bool,
    /// Set when a reconnect/downgrade dropped user input that had not been sent (C2).
    /// One-shot notice — cleared on the next accepted user keystroke. Cosmetic; never
    /// identity/authorization.
    pub input_discarded: bool, // rendered by presentation slice (2d+)
    /// Populated when lifecycle is [`Lifecycle::Detached`].
    pub detached_detail: Option<DetachedDetail>,
    /// Whether the host may offer an explicit reattach action.
    pub reattach_available: bool,
}

/// Terminal-local progress (OSC 9;4 style). Presentation only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub label: Option<String>,
    pub fraction: Option<f32>,
}

// ---------------------------------------------------------------------------
// The Frame — immutable render snapshot (the Send boundary).
// ---------------------------------------------------------------------------

pub use engine::frame::{CellStyle, Frame, FrameCell, FrameRow, Rgb, UnderlineStyle};
pub use engine::presentation::{ClipboardLocation, ClipboardMimePart};
pub use engine::{
    CursorPos, EgressFrame, EgressKind, EngineConfig, EngineError, EngineHandle, EngineInspect,
    FeedError, VtEngine,
};
pub use inspect::TerminalInspect;
pub use render::inspect::{RenderInspect, RenderInspectEvent, RenderInspectEventKind};

// ---------------------------------------------------------------------------
// Typed event seams (opaque; grow across slices — hence `#[non_exhaustive]`).
// ---------------------------------------------------------------------------

/// Opaque id for a host permission request (URL open, clipboard write, …).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HostRequestId(pub u64);

/// Host decision on a [`HostRequestId`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostRequestDecision {
    Allow,
    Deny,
    AllowSession,
    DenySession,
}

/// The single typed **inbound** seam: host → tab. Delivered via
/// [`TerminalTab::on_host_event`].
///
/// `#[non_exhaustive]` because the concrete set grows across slices (session
/// Sleep/wake/reset, `session.superseded`, normalized resource-generation
/// signals, preference changes, memory pressure, typed host-request responses).
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum TerminalHostEvent {
    /// Deliberate Sleep: release the engine + scrollback, retain final viewport.
    Sleep,
    /// Wake from Sleep: reattach if the same resource generation survived.
    Wake,
    /// Host response to a typed permission request emitted on [`TerminalEvent`].
    HostRequestResponse {
        id: HostRequestId,
        decision: HostRequestDecision,
    },
}

/// The single typed **outbound** stream: tab → host, via gpui's
/// [`gpui::EventEmitter`].
///
/// `#[non_exhaustive]` — presentation changes now; typed host requests
/// (user-gesture URL open, permissioned OSC 52 clipboard write, background
/// notifications) land with Slice 2.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum TerminalEvent {
    /// The tab's [`Presentation`] changed; host should re-read via
    /// [`TerminalTab::presentation`].
    PresentationChanged,
    /// User clicked a validated hyperlink; host may open the URL after policy.
    OpenUrlRequest { id: HostRequestId, url: String },
    /// Permissioned OSC 52 clipboard write; host responds via [`TerminalHostEvent`].
    ClipboardWriteRequest {
        id: HostRequestId,
        location: ClipboardLocation,
        contents: Vec<ClipboardMimePart>,
    },
    /// Emitted whenever a clipboard write is actually performed.
    ClipboardWriteNotice {
        location: ClipboardLocation,
        bytes: usize,
    },
    /// Multiline paste requires host confirmation before dispatch.
    PasteWarnRequest {
        id: HostRequestId,
        line_count: usize,
    },
}

// ---------------------------------------------------------------------------
// The tab entity.
// ---------------------------------------------------------------------------

/// A standalone, renderable GPUI terminal tab. Constructed via [`open`].
pub struct TerminalTab {
    focus_handle: FocusHandle,
    lifecycle: Lifecycle,
    presentation: Presentation,

    /// Full-snapshot render state (Slice 1c). Owns `latest_frame` + the shared
    /// canvas builder. Slice 1d makes the engine the source of `latest_frame`.
    render: render::state::TabRenderState,

    // Captured at `open()`; consumed by off-thread discovery/attach (Slice 1d).
    target: TerminalTarget,
    client: Arc<Client>,
    options: TerminalOpenOptions,

    runtime: Option<runtime::TerminalRuntime>,

    policy: PolicyState,
    policy_tx: Option<async_channel::Sender<BridgeEvent>>,
    current_session: Option<SessionId>,
    current_tid: Option<TerminalId>,

    /// Flipped by [`apply_newest_size_before_input`] on (re)connect; Slice 2 input
    /// gates on this.
    input_enabled: bool,
    /// Desired attach + engine inspect enablement; restored on reconnect.
    inspect_enabled: bool,
    /// IME composition preedit overlay (foreground-only; not in the PTY buffer).
    ime_preedit: Option<String>,
    /// Keys whose Press was enqueued — gates Release so suppressed presses never orphan.
    pressed_keys: HashSet<LensKey>,
    focus_in_sub: Option<Subscription>,
    focus_out_sub: Option<Subscription>,
    focus_subs_armed: bool,
    /// Monotonic id source for typed host permission requests.
    next_host_request_id: u64,
    clipboard_policy: Box<dyn ClipboardPolicy>,
    pending_clipboard_writes: VecDeque<(
        HostRequestId,
        engine::presentation::ClipboardLocation,
        Vec<engine::presentation::ClipboardMimePart>,
    )>,
    pending_pastes: VecDeque<(HostRequestId, Vec<u8>)>,
    /// Runtime mouse-local toggle (forces local selection over reporting). Slice 2c.
    mouse_local: bool,
    /// Report policy carried to the engine arbiter (Auto vs ForceLocal). Slice 2c.
    report_policy: engine::command::MouseReportPolicy,
    /// Monotonic base for MouseGesture.time multi-click derivation. Slice 2c.
    mouse_time_base: std::time::Instant,
    /// Per-click frame snapshots keyed by a foreground-minted click token. A `LocalClick`
    /// (hyperlink open) resolves its cell against the frame captured at ITS OWN Left-down
    /// (matched by the token the engine echoes), not the current frame — so intervening
    /// terminal output cannot repaint the cell and open an unclicked URL, and overlapping
    /// clicks each resolve against their own down-frame (codex F2 + re-review). Bounded
    /// ring: un-claimed entries (downs that became reports/drags) evict oldest-first.
    pending_click_frames: VecDeque<(u64, Arc<Frame>)>,
    /// Monotonic source for the Left-down click token.
    next_click_seq: u64,
}

const PENDING_HOST_REQUESTS_CAP: usize = 64;

/// Bound on retained per-click frame snapshots (F2). Only Left-downs that become a no-drag
/// local click claim theirs; un-claimed entries (reports/drags) evict oldest-first.
const CLICK_FRAME_CAP: usize = 16;

/// Maximum paste payload size (bytes). Over-cap pastes are rejected, never truncated.
pub const MAX_PASTE_BYTES: usize = 1 << 20;

impl TerminalTab {
    fn starting(
        target: TerminalTarget,
        client: Arc<Client>,
        options: TerminalOpenOptions,
        cx: &mut Context<Self>,
    ) -> Self {
        // Slice 1d kicks off off-thread discovery/attach here; Slice 0 only
        // freezes the "returns in Starting" invariant.
        Self {
            focus_handle: cx.focus_handle(),
            lifecycle: Lifecycle::Starting,
            presentation: starting_presentation(&target, &options),
            render: render::state::TabRenderState::new(),
            target,
            client,
            options,
            runtime: None,
            policy: PolicyState::new(),
            policy_tx: None,
            current_session: None,
            current_tid: None,
            input_enabled: false,
            inspect_enabled: false,
            ime_preedit: None,
            pressed_keys: HashSet::new(),
            focus_in_sub: None,
            focus_out_sub: None,
            focus_subs_armed: false,
            next_host_request_id: 0,
            clipboard_policy: Box::new(SessionClipboardPolicy::default()),
            pending_clipboard_writes: VecDeque::new(),
            pending_pastes: VecDeque::new(),
            mouse_local: false,
            report_policy: engine::command::MouseReportPolicy::Auto,
            mouse_time_base: std::time::Instant::now(),
            pending_click_frames: VecDeque::new(),
            next_click_seq: 0,
        }
    }

    #[cfg(any(test, feature = "test-util"))]
    fn with_engine_for_test(engine: Arc<EngineHandle>, cx: &mut Context<Self>) -> Self {
        let target = TerminalTarget::OpenOrCreate {
            session_id: SessionId::new("test_sess"),
            key: TerminalKey {
                terminal_name: "main".into(),
                session_key: "k".into(),
            },
        };
        let options = TerminalOpenOptions::default();
        Self {
            focus_handle: cx.focus_handle(),
            lifecycle: Lifecycle::Live,
            presentation: Presentation {
                lifecycle: Lifecycle::Live,
                access: AccessMode::Write,
                identity_title: identity_title_of(&target),
                reported_title: None,
                progress: None,
                output_gap: false,
                input_discarded: false,
                detached_detail: None,
                reattach_available: false,
            },
            render: render::state::TabRenderState::new(),
            target,
            client: Arc::new(Client::stub_for_test()),
            options,
            runtime: Some(runtime::TerminalRuntime {
                bridge: None,
                attach: None,
                engine: Some(engine),
            }),
            policy: PolicyState::new(),
            policy_tx: None,
            current_session: None,
            current_tid: None,
            input_enabled: true,
            inspect_enabled: false,
            ime_preedit: None,
            pressed_keys: HashSet::new(),
            focus_in_sub: None,
            focus_out_sub: None,
            focus_subs_armed: false,
            next_host_request_id: 0,
            clipboard_policy: Box::new(SessionClipboardPolicy::default()),
            pending_clipboard_writes: VecDeque::new(),
            pending_pastes: VecDeque::new(),
            mouse_local: false,
            report_policy: engine::command::MouseReportPolicy::Auto,
            mouse_time_base: std::time::Instant::now(),
            pending_click_frames: VecDeque::new(),
            next_click_seq: 0,
        }
    }

    /// Construct a writable [`TerminalTab`] backed by a test engine (harness only).
    #[cfg(any(test, feature = "test-util"))]
    pub fn open_with_engine_for_test(
        engine: Arc<EngineHandle>,
        cx: &mut App,
    ) -> Entity<TerminalTab> {
        cx.new(|cx| TerminalTab::with_engine_for_test(engine, cx))
    }

    /// Host-driven focus. Direct accessor, not a callback.
    pub fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }

    /// Latest atomic [`Presentation`] (title/lifecycle/access/progress).
    pub fn presentation(&self) -> Presentation {
        self.presentation.clone()
    }

    /// The single typed inbound seam.
    pub fn on_host_event(&mut self, event: TerminalHostEvent, cx: &mut Context<Self>) {
        match event {
            TerminalHostEvent::Sleep | TerminalHostEvent::Wake => {}
            TerminalHostEvent::HostRequestResponse { id, decision } => {
                let pos = self
                    .pending_clipboard_writes
                    .iter()
                    .position(|(pending_id, _, _)| *pending_id == id);
                if let Some(i) = pos {
                    let (_, location, contents) = self.pending_clipboard_writes.remove(i).unwrap();
                    match decision {
                        HostRequestDecision::Allow | HostRequestDecision::AllowSession => {
                            self.write_clipboard_contents(&location, &contents, cx);
                            if let Some(e) = self.engine_handle() {
                                e.record_clipboard_write_allowed();
                            }
                            if matches!(decision, HostRequestDecision::AllowSession) {
                                self.clipboard_policy
                                    .remember_osc52(location, HostRequestDecision::Allow);
                            }
                        }
                        HostRequestDecision::Deny | HostRequestDecision::DenySession => {
                            if let Some(e) = self.engine_handle() {
                                e.record_clipboard_write_denied();
                            }
                            if matches!(decision, HostRequestDecision::DenySession) {
                                self.clipboard_policy
                                    .remember_osc52(location, HostRequestDecision::Deny);
                            }
                        }
                    }
                }
                let paste_pos = self
                    .pending_pastes
                    .iter()
                    .position(|(pending_id, _)| *pending_id == id);
                if let Some(i) = paste_pos {
                    let (_, bytes) = self.pending_pastes.remove(i).unwrap();
                    match decision {
                        HostRequestDecision::Allow | HostRequestDecision::AllowSession => {
                            if matches!(decision, HostRequestDecision::AllowSession) {
                                self.clipboard_policy.suppress_paste_warn();
                            }
                            self.dispatch_paste(bytes, cx);
                        }
                        HostRequestDecision::Deny | HostRequestDecision::DenySession => {}
                    }
                }
            }
        }
    }

    /// Enable/disable the render Inspect ring (zero-cost when disabled).
    pub fn set_render_inspect_enabled(&self, enabled: bool) {
        self.render.inspect.set_enabled(enabled);
    }

    /// Snapshot the render Inspect state (paint counters + recent-paints ring).
    pub fn render_inspect(&self) -> RenderInspect {
        self.render.inspect.snapshot()
    }

    #[cfg(feature = "live-tests")]
    pub fn debug_send_input_for_test(&self, bytes: Vec<u8>) -> bool {
        if let Some(rt) = &self.runtime
            && let Some(a) = rt.attach_ref()
        {
            return a
                .outbound
                .try_send(lens_client::WsOutbound::Input(bytes))
                .is_ok();
        }
        false
    }

    #[cfg(feature = "live-tests")]
    pub fn debug_abort_attach_for_test(&mut self, cx: &mut Context<Self>) {
        // Take ONLY the attach (leave the bridge running so it observes Closed(Network)
        // and drives policy → Reconnecting). Abort off-foreground (abort_for_test joins the I/O thread).
        if let Some(rt) = &mut self.runtime
            && let Some(a) = rt.take_attach()
        {
            cx.background_executor()
                .spawn(async move {
                    a.abort_for_test();
                })
                .detach();
        }
    }

    #[cfg(feature = "live-tests")]
    pub fn debug_latest_frame_for_test(&self) -> Option<std::sync::Arc<Frame>> {
        self.render.latest_frame.clone()
    }

    /// Enable/disable attach + engine inspect rings (zero-cost when disabled).
    pub fn set_inspect_enabled(&mut self, enabled: bool) {
        self.inspect_enabled = enabled;
        if let Some(rt) = &self.runtime {
            if let Some(a) = rt.attach_ref() {
                a.set_inspect_enabled(enabled);
            }
            if let Some(e) = rt.engine_ref() {
                e.set_inspect_enabled(enabled);
            }
        }
    }

    /// Snapshot convergence inspect state (lifecycle, transport, engine).
    pub fn inspect(&self) -> TerminalInspect {
        let attach = self
            .runtime
            .as_ref()
            .and_then(|rt| rt.attach_ref().map(lens_client::AttachHandle::inspect));
        let engine = self
            .runtime
            .as_ref()
            .and_then(|rt| rt.engine_ref().map(|e| e.inspect()));
        let bridge_alive = self
            .runtime
            .as_ref()
            .is_some_and(runtime::TerminalRuntime::bridge_is_present);

        TerminalInspect {
            lifecycle: self.lifecycle,
            output_gap: self.presentation.output_gap,
            input_discarded: self.presentation.input_discarded,
            bridge_alive,
            input_enabled: self.input_enabled,
            attach,
            engine,
        }
    }

    /// Push a `Frame` into the render state (test/harness only — Slice 1d wires
    /// the engine wake sampler as the production source).
    #[cfg(any(test, feature = "test-util"))]
    pub fn set_frame_for_test(&mut self, frame: Arc<Frame>, cx: &mut Context<Self>) {
        self.render.set_frame(frame);
        cx.notify();
    }

    /// Currently-sampled frame (test/harness only). Reflects what mouse handlers
    /// would hit-test — i.e. the engine-sampled frame.
    #[cfg(any(test, feature = "test-util"))]
    pub fn latest_frame_for_test(&self) -> Option<Arc<Frame>> {
        self.render.latest_frame()
    }

    /// Last canvas paint origin (test/harness only).
    #[cfg(any(test, feature = "test-util"))]
    pub fn last_paint_origin_for_test(&self) -> Option<gpui::Point<Pixels>> {
        self.render.last_paint_origin()
    }

    /// Resolved cell metrics (test/harness only).
    #[cfg(any(test, feature = "test-util"))]
    pub fn cell_metrics_for_test(&self) -> Option<render::metrics::CellMetrics> {
        self.render.cell_metrics.clone()
    }

    /// Invoke the production left-click mouse-down path (harness only).
    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_mouse_down_for_test(
        &mut self,
        position: gpui::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_mouse_down(
            &MouseDownEvent {
                button: gpui::MouseButton::Left,
                position,
                ..Default::default()
            },
            window,
            cx,
        );
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_mouse_move_for_test(
        &mut self,
        position: gpui::Point<Pixels>,
        pressed_button: Option<gpui::MouseButton>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_mouse_move(
            &MouseMoveEvent {
                position,
                pressed_button,
                modifiers: Default::default(),
            },
            window,
            cx,
        );
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_mouse_up_for_test(
        &mut self,
        position: gpui::Point<Pixels>,
        button: gpui::MouseButton,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_mouse_up(
            &MouseUpEvent {
                button,
                position,
                modifiers: Default::default(),
                click_count: 1,
            },
            window,
            cx,
        );
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_mouse_up_out_for_test(
        &mut self,
        position: gpui::Point<Pixels>,
        button: gpui::MouseButton,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_mouse_up_out(
            &MouseUpEvent {
                button,
                position,
                modifiers: Default::default(),
                click_count: 1,
            },
            window,
            cx,
        );
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_wheel_for_test(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        self.handle_scroll_wheel(event, cx);
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_handle_copy_for_test(&mut self, cx: &mut Context<Self>) {
        self.handle_copy(cx);
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_select_all_for_test(&self) {
        if let Some(engine) = self.engine_handle() {
            let _ = engine.select_all();
        }
    }

    /// Drain presentation channel events (harness only).
    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_drain_presentation_for_test(&mut self, cx: &mut Context<Self>) {
        self.drain_presentation_events(cx);
    }

    /// Seed OSC 52 session policy (harness only).
    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_remember_osc52_for_test(
        &mut self,
        location: engine::presentation::ClipboardLocation,
        decision: HostRequestDecision,
    ) {
        self.clipboard_policy.remember_osc52(location, decision);
    }

    /// Pending clipboard host-request ids (harness only).
    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_pending_clipboard_ids_for_test(&self) -> Vec<HostRequestId> {
        self.pending_clipboard_writes
            .iter()
            .map(|(id, _, _)| *id)
            .collect()
    }

    /// Pending paste host-request ids (harness only).
    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_pending_paste_ids_for_test(&self) -> Vec<HostRequestId> {
        self.pending_pastes.iter().map(|(id, _)| *id).collect()
    }

    /// Paste-warn suppression flag (harness only).
    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_paste_warn_suppressed_for_test(&self) -> bool {
        self.clipboard_policy.paste_warn_suppressed()
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_mouse_local_for_test(&self) -> bool {
        self.mouse_local
    }

    fn lower_mouse_gesture(
        &mut self,
        kind: engine::command::MouseEventKind,
        button: Option<engine::command::MouseButtonKind>,
        position: gpui::Point<Pixels>,
        modifiers: &gpui::Modifiers,
        cell_override_none: bool,
        click_seq: u64,
    ) {
        let Some(frame) = self.render.latest_frame() else {
            return;
        };
        let Some(metrics) = self.render.cell_metrics.clone() else {
            return;
        };
        let Some(origin) = self.render.last_paint_origin() else {
            return;
        };
        let cell = if cell_override_none {
            None
        } else {
            hit_test::pixel_to_cell(origin, &metrics, position, frame.cols, frame.rows)
        };
        let px_x = f32::from(position.x - origin.x);
        let px_y = f32::from(position.y - origin.y);
        let Some(engine) = self.engine_handle() else {
            return;
        };
        let _ = engine.enqueue_mouse_gesture(engine::command::MouseGesture {
            kind,
            button,
            mods: gpui_mods_to_key_mods(modifiers),
            cell,
            px_x,
            px_y,
            time: self.mouse_time_base.elapsed(),
            mouse_local: self.mouse_local,
            policy: self.report_policy,
            click_seq,
            access_epoch: 0,
            ack: None,
        });
    }

    pub(crate) fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let button = gpui_button_to_kind(event.button);
        // Mint a click token and snapshot the click-time frame under it, so a later
        // LocalClick resolves its hyperlink against what was under the cursor at THIS press
        // (matched by token) — not a frame the terminal repainted since, and correctly even
        // with overlapping clicks (F2 + re-review).
        let click_seq = if button == Some(engine::command::MouseButtonKind::Left) {
            self.next_click_seq = self.next_click_seq.wrapping_add(1);
            let seq = self.next_click_seq;
            if let Some(frame) = self.render.latest_frame() {
                if self.pending_click_frames.len() >= CLICK_FRAME_CAP {
                    self.pending_click_frames.pop_front();
                }
                self.pending_click_frames.push_back((seq, frame));
            }
            seq
        } else {
            0
        };
        self.lower_mouse_gesture(
            engine::command::MouseEventKind::Down,
            button,
            event.position,
            &event.modifiers,
            false,
            click_seq,
        );
    }

    pub(crate) fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let button = event.pressed_button.and_then(gpui_button_to_kind);
        self.lower_mouse_gesture(
            engine::command::MouseEventKind::Move,
            button,
            event.position,
            &event.modifiers,
            false,
            0,
        );
    }

    pub(crate) fn handle_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let button = gpui_button_to_kind(event.button);
        self.lower_mouse_gesture(
            engine::command::MouseEventKind::Up,
            button,
            event.position,
            &event.modifiers,
            false,
            0,
        );
    }

    pub(crate) fn handle_mouse_up_out(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let button = gpui_button_to_kind(event.button);
        self.lower_mouse_gesture(
            engine::command::MouseEventKind::Up,
            button,
            event.position,
            &event.modifiers,
            true,
            0,
        );
    }

    /// Toggle mouse-local mode: forces local text selection instead of PTY mouse
    /// reporting for the NEXT gesture (the engine arbiter reads the flag carried on
    /// the command). Host UI/keybinding wiring lands with Slice 3.
    #[cfg_attr(not(test), expect(dead_code, reason = "host toggle wiring; Slice 3"))]
    pub(crate) fn toggle_mouse_local(&mut self, cx: &mut Context<Self>) {
        self.mouse_local = !self.mouse_local;
        cx.notify();
    }

    fn write_input_allowed(&self) -> bool {
        write_input_allowed(self.presentation.access, self.input_enabled)
    }

    fn ensure_focus_subscriptions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus_subs_armed {
            return;
        }
        self.focus_subs_armed = true;
        let focus_handle = self.focus_handle.clone();
        self.focus_in_sub = Some(cx.on_focus_in(&focus_handle, window, |this, _w, cx| {
            this.on_focus_changed(true, cx);
        }));
        self.focus_out_sub = Some(cx.on_blur(&focus_handle, window, |this, _w, cx| {
            this.on_focus_changed(false, cx);
        }));
    }

    fn on_focus_changed(&mut self, focused: bool, _cx: &mut Context<Self>) {
        let report = write_input_allowed(self.presentation.access, self.input_enabled);
        let Some(rt) = &self.runtime else {
            return;
        };
        let Some(engine) = &rt.engine else {
            return;
        };
        let _ = engine.enqueue_input(EngineCommand::Focus {
            focused,
            report,
            access_epoch: 0,
        });
    }

    pub(crate) fn handle_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _cx: &mut Context<Self>,
    ) {
        let Some(delta) = gpui_scroll_to_lens(event.delta) else {
            return;
        };
        let ScrollDelta::Lines(lines) = delta else {
            return;
        };
        let Some(engine) = self.engine_handle() else {
            return;
        };
        let (cell, px_x, px_y) = if let Some(frame) = self.render.latest_frame()
            && let Some(metrics) = self.render.cell_metrics.clone()
            && let Some(origin) = self.render.last_paint_origin()
        {
            let cell =
                hit_test::pixel_to_cell(origin, &metrics, event.position, frame.cols, frame.rows);
            let px_x = f32::from(event.position.x - origin.x);
            let px_y = f32::from(event.position.y - origin.y);
            (cell, px_x, px_y)
        } else {
            (None, 0.0, 0.0)
        };
        let _ = engine.enqueue_wheel(WheelInput {
            lines,
            cell,
            px_x,
            px_y,
            mods: gpui_mods_to_key_mods(&event.modifiers),
            access_epoch: 0,
            ack: None,
        });
    }

    fn clear_input_composition_state(&mut self) {
        self.ime_preedit = None;
        self.pressed_keys.clear();
    }

    fn is_paste_keystroke(ks: &gpui::Keystroke) -> bool {
        ks.modifiers.platform
            && !ks.modifiers.control
            && !ks.modifiers.alt
            && !ks.modifiers.function
            && ks.key == "v"
    }

    fn is_copy_keystroke(ks: &gpui::Keystroke) -> bool {
        ks.modifiers.platform
            && !ks.modifiers.control
            && !ks.modifiers.alt
            && !ks.modifiers.function
            && ks.key == "c"
    }

    fn is_select_all_keystroke(ks: &gpui::Keystroke) -> bool {
        ks.modifiers.platform
            && !ks.modifiers.control
            && !ks.modifiers.alt
            && !ks.modifiers.function
            && ks.key == "a"
    }

    const COPY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

    fn handle_copy(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.engine_handle() else {
            return;
        };
        let Ok(rx) = engine.request_copy() else {
            return;
        };
        cx.spawn(async move |weak, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { rx.recv_timeout(Self::COPY_TIMEOUT).ok() })
                .await;
            let _ = weak.update(cx, |_t, cx| {
                if let Some(engine::command::CopyResult { text: Some(t) }) = res
                    && !t.is_empty()
                {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(t));
                }
            });
        })
        .detach();
    }

    fn paste_needs_warn(text: &str, suppressed: bool) -> bool {
        !suppressed && text.contains('\n')
    }

    /// Special/modified keys only — plain printable text is owned by [`EntityInputHandler`].
    pub(crate) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ks = &event.keystroke;
        if Self::is_paste_keystroke(ks) {
            self.handle_paste(cx);
            cx.stop_propagation();
            return;
        }
        if Self::is_copy_keystroke(ks) {
            self.handle_copy(cx);
            cx.stop_propagation();
            return;
        }
        if Self::is_select_all_keystroke(ks) {
            if let Some(engine) = self.engine_handle() {
                let _ = engine.select_all();
            }
            cx.stop_propagation();
            return;
        }
        if !keydown_should_enqueue(&ks.key, &ks.modifiers) {
            return;
        }
        let lens_key = keystroke_to_lens(&ks.key);
        let action = if event.is_held {
            KeyAction::Repeat
        } else {
            KeyAction::Press
        };
        let enqueued = self.try_enqueue_key(KeyInput {
            action,
            key: lens_key,
            mods: gpui_mods_to_key_mods(&ks.modifiers),
            utf8: ks.key_char.clone(),
            composing: false,
            access_epoch: 0,
            ack: None,
        });
        if enqueued {
            if matches!(action, KeyAction::Press) {
                self.pressed_keys.insert(lens_key);
            }
            self.clear_input_discarded(cx);
            cx.stop_propagation();
        }
    }

    pub(crate) fn handle_key_up(
        &mut self,
        event: &KeyUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ks = &event.keystroke;
        if !keydown_should_enqueue(&ks.key, &ks.modifiers) {
            return;
        }
        let lens_key = keystroke_to_lens(&ks.key);
        if !self.pressed_keys.contains(&lens_key) {
            return;
        }
        let enqueued = self.try_enqueue_key(KeyInput {
            action: KeyAction::Release,
            key: lens_key,
            mods: gpui_mods_to_key_mods(&ks.modifiers),
            utf8: ks.key_char.clone(),
            composing: false,
            access_epoch: 0,
            ack: None,
        });
        if enqueued {
            self.pressed_keys.remove(&lens_key);
            cx.stop_propagation();
        }
    }

    fn try_enqueue_key(&mut self, input: KeyInput) -> bool {
        if !self.write_input_allowed() {
            return false;
        }
        let Some(rt) = &self.runtime else {
            return false;
        };
        let Some(engine) = &rt.engine else {
            return false;
        };
        engine.enqueue_input(EngineCommand::Key(input)).is_ok()
    }

    fn enqueue_committed_text(&mut self, text: &str) -> bool {
        if text.is_empty() || !self.write_input_allowed() {
            return false;
        }
        self.try_enqueue_key(KeyInput {
            action: KeyAction::Press,
            key: LensKey::Unidentified,
            mods: engine::command::KeyMods::default(),
            utf8: Some(text.to_owned()),
            composing: false,
            access_epoch: 0,
            ack: None,
        })
    }

    fn clear_input_discarded(&mut self, cx: &mut Context<Self>) {
        if self.presentation.input_discarded {
            self.presentation.input_discarded = false;
            cx.emit(TerminalEvent::PresentationChanged);
            cx.notify();
        }
    }

    /// Simulate keydown for harness tests; returns whether a key command was enqueued.
    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_key_down_for_test(&mut self, keystroke: gpui::Keystroke, is_held: bool) -> bool {
        if Self::is_paste_keystroke(&keystroke) {
            return false;
        }
        if !keydown_should_enqueue(&keystroke.key, &keystroke.modifiers) {
            return false;
        }
        if !self.write_input_allowed() {
            return false;
        }
        let action = if is_held {
            KeyAction::Repeat
        } else {
            KeyAction::Press
        };
        self.try_enqueue_key(KeyInput {
            action,
            key: keystroke_to_lens(&keystroke.key),
            mods: gpui_mods_to_key_mods(&keystroke.modifiers),
            utf8: keystroke.key_char.clone(),
            composing: false,
            access_epoch: 0,
            ack: None,
        })
    }

    /// Invoke the production keydown handler (real-window harness).
    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_handle_key_down_for_test(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_key_down(event, window, cx);
    }

    /// Invoke the production paste path (harness only).
    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_paste_for_test(&mut self, cx: &mut Context<Self>) {
        self.handle_paste(cx);
    }

    /// Invoke the production keyup handler (real-window harness).
    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_handle_key_up_for_test(
        &mut self,
        event: &KeyUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_key_up(event, window, cx);
    }

    /// Mirror [`EntityInputHandler::replace_text_in_range`] commit path (harness only).
    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_input_handler_text_for_test(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.ime_preedit = None;
        let _ = self.enqueue_committed_text(text);
    }

    /// Enqueue committed IME/text bytes and return the ACK receiver (never blocks inside).
    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_ime_commit_for_test(
        &mut self,
        text: &str,
    ) -> Option<crossbeam_channel::Receiver<InputAck>> {
        self.ime_preedit = None;
        if text.is_empty() || !self.write_input_allowed() {
            return None;
        }
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        let enqueued = self.try_enqueue_key(KeyInput {
            action: KeyAction::Press,
            key: LensKey::Unidentified,
            mods: engine::command::KeyMods::default(),
            utf8: Some(text.to_owned()),
            composing: false,
            access_epoch: 0,
            ack: Some(ack_tx),
        });
        enqueued.then_some(ack_rx)
    }

    fn handle_paste(&mut self, cx: &mut Context<Self>) {
        if !self.write_input_allowed() {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        // Cap BEFORE the warn/pending branch so an over-cap payload is never stashed in
        // pending_pastes (a count-capped-but-not-byte-capped leak otherwise; finding I1).
        if text.len() > MAX_PASTE_BYTES {
            self.reject_over_cap_paste(cx);
            return;
        }
        if Self::paste_needs_warn(&text, self.clipboard_policy.paste_warn_suppressed()) {
            let line_count = text.lines().count();
            let id = HostRequestId(self.next_host_request_id);
            self.next_host_request_id = self.next_host_request_id.wrapping_add(1);
            if self.pending_pastes.len() >= PENDING_HOST_REQUESTS_CAP {
                self.pending_pastes.pop_front();
            }
            self.pending_pastes.push_back((id, text.into_bytes()));
            if let Some(e) = self.engine_handle() {
                e.record_paste_warn_prompt();
            }
            cx.emit(TerminalEvent::PasteWarnRequest { id, line_count });
            return;
        }
        self.dispatch_paste(text.into_bytes(), cx);
    }

    /// Visible reject-with-marker for an over-cap paste (never silent truncation, DP5).
    fn reject_over_cap_paste(&mut self, cx: &mut Context<Self>) {
        if let Some(e) = self.engine_handle() {
            e.record_paste_over_cap_reject();
        }
        if !self.presentation.input_discarded {
            self.presentation.input_discarded = true;
            cx.emit(TerminalEvent::PresentationChanged);
            cx.notify();
        }
    }

    fn dispatch_paste(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if !self.write_input_allowed() {
            return;
        }
        if bytes.len() > MAX_PASTE_BYTES {
            self.reject_over_cap_paste(cx);
            return;
        }
        let Some(engine) = self.engine_handle() else {
            return;
        };
        let ok = engine
            .enqueue_input(EngineCommand::Paste(engine::command::PasteInput {
                bytes,
                access_epoch: 0,
                ack: None,
            }))
            .is_ok();
        if ok {
            if let Some(e) = self.engine_handle() {
                e.record_paste_sent();
            }
            self.clear_input_discarded(cx);
        }
    }

    fn on_attached(&mut self, parts: AttachedParts, cx: &mut Context<Self>) {
        // TODO(1b follow-up): EngineHandle::spawn should return a readiness result
        // so init failure never flashes Live.
        let AttachedParts {
            resource,
            runtime,
            wake_tx,
            wake_rx,
            policy_rx,
            policy_tx,
        } = parts;

        if let Some(engine) = runtime.engine.as_ref() {
            engine.set_waker(Box::new(move || {
                let _ = wake_tx.try_send(());
            }));
        }

        let outbound = runtime
            .attach_ref()
            .expect("attached runtime has attach")
            .outbound
            .clone();
        let engine = runtime.engine_arc().expect("attached runtime has engine");

        self.current_session = Some(resource.session_id.clone());
        self.current_tid = Some(resource.id.clone());
        self.policy_tx = Some(policy_tx);
        self.runtime = Some(runtime);
        self.lifecycle = Lifecycle::Live;
        self.presentation.lifecycle = Lifecycle::Live;
        self.presentation.access = access_mode_for(&self.options);
        self.presentation.identity_title = identity_title_from_resource(&resource);

        let snap = engine.inspect();
        let write_allowed = !matches!(self.presentation.access, AccessMode::ReadOnly);
        apply_newest_size_before_input(
            engine.as_ref(),
            &outbound,
            snap.cols,
            snap.rows,
            write_allowed,
            &mut self.input_enabled,
        );

        cx.emit(TerminalEvent::PresentationChanged);
        cx.notify();

        spawn_foreground_sampler(wake_rx, policy_rx, cx);
    }

    fn teardown_transport_off_foreground(&mut self, cx: &mut Context<Self>) {
        if let Some(rt) = &mut self.runtime {
            // Revoke any input still queued upstream (forwarder/cmd_tx): it belongs to the
            // connection being torn down and must never be encoded onto the next one.
            // Per-transport channels isolate already-emitted residue; this closes the
            // un-encoded-residue path (C2). Covers Retry AND downgrade.
            if let Some(engine) = rt.engine_ref() {
                engine.bump_access_epoch();
                // Engine-authoritative access: revoke report authority at the ordered
                // stream position NOW. The epoch bump only suppresses gestures queued
                // BEFORE teardown; a mouse gesture enqueued during the reconnect/detached
                // window carries the freshly-bumped epoch and would pass the epoch check,
                // so without this the engine's stale `write_allowed == true` would leak
                // reports onto the next connection's egress. Every teardown transitions to
                // a non-writable reconnecting/detached state; the next successful attach
                // re-sends the correct value via `apply_newest_size_before_input`.
                // (Closes read-only report leak — codex T5 review, CRITICAL.)
                let _ = engine.enqueue_set_access(false);
            }
            let (bridge, attach) = rt.take_transport();
            // Signal the outgoing bridge to stop SYNCHRONOUSLY, before the (detached,
            // possibly-delayed) join and before the next connection can attach its
            // egress. This sets the bridge's `stop` flag so a subsequent egress-sender
            // drop is recognised as teardown (suppressed, not a false EngineStopped =
            // Critical 1, closed by construction).
            //
            // C2 (reply-source) invariant — NOT closed by construction here: signal_stop
            // does not wait for the bridge to stop feeding. It is safe today only because
            // every teardown trigger is a bridge self-stop event (LoopExit::Stop) — the
            // outgoing bridge has already left its loop before we reach here, so it
            // cannot feed the engine after the next connection's egress attaches. A
            // future teardown path that fires while the bridge is still live (e.g. a
            // server-driven downgrade wired through on_host_event) MUST add
            // join-before-attach (or SetEgress(None) fencing) to keep C2 closed.
            if let Some(b) = &bridge {
                b.signal_stop();
            }
            cx.spawn(async move |_weak, cx| {
                cx.background_executor()
                    .spawn(async move {
                        if let Some(b) = bridge {
                            b.join();
                        }
                        if let Some(a) = attach {
                            a.close();
                        }
                    })
                    .await;
            })
            .detach();
        }
    }

    fn teardown_runtime_full(&mut self, cx: &mut Context<Self>) {
        if let Some(rt) = self.runtime.take() {
            cx.spawn(async move |_weak, cx| {
                cx.background_executor()
                    .spawn(async move {
                        rt.teardown_blocking();
                    })
                    .await;
            })
            .detach();
        }
    }

    fn set_detached_presentation(&mut self, detail: DetachedDetail, reattach_available: bool) {
        self.lifecycle = Lifecycle::Detached;
        self.presentation.lifecycle = Lifecycle::Detached;
        self.presentation.detached_detail = Some(detail);
        self.presentation.reattach_available = reattach_available;
        self.input_enabled = false;
    }

    fn on_detach(&mut self, detail: DetachedDetail, cx: &mut Context<Self>) {
        let reattach_available = matches!(detail, DetachedDetail::ClientDetached);
        self.clear_input_composition_state();
        self.teardown_runtime_full(cx);
        self.set_detached_presentation(detail, reattach_available);
        cx.emit(TerminalEvent::PresentationChanged);
        cx.notify();
    }

    fn sample_latest_frame_from_engine(&mut self) {
        if let Some(rt) = &self.runtime
            && let Some(engine) = &rt.engine
            && let Some(f) = engine.latest_frame()
        {
            self.render.set_frame(f);
        }
    }

    fn drain_presentation_events(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.runtime.as_ref().and_then(|r| r.engine.as_ref()) else {
            return;
        };
        let slot_update = engine.take_latest_title();
        let mut channel_events = Vec::new();
        while let Ok(ev) = engine.presentation_rx().try_recv() {
            channel_events.push(ev);
        }
        let result = engine::presentation::collect_presentation_drain(slot_update, channel_events);
        for url in &result.validated_hyperlink_urls {
            let id = HostRequestId(self.next_host_request_id);
            self.next_host_request_id = self.next_host_request_id.wrapping_add(1);
            cx.emit(TerminalEvent::OpenUrlRequest {
                id,
                url: url.clone(),
            });
        }
        for (col, row, seq) in &result.local_clicks {
            // Resolve against the frame captured at THIS click's own Left-down (matched by
            // token), so intervening output can't repaint the cell and open an unclicked
            // URL, and overlapping clicks don't cross frames (F2). Fall back to the current
            // frame only if the down was never snapshotted (e.g. click before first paint).
            // Inlined field access (not a &mut self method) so it coexists with `engine`.
            let click_frame = claim_click_frame(&mut self.pending_click_frames, *seq)
                .or_else(|| self.render.latest_frame());
            if let Some(frame) = click_frame
                && let Some(url) = hit_test::uri_for_gesture(frame.as_ref(), *col, *row)
            {
                let id = HostRequestId(self.next_host_request_id);
                self.next_host_request_id = self.next_host_request_id.wrapping_add(1);
                cx.emit(TerminalEvent::OpenUrlRequest { id, url });
            }
        }
        match &result.title_outcome {
            engine::presentation::TitleDrainOutcome::Set(title) => {
                apply_title_to_presentation(&mut self.presentation, title.clone());
                cx.emit(TerminalEvent::PresentationChanged);
            }
            engine::presentation::TitleDrainOutcome::Clear => {
                apply_title_to_presentation(&mut self.presentation, String::new());
                cx.emit(TerminalEvent::PresentationChanged);
            }
            engine::presentation::TitleDrainOutcome::NoChange => {}
        }
        engine.record_presentation_drain_inspect(&result);
        for (location, contents) in result.clipboard_writes {
            match self.clipboard_policy.osc52_session_decision(&location) {
                Some(HostRequestDecision::Allow | HostRequestDecision::AllowSession) => {
                    self.write_clipboard_contents(&location, &contents, cx);
                    if let Some(e) = self.engine_handle() {
                        e.record_clipboard_write_allowed();
                    }
                }
                Some(HostRequestDecision::Deny | HostRequestDecision::DenySession) => {
                    if let Some(e) = self.engine_handle() {
                        e.record_clipboard_write_denied();
                    }
                }
                None => {
                    let id = HostRequestId(self.next_host_request_id);
                    self.next_host_request_id = self.next_host_request_id.wrapping_add(1);
                    if self.pending_clipboard_writes.len() >= PENDING_HOST_REQUESTS_CAP {
                        self.pending_clipboard_writes.pop_front();
                    }
                    self.pending_clipboard_writes.push_back((
                        id,
                        location.clone(),
                        contents.clone(),
                    ));
                    cx.emit(TerminalEvent::ClipboardWriteRequest {
                        id,
                        location,
                        contents,
                    });
                }
            }
        }
    }

    fn engine_handle(&self) -> Option<&EngineHandle> {
        self.runtime.as_ref()?.engine.as_deref()
    }

    fn write_clipboard_contents(
        &self,
        location: &engine::presentation::ClipboardLocation,
        contents: &[engine::presentation::ClipboardMimePart],
        cx: &mut Context<Self>,
    ) {
        if contents.is_empty() {
            return;
        }
        let Some(part) = contents
            .iter()
            .find(|p| p.mime == "text/plain")
            .or_else(|| contents.first())
        else {
            return;
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(part.data.clone()));
        let bytes: usize = contents.iter().map(|p| p.data.len()).sum();
        cx.emit(TerminalEvent::ClipboardWriteNotice {
            location: location.clone(),
            bytes,
        });
    }

    /// Forward visibility to the engine worker; repaint when shown.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "host visibility; Slice 2 host wiring")
    )]
    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if let Some(rt) = &self.runtime
            && let Some(engine) = &rt.engine
        {
            let _ = engine.set_visible(visible);
        }
        if visible {
            cx.notify();
        }
    }

    fn apply_bridge_event(&mut self, ev: BridgeEvent, cx: &mut Context<Self>) {
        use lens_client::CloseCause;

        let action = match ev {
            BridgeEvent::Closed(cause) => self.policy.on_close(cause, Instant::now()),
            BridgeEvent::EngineStopped => PolicyAction::StopDetached {
                detail: DetachedDetail::EngineStopped,
                reattach_available: false,
            },
            BridgeEvent::FeedSaturated
            | BridgeEvent::OutboundSaturated
            | BridgeEvent::AttachDisconnected => {
                self.policy.on_close(CloseCause::Network, Instant::now())
            }
            BridgeEvent::StaleInputDiscarded => {
                self.presentation.input_discarded = true;
                cx.emit(TerminalEvent::PresentationChanged);
                cx.notify();
                return;
            }
        };

        match action {
            PolicyAction::StopDetached {
                detail,
                reattach_available,
            } => {
                self.clear_input_composition_state();
                if reattach_available {
                    self.teardown_transport_off_foreground(cx);
                } else {
                    self.teardown_runtime_full(cx);
                }
                self.set_detached_presentation(detail, reattach_available);
                cx.emit(TerminalEvent::PresentationChanged);
                cx.notify();
            }
            PolicyAction::DowngradeReadOnly => {
                let was_write = matches!(self.presentation.access, AccessMode::Write);
                if was_write {
                    if let Some(rt) = &self.runtime
                        && let Some(engine) = &rt.engine
                    {
                        engine.bump_access_epoch();
                    }
                    self.clear_input_composition_state();
                }
                self.presentation.access = AccessMode::ReadOnly;
                self.input_enabled = false;
                self.teardown_transport_off_foreground(cx);
                self.policy.retry.reset();
                self.lifecycle = Lifecycle::Reconnecting;
                self.presentation.lifecycle = Lifecycle::Reconnecting;
                cx.emit(TerminalEvent::PresentationChanged);
                cx.notify();
                self.schedule_reconnect(Duration::ZERO, cx);
            }
            PolicyAction::Retry { delay } => {
                self.clear_input_composition_state();
                self.lifecycle = Lifecycle::Reconnecting;
                self.presentation.lifecycle = Lifecycle::Reconnecting;
                self.input_enabled = false;
                cx.emit(TerminalEvent::PresentationChanged);
                cx.notify();
                self.teardown_transport_off_foreground(cx);
                self.schedule_reconnect(delay, cx);
            }
        }
    }

    fn reconnect_session_and_tid(&self) -> (SessionId, TerminalId) {
        match &self.target {
            TerminalTarget::Existing {
                session_id,
                terminal_id,
            } => (session_id.clone(), terminal_id.clone()),
            TerminalTarget::OpenOrCreate { .. } => (
                self.current_session
                    .clone()
                    .expect("current_session set at attach"),
                self.current_tid.clone().expect("current_tid set at attach"),
            ),
        }
    }

    fn schedule_reconnect(&mut self, first_delay: Duration, cx: &mut Context<Self>) {
        let client = Arc::clone(&self.client);
        let (session, tid) = self.reconnect_session_and_tid();
        let read_only = matches!(self.presentation.access, AccessMode::ReadOnly);

        cx.spawn(async move |weak, cx| {
            let mut first = Some(first_delay);
            loop {
                let delay = if let Some(d) = first.take() {
                    d
                } else {
                    match weak.update(cx, |tab, _| tab.policy.retry.next_delay(Instant::now())) {
                        Ok(Some(d)) => d,
                        Ok(None) => {
                            let _ = weak.update(cx, |tab, cx| {
                                tab.on_detach(DetachedDetail::RetriesExhausted, cx);
                            });
                            break;
                        }
                        Err(_) => break,
                    }
                };

                let attempt = cx
                    .background_executor()
                    .spawn({
                        let client = Arc::clone(&client);
                        let session = session.clone();
                        let tid = tid.clone();
                        async move {
                            std::thread::sleep(delay);
                            let resource =
                                match preflight_reconnect(client.as_ref(), &session, &tid) {
                                    Ok(r) => r,
                                    Err(DetachedDetail::TerminalGone) => {
                                        return Err(ReconnectOutcome::Fatal(
                                            DetachedDetail::TerminalGone,
                                        ));
                                    }
                                    Err(DetachedDetail::Unauthorized) => {
                                        return Err(ReconnectOutcome::Fatal(
                                            DetachedDetail::Unauthorized,
                                        ));
                                    }
                                    Err(_) => return Err(ReconnectOutcome::Retryable),
                                };
                            let attach = attach(
                                client.as_ref(),
                                &session,
                                &tid,
                                AttachOptions { read_only },
                            )
                            .map_err(|_| ReconnectOutcome::Retryable)?;
                            Ok::<_, ReconnectOutcome>((resource, attach))
                        }
                    })
                    .await;

                match attempt {
                    Ok((resource, attach)) => {
                        let _ = weak.update(cx, |tab, cx| {
                            tab.on_reconnect_success(resource, attach, cx);
                        });
                        break;
                    }
                    Err(ReconnectOutcome::Fatal(detail)) => {
                        let _ = weak.update(cx, |tab, cx| tab.on_detach(detail, cx));
                        break;
                    }
                    Err(ReconnectOutcome::Retryable) => {}
                }
            }
        })
        .detach();
    }

    fn on_reconnect_success(
        &mut self,
        resource: TerminalResource,
        attach: lens_client::AttachHandle,
        cx: &mut Context<Self>,
    ) {
        let read_only = matches!(self.presentation.access, AccessMode::ReadOnly);
        let write_allowed = !read_only;

        let Some(rt) = &mut self.runtime else {
            // Unreachable in the current design (no events arrive during the reconnect
            // window), but NEVER drop an AttachHandle on the gpui foreground — its Drop
            // joins the I/O thread synchronously.
            cx.spawn(async move |_w, cx| {
                cx.background_executor()
                    .spawn(async move {
                        attach.close();
                    })
                    .await;
            })
            .detach();
            return;
        };

        let outbound = attach.outbound.clone();
        let engine = rt.engine_arc().expect("engine retained during reconnect");
        let snap = engine.inspect();
        let (egress_tx, egress_rx) = crossbeam_channel::bounded(engine::worker::EGRESS_CHANNEL_CAP);
        if engine.attach_egress(egress_tx).is_err() {
            cx.spawn(async move |_w, cx| {
                cx.background_executor()
                    .spawn(async move {
                        attach.close();
                    })
                    .await;
            })
            .detach();
            // Near-unreachable cmd_tx-full defensive path; consumes retry budget.
            match self.policy.retry.next_delay(Instant::now()) {
                Some(delay) => self.schedule_reconnect(delay, cx),
                None => self.on_detach(DetachedDetail::RetriesExhausted, cx),
            }
            return;
        }
        let bridge = spawn_bridge(
            attach.inbound.clone(),
            attach.outbound.clone(),
            Arc::clone(&engine),
            self.policy_tx.clone().expect("policy_tx retained"),
            egress_rx,
        );
        rt.install_transport(bridge, attach);
        if self.inspect_enabled {
            if let Some(a) = rt.attach_ref() {
                a.set_inspect_enabled(true);
            }
            if let Some(e) = rt.engine_ref() {
                e.set_inspect_enabled(true);
            }
        }
        self.policy.retry.reset();
        apply_newest_size_before_input(
            engine.as_ref(),
            &outbound,
            snap.cols,
            snap.rows,
            write_allowed,
            &mut self.input_enabled,
        );

        self.current_session = Some(resource.session_id.clone());
        self.current_tid = Some(resource.id.clone());
        self.lifecycle = Lifecycle::Live;
        self.presentation.lifecycle = Lifecycle::Live;
        self.presentation.output_gap = true;
        self.presentation.identity_title = identity_title_from_resource(&resource);
        cx.emit(TerminalEvent::PresentationChanged);
        cx.notify();
    }
}

enum ReconnectOutcome {
    Fatal(DetachedDetail),
    Retryable,
}

fn access_mode_for(options: &TerminalOpenOptions) -> AccessMode {
    match options.access {
        AccessIntent::ReadOnly => AccessMode::ReadOnly,
        AccessIntent::Automatic => AccessMode::Write,
    }
}

fn spawn_foreground_sampler(
    wake_rx: async_channel::Receiver<()>,
    policy_rx: async_channel::Receiver<BridgeEvent>,
    cx: &mut Context<TerminalTab>,
) {
    use futures::FutureExt;

    cx.spawn(async move |weak, cx| {
        loop {
            futures::select! {
                r = wake_rx.recv().fuse() => {
                    match r {
                        Ok(()) => {
                            while wake_rx.try_recv().is_ok() {}
                            if weak
                                .update(cx, |tab, cx| {
                                    tab.sample_latest_frame_from_engine();
                                    tab.drain_presentation_events(cx);
                                    cx.notify();
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                ev = policy_rx.recv().fuse() => {
                    match ev {
                        Ok(ev) => {
                            if weak
                                .update(cx, |tab, cx| {
                                    tab.apply_bridge_event(ev, cx);
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    })
    .detach();
}

impl EventEmitter<TerminalEvent> for TerminalTab {}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

impl EntityInputHandler for TerminalTab {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        None
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.ime_preedit.as_ref().map(|text| 0..utf16_len(text))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.ime_preedit.take().is_some() {
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if text.is_empty() {
            return;
        }
        self.ime_preedit = None;
        if self.enqueue_committed_text(text) {
            self.clear_input_discarded(cx);
            cx.notify();
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime_preedit = if new_text.is_empty() {
            None
        } else {
            Some(new_text.to_owned())
        };
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // `element_bounds` is already the cursor cell (see `default_ime_bounds` registration).
        let metrics = self.render.cell_metrics.as_ref()?;
        Some(Bounds::new(
            element_bounds.origin,
            gpui::size(metrics.cell_w, metrics.cell_h),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for TerminalTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_focus_subscriptions(window, cx);
        self.sample_latest_frame_from_engine();
        self.drain_presentation_events(cx);
        // The one shared canvas builder (I6). No frame yet → modeled
        // placeholder (`identity — lifecycle`); a frame → full-snapshot paint.
        // Never panics.
        let title = self.presentation.identity_title.clone();
        let life = format!("{:?}", self.lifecycle);
        let tab = cx.entity().clone();
        self.render.render_element(
            &self.focus_handle,
            &title,
            &life,
            Some((self.ime_preedit.as_deref(), tab)),
            window,
            cx,
        )
    }
}

// ---------------------------------------------------------------------------
// Constructor.
// ---------------------------------------------------------------------------

/// Open a terminal tab. Returns immediately in [`Lifecycle::Starting`];
/// discovery/create/attach run off-thread and resolve into lifecycle values.
pub fn open(
    target: TerminalTarget,
    client: Arc<Client>,
    options: TerminalOpenOptions,
    cx: &mut App,
) -> Entity<TerminalTab> {
    let entity = cx
        .new(|cx| TerminalTab::starting(target.clone(), Arc::clone(&client), options.clone(), cx));
    let weak = entity.downgrade();
    cx.spawn(async move |cx| {
        let outcome = cx
            .background_executor()
            .spawn(async move { discover_and_attach(client, target, options) })
            .await;
        let _ = weak.update(cx, |tab, cx| match outcome {
            Ok(parts) => tab.on_attached(parts, cx),
            Err(detail) => tab.on_detach(detail, cx),
        });
    })
    .detach();
    entity
}

fn apply_title_to_presentation(presentation: &mut Presentation, title: String) {
    if title.is_empty() {
        presentation.reported_title = None;
    } else {
        presentation.reported_title = Some(title);
    }
    // NEVER touch presentation.identity_title here.
}

fn starting_presentation(target: &TerminalTarget, options: &TerminalOpenOptions) -> Presentation {
    Presentation {
        lifecycle: Lifecycle::Starting,
        access: match options.access {
            AccessIntent::ReadOnly => AccessMode::ReadOnly,
            // `Automatic` resolves to the server-authoritative mode once
            // attached; before attach it presents read-only.
            AccessIntent::Automatic => AccessMode::ReadOnly,
        },
        identity_title: identity_title_of(target),
        reported_title: None,
        progress: None,
        output_gap: false,
        input_discarded: false,
        detached_detail: None,
        reattach_available: false,
    }
}

/// Claim the click-time frame for a `LocalClick` token, dropping it and any older un-claimed
/// snapshots (downs that became reports/drags never emit a LocalClick, so entries preceding
/// the claimed one can never be claimed and are pruned). Free fn so it borrows only the deque
/// field (coexists with the `engine` borrow in `drain_presentation_events`).
fn claim_click_frame(frames: &mut VecDeque<(u64, Arc<Frame>)>, seq: u64) -> Option<Arc<Frame>> {
    let pos = frames.iter().position(|(s, _)| *s == seq)?;
    frames.drain(..pos);
    frames.pop_front().map(|(_, frame)| frame)
}

fn gpui_button_to_kind(b: gpui::MouseButton) -> Option<engine::command::MouseButtonKind> {
    use engine::command::MouseButtonKind;
    match b {
        gpui::MouseButton::Left => Some(MouseButtonKind::Left),
        gpui::MouseButton::Right => Some(MouseButtonKind::Right),
        gpui::MouseButton::Middle => Some(MouseButtonKind::Middle),
        _ => None,
    }
}

fn gpui_scroll_to_lens(delta: gpui::ScrollDelta) -> Option<ScrollDelta> {
    match delta {
        gpui::ScrollDelta::Lines(p) => {
            let lines = p.y.round() as i32;
            if lines == 0 {
                None
            } else {
                // saturating_neg guards against `-i32::MIN` overflow (panics in debug) for
                // a pathological scroll delta (codex whole-slice F3).
                Some(ScrollDelta::Lines(lines.saturating_neg()))
            }
        }
        gpui::ScrollDelta::Pixels(p) => {
            let lines = (-p.y / Pixels::from(16.0)).round() as i32;
            if lines == 0 {
                None
            } else {
                Some(ScrollDelta::Lines(lines))
            }
        }
    }
}

fn apply_newest_size_before_input(
    engine: &EngineHandle,
    outbound: &Sender<WsOutbound>,
    cols: u16,
    rows: u16,
    write_allowed: bool,
    input_enabled: &mut bool,
) {
    let _ = engine.resize(cols, rows);
    let _ = outbound.try_send(WsOutbound::Resize { cols, rows });
    *input_enabled = write_allowed;
    let _ = engine.enqueue_set_access(write_allowed);
}

fn identity_title_of(target: &TerminalTarget) -> String {
    match target {
        TerminalTarget::OpenOrCreate { key, .. } => {
            format!("{}:{}", key.terminal_name, key.session_key)
        }
        // Pre-GET we only hold the opaque id; the stable `name:key` title is
        // filled in once the resource is fetched (Slice 1d).
        TerminalTarget::Existing { terminal_id, .. } => terminal_id.to_string(),
    }
}

// Slice 0 tests exercise only the **offline** surface. `open()` requires an
// already-handshaked `Arc<Client>` (`Client::new` does a live health→version→
// info handshake), so the "open returns in `Starting`" seam invariant gets its
// proof in the Slice 1d live vertical test, not here. What *is* offline-testable
// — title derivation, defaults, and the frozen values' serde round-trips — is
// covered below.
#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::thread;
    use std::time::{Duration, Instant};

    use lens_client::WsOutbound;

    use super::*;
    use crate::engine::vt::EngineConfig;

    fn test_cfg() -> EngineConfig {
        EngineConfig {
            cols: 20,
            rows: 3,
            max_scrollback: 100,
            cell_w_px: 8,
            cell_h_px: 16,
        }
    }

    #[test]
    fn apply_title_event_updates_reported_only() {
        let mut presentation = Presentation {
            lifecycle: Lifecycle::Live,
            access: AccessMode::Write,
            identity_title: "main:workspace".into(),
            reported_title: None,
            progress: None,
            output_gap: false,
            input_discarded: false,
            detached_detail: None,
            reattach_available: false,
        };
        apply_title_to_presentation(&mut presentation, "Shell Title".into());
        assert_eq!(presentation.identity_title, "main:workspace");
        assert_eq!(presentation.reported_title.as_deref(), Some("Shell Title"));
    }

    fn wait_for_engine_frame(engine: &EngineHandle) -> Arc<Frame> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Some(f) = engine.latest_frame() {
                return f;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("timeout waiting for engine frame");
    }

    fn wait_for_wake_count(at_least: usize, counter: &AtomicUsize, label: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while counter.load(SeqCst) < at_least {
            if Instant::now() >= deadline {
                panic!("{label}: wake count stuck at {}", counter.load(SeqCst));
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[gpui::test]
    async fn sample_updates_tab_latest_frame(cx: &mut gpui::TestAppContext) {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        engine.feed(b"Hi".to_vec()).expect("feed");
        engine.build_now().expect("build_now");
        let _ = wait_for_engine_frame(engine.as_ref());

        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        tab.update(cx, |tab, _cx| {
            tab.sample_latest_frame_from_engine();
            assert!(tab.render.latest_frame.is_some());
            let f = tab.render.latest_frame.as_ref().unwrap();
            assert!(
                f.grid[0]
                    .cells
                    .iter()
                    .any(|c| c.grapheme == "H" || c.grapheme == "i")
            );
        });
    }

    #[test]
    fn resize_before_input_orders_engine_and_outbound() {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let (outbound_tx, outbound_rx) = crossbeam_channel::bounded(4);
        let mut input_enabled = false;
        apply_newest_size_before_input(
            engine.as_ref(),
            &outbound_tx,
            120,
            40,
            true,
            &mut input_enabled,
        );
        let first = outbound_rx.try_recv().unwrap();
        assert_eq!(
            first,
            WsOutbound::Resize {
                cols: 120,
                rows: 40
            }
        );
        assert!(outbound_rx.try_recv().is_err(), "no Input before enable");
        assert!(input_enabled);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let snap = engine.inspect();
            if snap.cols == 120 && snap.rows == 40 {
                assert_eq!((snap.cols, snap.rows), (120, 40));
                break;
            }
            if Instant::now() >= deadline {
                panic!(
                    "engine resize not applied: cols={} rows={}",
                    snap.cols, snap.rows
                );
            }
            thread::sleep(Duration::from_millis(1));
        }
        if let Ok(owned) = Arc::try_unwrap(engine) {
            owned.stop();
        }
    }

    #[gpui::test]
    async fn tab_set_visible_forwards_to_engine(cx: &mut gpui::TestAppContext) {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let wake_count = Arc::new(AtomicUsize::new(0));
        {
            let n = Arc::clone(&wake_count);
            engine.set_waker(Box::new(move || {
                n.fetch_add(1, SeqCst);
            }));
        }

        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));

        // Prime: visible publish wakes at least once.
        engine.feed(b"XY".to_vec()).expect("feed");
        engine.build_now().expect("build_now");
        let _ = wait_for_engine_frame(engine.as_ref());
        wait_for_wake_count(1, &wake_count, "initial publish");
        let after_first = wake_count.load(SeqCst);

        tab.update(cx, |tab, cx| tab.set_visible(false, cx));

        engine.feed(b"ZZ".to_vec()).expect("feed");
        engine.build_now().expect("build_now");
        thread::sleep(Duration::from_millis(20));
        assert_eq!(wake_count.load(SeqCst), after_first, "no wake while hidden");

        tab.update(cx, |tab, cx| tab.set_visible(true, cx));
        wait_for_wake_count(after_first + 1, &wake_count, "show-after-hide");
    }

    #[gpui::test]
    async fn teardown_transport_bumps_access_epoch(cx: &mut gpui::TestAppContext) {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let before = engine.access_epoch();
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        tab.update(cx, |tab, cx| tab.teardown_transport_off_foreground(cx));
        assert_eq!(engine.access_epoch(), before + 1);
    }

    #[gpui::test]
    async fn toggle_mouse_local_flips_carried_flag(cx: &mut gpui::TestAppContext) {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        tab.update(cx, |tab, _cx| assert!(!tab.debug_mouse_local_for_test()));
        tab.update(cx, |tab, cx| tab.toggle_mouse_local(cx));
        tab.update(cx, |tab, _cx| assert!(tab.debug_mouse_local_for_test()));
        tab.update(cx, |tab, cx| tab.toggle_mouse_local(cx));
        tab.update(cx, |tab, _cx| assert!(!tab.debug_mouse_local_for_test()));
    }

    // Regression (codex T5 review, CRITICAL): teardown must revoke report authority
    // engine-side via an ordered SetAccess(false). The epoch bump alone is insufficient —
    // a gesture enqueued during the reconnect/detached window carries the freshly-bumped
    // epoch and passes the epoch gate, so a stale engine `write_allowed == true` would leak
    // mouse reports onto the next connection. Proof: after teardown, a wheel under active
    // tracking (same-epoch, would otherwise report) must fall back to local scroll.
    #[gpui::test]
    async fn teardown_revokes_engine_report_authority(cx: &mut gpui::TestAppContext) {
        use crate::engine::command::{GestureDisposition, KeyMods, MouseAck};

        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        // Button-event tracking + SGR: a wheel would report while writable.
        engine
            .feed(b"\x1b[?1002h\x1b[?1006h".to_vec())
            .expect("feed tracking");
        engine.build_now().expect("build");
        let _ = wait_for_engine_frame(engine.as_ref());

        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        tab.update(cx, |tab, cx| tab.teardown_transport_off_foreground(cx));

        // Enqueued AFTER teardown → carries the bumped epoch (epoch gate passes); only the
        // ordered SetAccess(false) can suppress the report. FIFO forwarder makes this
        // deterministic (SetAccess(false) precedes this wheel) — no sleep needed.
        let (ack_tx, ack_rx) = crossbeam_channel::bounded::<MouseAck>(1);
        engine
            .enqueue_wheel(WheelInput {
                lines: -1,
                cell: None,
                px_x: 0.0,
                px_y: 0.0,
                mods: KeyMods::default(),
                access_epoch: 0,
                ack: Some(ack_tx),
            })
            .expect("enqueue wheel");
        let ack = ack_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("wheel ack");
        assert_eq!(
            ack.disposition,
            GestureDisposition::ScrolledLocal,
            "teardown must revoke write_allowed so a post-teardown report becomes local scroll"
        );
    }

    #[gpui::test]
    async fn inspect_disabled_child_rings_cheap(cx: &mut gpui::TestAppContext) {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        engine.feed(b"cheap".to_vec()).expect("feed");
        engine.build_now().expect("build_now");
        let _ = wait_for_engine_frame(engine.as_ref());

        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        tab.update(cx, |tab, _cx| {
            let snap = tab.inspect();
            assert_eq!(snap.lifecycle, Lifecycle::Live);
            assert!(!snap.output_gap);
            assert!(!snap.input_discarded);
            assert!(!snap.bridge_alive);
            assert!(snap.input_enabled);
            assert!(snap.attach.is_none());
            assert!(snap.engine.is_some());
            let engine_snap = snap.engine.unwrap();
            assert!(engine_snap.recent.is_empty());
            assert!(engine_snap.bytes_fed > 0);
        });
    }

    #[gpui::test]
    async fn inspect_enabled_populates_engine(cx: &mut gpui::TestAppContext) {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));

        tab.update(cx, |tab, _cx| tab.set_inspect_enabled(true));
        engine.feed(b"inspect-tab".to_vec()).expect("feed");
        engine.build_now().expect("build_now");
        let _ = wait_for_engine_frame(engine.as_ref());

        tab.update(cx, |tab, _cx| {
            let snap = tab.inspect();
            let engine_snap = snap.engine.expect("engine inspect present");
            assert!(engine_snap.frames_built >= 1);
            assert!(engine_snap.bytes_fed > 0);
            assert!(!engine_snap.recent.is_empty());
        });
    }

    #[test]
    fn tab_render_state_starts_empty() {
        let s = render::state::TabRenderState::new();
        assert!(s.latest_frame.is_none());
        assert!(s.last_stats().is_none());
    }

    #[test]
    fn identity_title_prefers_name_key_for_open_or_create() {
        let t = TerminalTarget::OpenOrCreate {
            session_id: SessionId::new("sess_1"),
            key: TerminalKey {
                terminal_name: "main".into(),
                session_key: "k".into(),
            },
        };
        assert_eq!(identity_title_of(&t), "main:k");
    }

    #[test]
    fn identity_title_falls_back_to_opaque_id_for_existing() {
        // Pre-GET, only the opaque id is known; the stable `name:key` title is
        // resolved once the resource is fetched (Slice 1d).
        let t = TerminalTarget::Existing {
            session_id: SessionId::new("sess_1"),
            terminal_id: TerminalId::new("term_xyz"),
        };
        assert_eq!(identity_title_of(&t), "term_xyz");
    }

    #[test]
    fn open_options_default_is_automatic_engine_scrollback() {
        let o = TerminalOpenOptions::default();
        assert_eq!(o.access, AccessIntent::Automatic);
        assert_eq!(o.scrollback_lines, None);
    }

    #[test]
    fn target_round_trips_json() {
        // Frozen public values are serializable (supports the Inspect contract).
        let t = TerminalTarget::OpenOrCreate {
            session_id: SessionId::new("sess_1"),
            key: TerminalKey {
                terminal_name: "main".into(),
                session_key: "k".into(),
            },
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: TerminalTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn starting_presentation_has_policy_defaults() {
        let target = TerminalTarget::OpenOrCreate {
            session_id: SessionId::new("sess_1"),
            key: TerminalKey {
                terminal_name: "main".into(),
                session_key: "k".into(),
            },
        };
        let p = starting_presentation(&target, &TerminalOpenOptions::default());
        assert!(!p.output_gap);
        assert!(!p.input_discarded);
        assert!(p.detached_detail.is_none());
        assert!(!p.reattach_available);
    }

    #[gpui::test]
    async fn stale_input_discarded_sets_and_clears_on_accepted_keydown(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::{KeyDownEvent, KeyUpEvent, Keystroke};

        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));

        tab.update(cx, |tab, cx| {
            tab.apply_bridge_event(crate::bridge::BridgeEvent::StaleInputDiscarded, cx);
            assert!(tab.presentation().input_discarded);
            assert!(tab.inspect().input_discarded);
        });

        tab.update(cx, |tab, cx| {
            tab.on_focus_changed(true, cx);
            assert!(
                tab.presentation().input_discarded,
                "focus report must not clear input_discarded"
            );
        });

        let (_win, vcx) = cx.add_window_view(|_, _| gpui::Empty);
        vcx.update(|window, cx| {
            let up = Keystroke::parse("up").expect("parse up");
            tab.update(cx, |tab, cx| {
                tab.debug_handle_key_up_for_test(&KeyUpEvent { keystroke: up }, window, cx);
                assert!(
                    tab.presentation().input_discarded,
                    "key-up without prior press must not clear input_discarded"
                );
            });
        });

        vcx.update(|window, cx| {
            let enter = Keystroke::parse("enter").expect("parse enter");
            tab.update(cx, |tab, cx| {
                tab.debug_handle_key_down_for_test(
                    &KeyDownEvent {
                        keystroke: enter,
                        is_held: false,
                    },
                    window,
                    cx,
                );
                assert!(
                    !tab.presentation().input_discarded,
                    "accepted keydown must clear input_discarded"
                );
                assert!(!tab.inspect().input_discarded);
            });
        });
    }

    #[gpui::test]
    async fn printable_key_emits_exactly_once_via_input_handler_not_keydown(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::Keystroke;

        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let egress = engine.attach_test_egress();
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        while egress.try_recv().is_ok() {}

        let plain_a = Keystroke {
            key: "a".into(),
            modifiers: Default::default(),
            key_char: Some("a".into()),
        };
        tab.update(cx, |tab, _cx| {
            assert!(
                !tab.debug_key_down_for_test(plain_a, false),
                "unmodified printable must not enqueue from keydown"
            );
        });
        assert!(
            egress.try_recv().is_err(),
            "keydown must not emit egress for plain a"
        );

        tab.update(cx, |tab, _cx| tab.debug_input_handler_text_for_test("a"));
        let frame = egress
            .recv_timeout(Duration::from_secs(2))
            .expect("committed text egress");
        assert_eq!(frame.bytes, b"a");
        assert!(
            egress.try_recv().is_err(),
            "exactly one egress for printable via InputHandler"
        );
    }

    #[gpui::test]
    async fn ime_commit_hook_returns_receiver_without_blocking(cx: &mut gpui::TestAppContext) {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let _egress = engine.attach_test_egress();
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        let rx = tab.update(cx, |tab, _cx| {
            tab.debug_ime_commit_for_test("你好").expect("rx")
        });
        let ack = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(ack.encoded, "你好".as_bytes());
        assert!(ack.accepted);
    }

    fn test_clipboard_contents(text: &str) -> Vec<engine::presentation::ClipboardMimePart> {
        vec![engine::presentation::ClipboardMimePart {
            mime: "text/plain".into(),
            data: text.into(),
        }]
    }

    fn subscribe_terminal_events(
        cx: &mut gpui::TestAppContext,
        tab: &Entity<TerminalTab>,
    ) -> (
        std::rc::Rc<std::cell::RefCell<Vec<TerminalEvent>>>,
        Subscription,
    ) {
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::<TerminalEvent>::new()));
        let sink = events.clone();
        let tab_for_sub = tab.clone();
        let sub = cx.update(|cx| {
            cx.subscribe(&tab_for_sub, move |_entity, event, _cx| {
                sink.borrow_mut().push(event.clone());
            })
        });
        (events, sub)
    }

    fn enqueue_clipboard_write(
        engine: &EngineHandle,
        location: engine::presentation::ClipboardLocation,
        contents: Vec<engine::presentation::ClipboardMimePart>,
    ) {
        engine
            .enqueue_presentation(
                engine::presentation::EnginePresentationEvent::ClipboardWrite {
                    location,
                    contents,
                },
            )
            .expect("enqueue clipboard write");
    }

    #[gpui::test]
    async fn clipboard_write_with_no_session_decision_emits_request(cx: &mut gpui::TestAppContext) {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        let (events, _sub) = subscribe_terminal_events(cx, &tab);

        let contents = test_clipboard_contents("hello-osc52");
        enqueue_clipboard_write(
            engine.as_ref(),
            engine::presentation::ClipboardLocation::Standard,
            contents.clone(),
        );
        tab.update(cx, |tab, cx| tab.debug_drain_presentation_for_test(cx));

        let evs = events.borrow();
        assert_eq!(evs.len(), 1);
        match &evs[0] {
            TerminalEvent::ClipboardWriteRequest {
                id,
                location,
                contents: got,
            } => {
                assert_eq!(*location, engine::presentation::ClipboardLocation::Standard);
                assert_eq!(got, &contents);
                tab.update(cx, |tab, _cx| {
                    assert_eq!(tab.debug_pending_clipboard_ids_for_test(), vec![*id]);
                });
            }
            other => panic!("expected ClipboardWriteRequest, got {other:?}"),
        }
    }

    #[gpui::test]
    async fn remembered_deny_suppresses_request_and_records_denied(cx: &mut gpui::TestAppContext) {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        tab.update(cx, |tab, _cx| tab.set_inspect_enabled(true));
        tab.update(cx, |tab, _cx| {
            tab.debug_remember_osc52_for_test(
                engine::presentation::ClipboardLocation::Standard,
                HostRequestDecision::Deny,
            );
        });

        let (events, _sub) = subscribe_terminal_events(cx, &tab);
        enqueue_clipboard_write(
            engine.as_ref(),
            engine::presentation::ClipboardLocation::Standard,
            test_clipboard_contents("denied"),
        );
        tab.update(cx, |tab, cx| tab.debug_drain_presentation_for_test(cx));

        assert!(
            events
                .borrow()
                .iter()
                .all(|e| !matches!(e, TerminalEvent::ClipboardWriteRequest { .. })),
            "deny must suppress ClipboardWriteRequest"
        );
        tab.update(cx, |tab, _cx| {
            assert!(tab.debug_pending_clipboard_ids_for_test().is_empty());
            let snap = tab.inspect().engine.expect("engine inspect");
            assert_eq!(snap.clipboard_writes_denied, 1);
            assert_eq!(snap.clipboard_writes_allowed, 0);
        });
    }

    #[gpui::test]
    async fn host_allow_writes_clipboard_and_evicts_pending(cx: &mut gpui::TestAppContext) {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        let (events, _sub) = subscribe_terminal_events(cx, &tab);

        let contents = test_clipboard_contents("paste-me");
        enqueue_clipboard_write(
            engine.as_ref(),
            engine::presentation::ClipboardLocation::Standard,
            contents,
        );
        tab.update(cx, |tab, cx| tab.debug_drain_presentation_for_test(cx));

        let request_id = match &events.borrow()[0] {
            TerminalEvent::ClipboardWriteRequest { id, .. } => *id,
            other => panic!("expected ClipboardWriteRequest, got {other:?}"),
        };

        tab.update(cx, |tab, cx| {
            tab.on_host_event(
                TerminalHostEvent::HostRequestResponse {
                    id: request_id,
                    decision: HostRequestDecision::Allow,
                },
                cx,
            );
            assert!(tab.debug_pending_clipboard_ids_for_test().is_empty());
            let clip = cx.read_from_clipboard().and_then(|c| c.text());
            assert_eq!(clip.as_deref(), Some("paste-me"));
        });

        assert!(
            events.borrow().iter().any(|e| matches!(
                e,
                TerminalEvent::ClipboardWriteNotice {
                    location: engine::presentation::ClipboardLocation::Standard,
                    bytes: 8,
                }
            )),
            "Allow path must emit ClipboardWriteNotice"
        );
    }

    #[gpui::test]
    async fn pending_clipboard_writes_are_bounded_drop_oldest(cx: &mut gpui::TestAppContext) {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        let (_events, _sub) = subscribe_terminal_events(cx, &tab);

        for i in 0..=PENDING_HOST_REQUESTS_CAP {
            enqueue_clipboard_write(
                engine.as_ref(),
                engine::presentation::ClipboardLocation::Standard,
                test_clipboard_contents(&format!("write-{i}")),
            );
            tab.update(cx, |tab, cx| tab.debug_drain_presentation_for_test(cx));
        }

        tab.update(cx, |tab, _cx| {
            let ids = tab.debug_pending_clipboard_ids_for_test();
            assert_eq!(ids.len(), PENDING_HOST_REQUESTS_CAP);
            assert!(
                !ids.contains(&HostRequestId(0)),
                "oldest id must be evicted when cap exceeded"
            );
            assert!(ids.contains(&HostRequestId(PENDING_HOST_REQUESTS_CAP as u64)));
        });
    }

    #[test]
    fn paste_needs_warn_only_on_multiline_and_not_suppressed() {
        assert!(TerminalTab::paste_needs_warn("a\nb", false));
        assert!(!TerminalTab::paste_needs_warn("ab", false));
        assert!(!TerminalTab::paste_needs_warn("a\nb", true));
    }

    #[gpui::test]
    async fn real_cmd_v_keystroke_routes_to_paste_not_key_encoder(cx: &mut gpui::TestAppContext) {
        use gpui::{ClipboardItem, KeyDownEvent, Keystroke};

        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let egress = engine.attach_test_egress();
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        tab.update(cx, |tab, _cx| tab.set_inspect_enabled(true));
        while egress.try_recv().is_ok() {}

        cx.write_to_clipboard(ClipboardItem::new_string("hi".to_string()));

        let cmd_v = Keystroke {
            key: "v".into(),
            modifiers: gpui::Modifiers {
                platform: true,
                ..Default::default()
            },
            key_char: None,
        };

        let (_win, vcx) = cx.add_window_view(|_, _| gpui::Empty);
        vcx.update(|window, cx| {
            tab.update(cx, |tab, cx| {
                tab.debug_handle_key_down_for_test(
                    &KeyDownEvent {
                        keystroke: cmd_v,
                        is_held: false,
                    },
                    window,
                    cx,
                );
            });
            tab.update(cx, |tab, _cx| {
                let snap = tab.inspect().engine.expect("engine inspect");
                assert_eq!(snap.pastes_sent, 1);
            });
            let up = Keystroke::parse("up").expect("parse up");
            tab.update(cx, |tab, cx| {
                tab.debug_handle_key_down_for_test(
                    &KeyDownEvent {
                        keystroke: up,
                        is_held: false,
                    },
                    window,
                    cx,
                );
            });
        });

        let paste_frame = egress
            .recv_timeout(Duration::from_secs(2))
            .expect("paste must emit egress");
        assert_eq!(paste_frame.kind, EgressKind::Input);
        assert_eq!(paste_frame.bytes, b"hi");

        let sentinel_frame = egress
            .recv_timeout(Duration::from_secs(2))
            .expect("sentinel key must follow paste in FIFO order");
        assert_eq!(sentinel_frame.kind, EgressKind::Input);
        assert_eq!(
            sentinel_frame.bytes, b"\x1b[A",
            "next frame must be ArrowUp sentinel, not a stray Cmd+V key encode"
        );

        tab.update(cx, |tab, _cx| {
            let snap = tab.inspect().engine.expect("engine inspect");
            assert_eq!(
                snap.keys_encoded, 1,
                "only the sentinel key should increment keys_encoded"
            );
        });
    }

    #[gpui::test]
    async fn cmd_v_single_line_dispatches_paste(cx: &mut gpui::TestAppContext) {
        use gpui::ClipboardItem;

        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        tab.update(cx, |tab, _cx| tab.set_inspect_enabled(true));
        let (events, _sub) = subscribe_terminal_events(cx, &tab);

        cx.write_to_clipboard(ClipboardItem::new_string("hello".to_string()));
        tab.update(cx, |tab, cx| tab.debug_paste_for_test(cx));

        tab.update(cx, |tab, _cx| {
            let snap = tab.inspect().engine.expect("engine inspect");
            assert_eq!(snap.pastes_sent, 1);
        });
        assert!(
            events
                .borrow()
                .iter()
                .all(|e| !matches!(e, TerminalEvent::PasteWarnRequest { .. })),
            "single-line paste must not emit PasteWarnRequest"
        );
    }

    #[gpui::test]
    async fn cmd_v_multiline_unsuppressed_emits_warn_no_dispatch(cx: &mut gpui::TestAppContext) {
        use gpui::ClipboardItem;

        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        tab.update(cx, |tab, _cx| tab.set_inspect_enabled(true));
        let (events, _sub) = subscribe_terminal_events(cx, &tab);

        cx.write_to_clipboard(ClipboardItem::new_string("a\nb".to_string()));
        tab.update(cx, |tab, cx| tab.debug_paste_for_test(cx));

        let evs = events.borrow();
        let warn_count = evs
            .iter()
            .filter(|e| matches!(e, TerminalEvent::PasteWarnRequest { .. }))
            .count();
        assert_eq!(warn_count, 1);
        match &evs[0] {
            TerminalEvent::PasteWarnRequest { line_count, .. } => {
                assert_eq!(*line_count, 2);
            }
            other => panic!("expected PasteWarnRequest, got {other:?}"),
        }
        tab.update(cx, |tab, _cx| {
            let snap = tab.inspect().engine.expect("engine inspect");
            assert_eq!(snap.pastes_sent, 0);
            assert_eq!(snap.paste_warn_prompts, 1);
            assert_eq!(tab.debug_pending_paste_ids_for_test().len(), 1);
        });
    }

    #[gpui::test]
    async fn paste_warn_allow_session_suppresses_and_dispatches(cx: &mut gpui::TestAppContext) {
        use gpui::ClipboardItem;

        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        tab.update(cx, |tab, _cx| tab.set_inspect_enabled(true));
        let (events, _sub) = subscribe_terminal_events(cx, &tab);

        cx.write_to_clipboard(ClipboardItem::new_string("a\nb".to_string()));
        tab.update(cx, |tab, cx| tab.debug_paste_for_test(cx));

        let request_id = match &events.borrow()[0] {
            TerminalEvent::PasteWarnRequest { id, .. } => *id,
            other => panic!("expected PasteWarnRequest, got {other:?}"),
        };

        tab.update(cx, |tab, cx| {
            tab.on_host_event(
                TerminalHostEvent::HostRequestResponse {
                    id: request_id,
                    decision: HostRequestDecision::AllowSession,
                },
                cx,
            );
            assert!(tab.debug_paste_warn_suppressed_for_test());
            assert!(tab.debug_pending_paste_ids_for_test().is_empty());
            let snap = tab.inspect().engine.expect("engine inspect");
            assert_eq!(snap.pastes_sent, 1);
        });
    }

    #[gpui::test]
    async fn over_cap_paste_rejected_before_pending(cx: &mut gpui::TestAppContext) {
        use gpui::ClipboardItem;

        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        tab.update(cx, |tab, _cx| tab.set_inspect_enabled(true));
        let (events, _sub) = subscribe_terminal_events(cx, &tab);

        let over_cap = "x\n".repeat((MAX_PASTE_BYTES / 2) + 1);
        assert!(over_cap.len() > MAX_PASTE_BYTES);
        assert!(over_cap.contains('\n'));

        cx.write_to_clipboard(ClipboardItem::new_string(over_cap));
        tab.update(cx, |tab, cx| tab.debug_paste_for_test(cx));

        tab.update(cx, |tab, _cx| {
            assert!(tab.presentation().input_discarded);
            let snap = tab.inspect().engine.expect("engine inspect");
            assert_eq!(snap.pastes_sent, 0);
            assert_eq!(snap.paste_over_cap_rejects, 1);
            assert!(tab.debug_pending_paste_ids_for_test().is_empty());
        });
        assert!(
            events
                .borrow()
                .iter()
                .any(|e| matches!(e, TerminalEvent::PresentationChanged)),
            "over-cap reject must emit PresentationChanged"
        );
        assert!(
            events
                .borrow()
                .iter()
                .all(|e| !matches!(e, TerminalEvent::PasteWarnRequest { .. })),
            "over-cap must not enter pending_pastes warn path"
        );
    }

    #[gpui::test]
    async fn deferred_paste_allow_after_readonly_downgrade_is_suppressed(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::ClipboardItem;

        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let egress = engine.attach_test_egress();
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        tab.update(cx, |tab, _cx| tab.set_inspect_enabled(true));
        let (events, _sub) = subscribe_terminal_events(cx, &tab);
        while egress.try_recv().is_ok() {}

        cx.write_to_clipboard(ClipboardItem::new_string("a\nb".to_string()));
        tab.update(cx, |tab, cx| tab.debug_paste_for_test(cx));

        let request_id = match &events.borrow()[0] {
            TerminalEvent::PasteWarnRequest { id, line_count } => {
                assert_eq!(*line_count, 2);
                *id
            }
            other => panic!("expected PasteWarnRequest, got {other:?}"),
        };
        tab.update(cx, |tab, _cx| {
            assert_eq!(tab.debug_pending_paste_ids_for_test(), vec![request_id]);
            let snap = tab.inspect().engine.expect("engine inspect");
            assert_eq!(snap.pastes_sent, 0);
            tab.presentation.access = AccessMode::ReadOnly;
        });

        tab.update(cx, |tab, cx| {
            tab.on_host_event(
                TerminalHostEvent::HostRequestResponse {
                    id: request_id,
                    decision: HostRequestDecision::AllowSession,
                },
                cx,
            );
        });

        tab.update(cx, |tab, _cx| {
            let snap = tab.inspect().engine.expect("engine inspect");
            assert_eq!(snap.pastes_sent, 0);
            assert!(tab.debug_pending_paste_ids_for_test().is_empty());
        });
        assert!(
            egress.try_recv().is_err(),
            "read-only downgrade must suppress deferred paste egress"
        );
    }

    #[gpui::test]
    async fn read_only_tab_ignores_cmd_v(cx: &mut gpui::TestAppContext) {
        use gpui::ClipboardItem;

        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let egress = engine.attach_test_egress();
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        tab.update(cx, |tab, _cx| tab.set_inspect_enabled(true));
        while egress.try_recv().is_ok() {}

        tab.update(cx, |tab, _cx| {
            tab.presentation.access = AccessMode::ReadOnly;
        });

        cx.write_to_clipboard(ClipboardItem::new_string("hello".to_string()));
        tab.update(cx, |tab, cx| tab.debug_paste_for_test(cx));

        tab.update(cx, |tab, _cx| {
            let snap = tab.inspect().engine.expect("engine inspect");
            assert_eq!(snap.pastes_sent, 0);
            assert_eq!(snap.paste_warn_prompts, 0);
            assert_eq!(snap.paste_over_cap_rejects, 0);
        });
        assert!(
            egress.try_recv().is_err(),
            "read-only tab must not dispatch paste"
        );
    }

    fn osc52_vt_write_bytes(decoded: &[u8]) -> Vec<u8> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let mut v = Vec::from(b"\x1b]52;c;");
        v.extend_from_slice(STANDARD.encode(decoded).as_bytes());
        v.push(0x07);
        v
    }

    #[gpui::test]
    async fn inspect_exposes_osc52_forwarded_and_pastes_sent_counters(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::ClipboardItem;

        use crate::engine::presentation::{ClipboardLocation, EnginePresentationEvent};

        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let egress = engine.attach_test_egress();
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        tab.update(cx, |tab, _cx| tab.set_inspect_enabled(true));
        while egress.try_recv().is_ok() {}

        engine
            .feed(osc52_vt_write_bytes(b"inspect-exposure"))
            .expect("feed osc52");

        let ev = engine
            .presentation_rx()
            .recv_timeout(Duration::from_secs(2))
            .expect("OSC-52 clipboard write must arrive on presentation channel");
        match ev {
            EnginePresentationEvent::ClipboardWrite { location, contents } => {
                assert_eq!(location, ClipboardLocation::Standard);
                assert_eq!(contents.len(), 1);
                assert_eq!(contents[0].data, "inspect-exposure");
            }
            other => panic!("expected ClipboardWrite presentation event, got {other:?}"),
        }

        cx.write_to_clipboard(ClipboardItem::new_string("paste".to_string()));
        tab.update(cx, |tab, cx| tab.debug_paste_for_test(cx));

        let paste_frame = egress
            .recv_timeout(Duration::from_secs(2))
            .expect("paste must emit egress after OSC-52 feed");
        assert_eq!(paste_frame.kind, EgressKind::Input);
        assert_eq!(paste_frame.bytes, b"paste");

        tab.update(cx, |tab, _cx| {
            let snap = tab.inspect().engine.expect("engine inspect");
            assert_eq!(
                snap.osc52_forwarded, 1,
                "OSC-52 forward must be inspect-visible"
            );
            assert_eq!(
                snap.pastes_sent, 1,
                "paste dispatch must be inspect-visible"
            );
        });
    }
}
