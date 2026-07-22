//! Discovery, attach, close-code policy, and resolve helpers (Slice 1d).

use std::sync::Arc;
use std::time::{Duration, Instant};

use lens_client::error::ClientError;
use lens_client::ids::{SessionId, TerminalId};
use lens_client::{
    AttachOptions, Backoff, Client, CloseCause, TerminalCreate, TerminalResource, attach,
};

use crate::bridge::{self, BridgeEvent};
use crate::engine::EngineHandle;
use crate::engine::vt::EngineConfig;
use crate::runtime::TerminalRuntime;
use crate::{AccessIntent, DetachedDetail, TerminalKey, TerminalOpenOptions, TerminalTarget};

/// Close-code policy outcome — consumed by the tab foreground handler.
#[derive(Debug)]
pub(crate) enum PolicyAction {
    StopDetached {
        detail: DetachedDetail,
        reattach_available: bool,
    },
    Retry {
        delay: Duration,
    },
    DowngradeReadOnly,
}

/// 30s wall-clock retry window with per-attempt backoff (injected `now` for tests).
pub(crate) struct RetryWindow {
    started: Option<Instant>,
    backoff: Backoff,
}

impl RetryWindow {
    pub fn new() -> Self {
        Self {
            started: None,
            backoff: Backoff::default(),
        }
    }

    /// `now` injected. First call starts a 30s wall window. Returns None once `now >= started+30s`.
    /// `Some(delay)` = `backoff.next_delay()` clamped to the remaining window.
    pub fn next_delay(&mut self, now: Instant) -> Option<Duration> {
        let window = Duration::from_secs(30);
        match self.started {
            None => {
                self.started = Some(now);
                Some(self.backoff.next_delay().min(window))
            }
            Some(start) => {
                if now >= start + window {
                    None
                } else {
                    let remaining = (start + window).saturating_duration_since(now);
                    Some(self.backoff.next_delay().min(remaining))
                }
            }
        }
    }

    pub fn reset(&mut self) {
        self.started = None;
        self.backoff.reset();
    }
}

/// Pure close-code policy state (unauthorized-twice tracking + retry window).
pub(crate) struct PolicyState {
    pub retry: RetryWindow,
    pub unauthorized_once: bool,
}

impl PolicyState {
    pub fn new() -> Self {
        Self {
            retry: RetryWindow::new(),
            unauthorized_once: false,
        }
    }

    pub fn on_close(&mut self, cause: CloseCause, now: Instant) -> PolicyAction {
        match cause {
            CloseCause::TerminalNotFound => PolicyAction::StopDetached {
                detail: DetachedDetail::TerminalGone,
                reattach_available: false,
            },
            CloseCause::TerminalDetached => PolicyAction::StopDetached {
                detail: DetachedDetail::ClientDetached,
                reattach_available: true,
            },
            CloseCause::Unauthorized => {
                if self.unauthorized_once {
                    PolicyAction::StopDetached {
                        detail: DetachedDetail::Unauthorized,
                        reattach_available: false,
                    }
                } else {
                    self.unauthorized_once = true;
                    PolicyAction::DowngradeReadOnly
                }
            }
            CloseCause::Internal | CloseCause::Network => self.retry_or_exhausted(now),
            _ => self.retry_or_exhausted(now),
        }
    }

    fn retry_or_exhausted(&mut self, now: Instant) -> PolicyAction {
        match self.retry.next_delay(now) {
            Some(delay) => PolicyAction::Retry { delay },
            None => PolicyAction::StopDetached {
                detail: DetachedDetail::RetriesExhausted,
                reattach_available: false,
            },
        }
    }
}

/// Outcome of off-thread discover + attach, applied on the foreground.
pub(crate) struct AttachedParts {
    pub resource: TerminalResource,
    pub runtime: TerminalRuntime,
    pub wake_tx: async_channel::Sender<()>,
    pub wake_rx: async_channel::Receiver<()>,
    pub policy_rx: async_channel::Receiver<BridgeEvent>,
    /// Retained on the tab so sampler `policy_rx` survives bridge replacement on reconnect.
    pub policy_tx: async_channel::Sender<BridgeEvent>,
}

/// Blocking REST + attach + engine + bridge. No gpui.
pub(crate) fn discover_and_attach(
    client: Arc<Client>,
    target: TerminalTarget,
    options: TerminalOpenOptions,
) -> Result<AttachedParts, DetachedDetail> {
    let read_only = matches!(options.access, AccessIntent::ReadOnly);
    let resource = resolve_terminal(&client, &target)?;
    let sid = resource.session_id.clone();
    let tid = resource.id.clone();

    let attach_handle = attach(client.as_ref(), &sid, &tid, AttachOptions { read_only })
        .map_err(|_| DetachedDetail::DiscoveryFailed)?;

    let cfg = engine_config_for(&resource, &options);
    let engine = Arc::new(EngineHandle::spawn(cfg).map_err(DetachedDetail::from)?);
    let (egress_tx, egress_rx) =
        crossbeam_channel::bounded(crate::engine::worker::EGRESS_CHANNEL_CAP);
    engine
        .attach_egress(egress_tx)
        .map_err(|_| DetachedDetail::DiscoveryFailed)?;

    let (wake_tx, wake_rx) = async_channel::bounded(1);
    let (policy_tx, policy_rx) = async_channel::bounded(32);
    let bridge = bridge::spawn_bridge(
        attach_handle.inbound.clone(),
        attach_handle.outbound.clone(),
        Arc::clone(&engine),
        policy_tx.clone(),
        egress_rx,
    );

    let runtime = TerminalRuntime {
        bridge: Some(bridge),
        attach: Some(attach_handle),
        engine: Some(engine),
    };

    Ok(AttachedParts {
        resource,
        runtime,
        wake_tx,
        wake_rx,
        policy_rx,
        policy_tx,
    })
}

fn resolve_terminal(
    client: &Client,
    target: &TerminalTarget,
) -> Result<TerminalResource, DetachedDetail> {
    match target {
        TerminalTarget::Existing {
            session_id,
            terminal_id,
        } => client
            .terminals(session_id.clone())
            .get(terminal_id)
            .map_err(map_resolve_error),
        TerminalTarget::OpenOrCreate { session_id, key } => {
            let terminals = client.terminals(session_id.clone());
            let list = terminals.list().map_err(map_resolve_error)?;
            if let Some(found) = list.into_iter().find(|r| matches_key(r, key)) {
                return Ok(found);
            }
            terminals
                .create(&TerminalCreate {
                    terminal: key.terminal_name.clone(),
                    session_key: key.session_key.clone(),
                })
                .map_err(map_resolve_error)
        }
    }
}

fn map_resolve_error(err: ClientError) -> DetachedDetail {
    match err {
        ClientError::NotFound { .. } => DetachedDetail::TerminalGone,
        ClientError::Auth { .. } => DetachedDetail::Unauthorized,
        _ => DetachedDetail::DiscoveryFailed,
    }
}

/// Pre-reconnect GET guard — confirms the terminal still exists before attach retry.
pub(crate) fn preflight_reconnect(
    client: &Client,
    session: &SessionId,
    tid: &TerminalId,
) -> Result<TerminalResource, DetachedDetail> {
    client
        .terminals(session.clone())
        .get(tid)
        .map_err(map_resolve_error)
}

/// Whether a listed terminal resource matches an [`OpenOrCreate`] key.
pub(crate) fn matches_key(resource: &TerminalResource, key: &TerminalKey) -> bool {
    resource.metadata.terminal_name.as_deref() == Some(key.terminal_name.as_str())
        && resource.metadata.session_key.as_deref() == Some(key.session_key.as_str())
}

pub(crate) fn identity_title_from_resource(resource: &TerminalResource) -> String {
    match (
        resource.metadata.terminal_name.as_deref(),
        resource.metadata.session_key.as_deref(),
    ) {
        (Some(name), Some(key)) => format!("{name}:{key}"),
        _ => resource.id.to_string(),
    }
}

fn engine_config_for(resource: &TerminalResource, options: &TerminalOpenOptions) -> EngineConfig {
    let _ = resource;
    EngineConfig {
        cols: 80,
        rows: 24,
        max_scrollback: options.scrollback_lines.unwrap_or(1000),
        cell_w_px: 8,
        cell_h_px: 16,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use lens_client::CloseCause;
    use lens_client::ids::{SessionId, TerminalId};

    fn sample_resource(name: &str, key: &str) -> TerminalResource {
        TerminalResource {
            id: TerminalId::new("term_1"),
            session_id: SessionId::new("sess_1"),
            name: Some(name.into()),
            object: None,
            kind: Some("terminal".into()),
            environment: None,
            metadata: lens_client::TerminalMetadata {
                terminal_name: Some(name.into()),
                session_key: Some(key.into()),
                running: Some(true),
                terminal_transport: None,
            },
        }
    }

    #[test]
    fn map_resolve_error_not_found_maps_to_terminal_gone() {
        let err = ClientError::NotFound {
            what: "terminal".into(),
        };
        assert_eq!(map_resolve_error(err), DetachedDetail::TerminalGone);
    }

    #[test]
    fn matches_key_requires_both_metadata_fields() {
        let key = TerminalKey {
            terminal_name: "main".into(),
            session_key: "k".into(),
        };
        assert!(matches_key(&sample_resource("main", "k"), &key));
        assert!(!matches_key(&sample_resource("other", "k"), &key));
        assert!(!matches_key(&sample_resource("main", "other"), &key));
    }

    #[test]
    fn identity_title_from_resource_uses_name_colon_key() {
        let r = sample_resource("shell", "workspace");
        assert_eq!(identity_title_from_resource(&r), "shell:workspace");
    }

    #[test]
    fn identity_title_from_resource_falls_back_to_id() {
        let mut r = sample_resource("shell", "workspace");
        r.metadata.terminal_name = None;
        assert_eq!(identity_title_from_resource(&r), "term_1");
    }

    fn assert_stop_detached(
        action: PolicyAction,
        detail: DetachedDetail,
        reattach_available: bool,
    ) {
        match action {
            PolicyAction::StopDetached {
                detail: d,
                reattach_available: r,
            } => {
                assert_eq!(d, detail);
                assert_eq!(r, reattach_available);
            }
            other => panic!("expected StopDetached, got {other:?}"),
        }
    }

    #[test]
    fn close_cause_policy_table() {
        let now = Instant::now();
        let mut state = PolicyState::new();

        assert_stop_detached(
            state.on_close(CloseCause::TerminalNotFound, now),
            DetachedDetail::TerminalGone,
            false,
        );
        assert_stop_detached(
            state.on_close(CloseCause::TerminalDetached, now),
            DetachedDetail::ClientDetached,
            true,
        );

        let mut retry_state = PolicyState::new();
        assert!(matches!(
            retry_state.on_close(CloseCause::Internal, now),
            PolicyAction::Retry { .. }
        ));
        assert!(matches!(
            retry_state.on_close(CloseCause::Network, now),
            PolicyAction::Retry { .. }
        ));

        let mut auth_state = PolicyState::new();
        assert!(matches!(
            auth_state.on_close(CloseCause::Unauthorized, now),
            PolicyAction::DowngradeReadOnly
        ));
    }

    #[test]
    fn unauthorized_twice_detaches() {
        let now = Instant::now();
        let mut state = PolicyState::new();
        assert!(matches!(
            state.on_close(CloseCause::Unauthorized, now),
            PolicyAction::DowngradeReadOnly
        ));
        assert_stop_detached(
            state.on_close(CloseCause::Unauthorized, now),
            DetachedDetail::Unauthorized,
            false,
        );
    }

    #[test]
    fn retry_window_boundaries_with_injected_now() {
        let t0 = Instant::now();
        let mut w = RetryWindow::new();
        let d0 = w.next_delay(t0).expect("first retry");
        assert!(d0 <= Duration::from_secs(30));

        let t_almost = t0 + Duration::from_millis(29_999);
        let d = w.next_delay(t_almost).expect("inside window");
        let remaining = (t0 + Duration::from_secs(30)).saturating_duration_since(t_almost);
        assert!(d <= remaining);

        assert!(w.next_delay(t0 + Duration::from_secs(30)).is_none());

        w.reset();
        assert!(w.next_delay(t0 + Duration::from_secs(60)).is_some());
    }

    #[test]
    fn non_exhaustive_unknown_maps_to_network_retry() {
        // Compile-exhaustiveness guard: `CloseCause` is `#[non_exhaustive]` and
        // cross-crate-unconstructible — we cannot fake an unknown variant at runtime.
        // The `_ => retry_or_exhausted` arm in `PolicyState::on_close` ensures future
        // variants conservatively follow the Network-retry path.
        fn maps_to_retry(cause: CloseCause, now: Instant) -> bool {
            let mut state = PolicyState::new();
            match cause {
                CloseCause::TerminalNotFound
                | CloseCause::TerminalDetached
                | CloseCause::Unauthorized => false,
                CloseCause::Internal | CloseCause::Network => {
                    matches!(state.on_close(cause, now), PolicyAction::Retry { .. })
                }
                _ => matches!(state.on_close(cause, now), PolicyAction::Retry { .. }),
            }
        }
        let now = Instant::now();
        assert!(maps_to_retry(CloseCause::Internal, now));
        assert!(maps_to_retry(CloseCause::Network, now));
    }

    #[test]
    fn engine_spawn_error_converts_to_engine_spawn_failed() {
        let err =
            crate::engine::EngineSpawnError::from_io_for_test(std::io::Error::other("no threads"));
        let detail: DetachedDetail = err.into();
        assert_eq!(detail, DetachedDetail::EngineSpawnFailed);
    }
}
