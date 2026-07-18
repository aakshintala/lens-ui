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

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use lens_client::WsOutbound;

mod bridge;
mod engine;
mod inspect;
mod policy;
mod render;
mod runtime;

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

use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle, IntoElement,
    KeyDownEvent, KeyUpEvent, Pixels, Render, UTF16Selection, Window,
};
use lens_client::Client;
use lens_client::ids::{SessionId, TerminalId};
use lens_client::{AttachOptions, TerminalResource, attach};
use serde::{Deserialize, Serialize};

use bridge::{BridgeEvent, spawn_bridge};
use engine::command::{InputAck, KeyAction, KeyInput, LensKey};
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
pub use engine::{
    CursorPos, EngineConfig, EngineError, EngineHandle, EngineInspect, FeedError, VtEngine,
};
pub use inspect::TerminalInspect;
pub use render::inspect::{RenderInspect, RenderInspectEvent, RenderInspectEventKind};

// ---------------------------------------------------------------------------
// Typed event seams (opaque; grow across slices — hence `#[non_exhaustive]`).
// ---------------------------------------------------------------------------

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
}

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

    /// The single typed inbound seam. Slice 0 accepts and ignores events; the
    /// concrete handling lands with the slice that owns each variant.
    pub fn on_host_event(&mut self, _event: TerminalHostEvent, _cx: &mut Context<Self>) {
        // Slice 1d+ dispatches Sleep/wake/reset/etc.
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

    fn write_input_allowed(&self) -> bool {
        self.input_enabled && matches!(self.presentation.access, AccessMode::Write)
    }

    /// Special/modified keys only — plain printable text is owned by [`EntityInputHandler`].
    pub(crate) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let ks = &event.keystroke;
        if !keydown_should_enqueue(&ks.key, &ks.modifiers) {
            return;
        }
        let action = if event.is_held {
            KeyAction::Repeat
        } else {
            KeyAction::Press
        };
        self.enqueue_key(KeyInput {
            action,
            key: keystroke_to_lens(&ks.key),
            mods: gpui_mods_to_key_mods(&ks.modifiers),
            utf8: ks.key_char.clone(),
            composing: false,
            access_epoch: 0,
            ack: None,
        });
    }

    pub(crate) fn handle_key_up(
        &mut self,
        event: &KeyUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let ks = &event.keystroke;
        if !keydown_should_enqueue(&ks.key, &ks.modifiers) {
            return;
        }
        self.enqueue_key(KeyInput {
            action: KeyAction::Release,
            key: keystroke_to_lens(&ks.key),
            mods: gpui_mods_to_key_mods(&ks.modifiers),
            utf8: ks.key_char.clone(),
            composing: false,
            access_epoch: 0,
            ack: None,
        });
    }

    fn enqueue_key(&mut self, input: KeyInput) {
        if !self.write_input_allowed() {
            return;
        }
        let Some(rt) = &self.runtime else {
            return;
        };
        let Some(engine) = &rt.engine else {
            return;
        };
        let _ = engine.enqueue_input(EngineCommand::Key(input));
    }

    fn enqueue_committed_text(
        &mut self,
        text: &str,
    ) -> Option<crossbeam_channel::Receiver<InputAck>> {
        if text.is_empty() || !self.write_input_allowed() {
            return None;
        }
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        let input = KeyInput {
            action: KeyAction::Press,
            key: LensKey::Unidentified,
            mods: engine::command::KeyMods::default(),
            utf8: Some(text.to_owned()),
            composing: false,
            access_epoch: 0,
            ack: Some(ack_tx),
        };
        let rt = self.runtime.as_ref()?;
        let engine = rt.engine.as_ref()?;
        engine
            .enqueue_input(EngineCommand::Key(input))
            .ok()
            .map(|()| ack_rx)
    }

    /// Simulate keydown for harness tests; returns whether a key command was enqueued.
    #[cfg(any(test, feature = "test-util"))]
    pub fn debug_key_down_for_test(&mut self, keystroke: gpui::Keystroke, is_held: bool) -> bool {
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
        self.enqueue_key(KeyInput {
            action,
            key: keystroke_to_lens(&keystroke.key),
            mods: gpui_mods_to_key_mods(&keystroke.modifiers),
            utf8: keystroke.key_char.clone(),
            composing: false,
            access_epoch: 0,
            ack: None,
        });
        true
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
        self.enqueue_committed_text(text)
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
            let (bridge, attach) = rt.take_transport();
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
        };

        match action {
            PolicyAction::StopDetached {
                detail,
                reattach_available,
            } => {
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
        let bridge = spawn_bridge(
            attach.inbound.clone(),
            attach.outbound.clone(),
            Arc::clone(&engine),
            self.policy_tx.clone().expect("policy_tx retained"),
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
        let _ = self.enqueue_committed_text(text);
        cx.notify();
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
        let frame = self.render.latest_frame.as_ref()?;
        let cursor = frame.cursor?;
        let metrics = self.render.cell_metrics.as_ref()?;
        let x = element_bounds.origin.x + metrics.cell_w * f32::from(cursor.col);
        let y = element_bounds.origin.y + metrics.cell_h * f32::from(cursor.row);
        Some(Bounds::new(
            gpui::point(x, y),
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
        self.sample_latest_frame_from_engine();
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
        detached_detail: None,
        reattach_available: false,
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
        assert!(p.detached_detail.is_none());
        assert!(!p.reattach_available);
    }

    #[gpui::test]
    async fn printable_key_emits_exactly_once_via_input_handler_not_keydown(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::Keystroke;

        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        while engine.egress_rx().try_recv().is_ok() {}

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
            engine.egress_rx().try_recv().is_err(),
            "keydown must not emit egress for plain a"
        );

        tab.update(cx, |tab, _cx| tab.debug_input_handler_text_for_test("a"));
        let bytes = engine
            .egress_rx()
            .recv_timeout(Duration::from_secs(2))
            .expect("committed text egress");
        assert_eq!(bytes, b"a");
        assert!(
            engine.egress_rx().try_recv().is_err(),
            "exactly one egress for printable via InputHandler"
        );
    }

    #[gpui::test]
    async fn ime_commit_hook_returns_receiver_without_blocking(cx: &mut gpui::TestAppContext) {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let tab = cx.new(|cx| TerminalTab::with_engine_for_test(Arc::clone(&engine), cx));
        let rx = tab.update(cx, |tab, _cx| {
            tab.debug_ime_commit_for_test("你好").expect("rx")
        });
        let ack = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(ack.encoded, "你好".as_bytes());
        assert!(ack.accepted);
    }
}
