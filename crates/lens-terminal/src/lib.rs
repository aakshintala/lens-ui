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

use std::sync::Arc;

mod engine;

use gpui::prelude::*;
use gpui::{App, Context, Entity, EventEmitter, FocusHandle, IntoElement, Render, Window, div};
use lens_client::Client;
use lens_client::ids::{SessionId, TerminalId};
use serde::{Deserialize, Serialize};

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

    // Captured at `open()`; consumed by the transport (Slice 1a) + convergence
    // (Slice 1d) that drive off-thread discovery/attach. `#[expect]` (not
    // `#[allow]`) so the lint fires the moment a later slice starts reading
    // these — a self-clearing reminder to drop the attribute.
    #[expect(dead_code, reason = "consumed by Slice 1a transport + 1d convergence")]
    target: TerminalTarget,
    #[expect(dead_code, reason = "consumed by Slice 1a transport + 1d convergence")]
    client: Arc<Client>,
    #[expect(dead_code, reason = "consumed by Slice 1a transport + 1d convergence")]
    options: TerminalOpenOptions,
}

impl TerminalTab {
    fn starting(
        target: TerminalTarget,
        client: Arc<Client>,
        options: TerminalOpenOptions,
        cx: &mut Context<Self>,
    ) -> Self {
        let identity_title = identity_title_of(&target);
        // Slice 1d kicks off off-thread discovery/attach here; Slice 0 only
        // freezes the "returns in Starting" invariant.
        Self {
            focus_handle: cx.focus_handle(),
            lifecycle: Lifecycle::Starting,
            presentation: Presentation {
                lifecycle: Lifecycle::Starting,
                access: match options.access {
                    AccessIntent::ReadOnly => AccessMode::ReadOnly,
                    // `Automatic` resolves to the server-authoritative mode once
                    // attached; before attach it presents read-only.
                    AccessIntent::Automatic => AccessMode::ReadOnly,
                },
                identity_title,
                reported_title: None,
                progress: None,
            },
            target,
            client,
            options,
        }
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
}

impl EventEmitter<TerminalEvent> for TerminalTab {}

impl Render for TerminalTab {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Slice 1c replaces this placeholder with the full-snapshot `Frame`
        // painter. Renders modeled values only — never panics.
        div().track_focus(&self.focus_handle).child(format!(
            "{} — {:?}",
            self.presentation.identity_title, self.lifecycle
        ))
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
    cx.new(|cx| TerminalTab::starting(target, client, options, cx))
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
    use super::*;

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
}
