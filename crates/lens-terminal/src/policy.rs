//! Discovery, attach, and resolve helpers (Slice 1d Task 3).

use std::sync::Arc;

use lens_client::error::ClientError;
use lens_client::ids::{SessionId, TerminalId};
use lens_client::{AttachOptions, Client, TerminalCreate, TerminalResource, attach};

use crate::bridge::{self, BridgeEvent};
use crate::engine::EngineHandle;
use crate::engine::vt::EngineConfig;
use crate::runtime::TerminalRuntime;
use crate::{AccessIntent, DetachedDetail, TerminalKey, TerminalOpenOptions, TerminalTarget};

/// Outcome of off-thread discover + attach, applied on the foreground.
pub(crate) struct AttachedParts {
    pub resource: TerminalResource,
    pub runtime: TerminalRuntime,
    pub wake_tx: async_channel::Sender<()>,
    pub wake_rx: async_channel::Receiver<()>,
    pub policy_rx: async_channel::Receiver<BridgeEvent>,
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
    let engine = Arc::new(EngineHandle::spawn(cfg));

    let (wake_tx, wake_rx) = async_channel::bounded(1);
    let (policy_tx, policy_rx) = async_channel::bounded(32);
    let bridge = bridge::spawn_bridge(
        attach_handle.inbound.clone(),
        attach_handle.outbound.clone(),
        Arc::clone(&engine),
        policy_tx,
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
#[expect(dead_code, reason = "consumed by Task 4 reconnect scheduling")]
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
    use super::*;
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
}
