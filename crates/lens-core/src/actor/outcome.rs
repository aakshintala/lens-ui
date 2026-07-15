//! Unified actor → foreground outcome channel (Task 9 extends this).

use crate::actor::api::CommandOutcome;
use crate::actor::transport::{ActorTransport, ParkReason};
use lens_client::error::ClientError;
use std::collections::VecDeque;

/// Unified actor → foreground outcome channel. `Parked` is the sole terminal
/// disconnect outcome — the actor thread exits and recovery is a fresh respawn.
#[derive(Clone, Debug)]
pub enum ActorOutcome {
    Command(CommandOutcome),
    PersistError {
        where_: &'static str,
        message: String,
    },
    TransportChanged {
        transport: ActorTransport,
        reconcile_in_flight: bool,
    },
    Parked {
        reason: ParkReason,
    },
    /// D21: in-loop sleep succeeded — lifecycle flushed `Slept`, actor stopping.
    Slept,
    /// D21: sleep declined — session not quiescent; actor continues.
    SleepDeclined,
    /// Unified feed receiver dropped — Summary emit failed; actor continues.
    FeedConsumerGone,
    /// Optimistic bubble confirmed lost after reconnect held reconcile (D28).
    SendLost {
        lens_pending_id: String,
        content: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mapped {
    NetworkTransient, // Network — non-fatal, send rolled back
    ServerTransient,  // Server 5xx — retry-eligible, keep
    ReAuth,           // Auth 401
    LostAccess,       // Auth 403
    Tombstone,        // NotFound 404
    Denied,           // Server other-4xx
    WrongVersion,     // ContractMismatch
    DecodeDrift,      // Parse
    /// Stream-open failure — the actor never starts (`send_event` cannot return this).
    Fatal, // ThreadSpawn
}

impl Mapped {
    /// Per Table B: does this error mean the optimistic send definitively won't land?
    pub fn rolls_back_send(&self) -> bool {
        matches!(
            self,
            Mapped::NetworkTransient | Mapped::LostAccess | Mapped::Tombstone | Mapped::Denied
        )
    }
}

/// Map `ClientError` on the command/REST path to Table B semantics (design §13.1).
///
/// `ThreadSpawn` is a stream-open failure — the actor never starts — and cannot
/// arrive from `send_event`; mapping it here is for the future stream-attach site.
/// No `Ws` variant — the WS terminal path is deferred (design §13.1).
pub fn map_client_error(e: &ClientError) -> Mapped {
    match e {
        ClientError::Network(_) => Mapped::NetworkTransient,
        ClientError::Auth { status: 401 } => Mapped::ReAuth,
        ClientError::Auth { .. } => Mapped::LostAccess, // 403 (and any other Auth status)
        ClientError::NotFound { .. } => Mapped::Tombstone,
        ClientError::Server { status, .. } if *status >= 500 => Mapped::ServerTransient,
        ClientError::Server { .. } => Mapped::Denied, // other 4xx
        ClientError::ContractMismatch { .. } => Mapped::WrongVersion,
        ClientError::Parse(_) => Mapped::DecodeDrift,
        ClientError::ThreadSpawn(_) => Mapped::Fatal,
    }
}

/// Bounded, actor-owned diagnostic buffer. `push` NEVER blocks (drops the oldest
/// when full) so persist/emit paths can record errors without back-pressuring the
/// actor thread. Drained into the outcome channel after each event batch.
pub struct OutcomeRing {
    buf: VecDeque<ActorOutcome>,
    cap: usize,
}

impl OutcomeRing {
    pub fn with_cap(cap: usize) -> Self {
        Self {
            buf: VecDeque::new(),
            cap,
        }
    }

    pub fn push(&mut self, o: ActorOutcome) {
        if self.buf.len() == self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(o);
    }

    /// Drain via `send`; stop on first `false` (e.g. channel full), leave remainder.
    pub fn try_drain<F>(&mut self, mut send: F)
    where
        F: FnMut(ActorOutcome) -> bool,
    {
        while let Some(o) = self.buf.front().cloned() {
            if !send(o) {
                break;
            }
            self.buf.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn table_b_network_maps_to_network_transient() {
        assert_eq!(
            map_client_error(&ClientError::network_for_test()),
            Mapped::NetworkTransient
        );
    }

    #[test]
    fn table_b_auth_401_maps_to_re_auth() {
        assert_eq!(
            map_client_error(&ClientError::Auth { status: 401 }),
            Mapped::ReAuth
        );
    }

    #[test]
    fn table_b_auth_403_maps_to_lost_access() {
        assert_eq!(
            map_client_error(&ClientError::Auth { status: 403 }),
            Mapped::LostAccess
        );
    }

    #[test]
    fn table_b_not_found_maps_to_tombstone() {
        assert_eq!(
            map_client_error(&ClientError::NotFound {
                what: "session".into()
            }),
            Mapped::Tombstone
        );
    }

    #[test]
    fn table_b_server_5xx_is_server_transient() {
        assert_eq!(
            map_client_error(&ClientError::Server {
                status: 503,
                body: json!({})
            }),
            Mapped::ServerTransient
        );
    }

    #[test]
    fn table_b_server_400_is_denied() {
        assert_eq!(
            map_client_error(&ClientError::Server {
                status: 400,
                body: json!({})
            }),
            Mapped::Denied
        );
    }

    #[test]
    fn table_b_server_422_is_denied() {
        assert_eq!(
            map_client_error(&ClientError::Server {
                status: 422,
                body: json!({})
            }),
            Mapped::Denied
        );
    }

    #[test]
    fn table_b_contract_mismatch_maps_to_wrong_version() {
        assert_eq!(
            map_client_error(&ClientError::ContractMismatch {
                expected: "0.4.0",
                actual: "0.3.0".into(),
            }),
            Mapped::WrongVersion
        );
    }

    #[test]
    fn table_b_parse_maps_to_decode_drift() {
        assert_eq!(
            map_client_error(&ClientError::from(
                serde_json::from_str::<i32>("not-json").unwrap_err()
            )),
            Mapped::DecodeDrift
        );
    }

    #[test]
    fn table_b_thread_spawn_maps_to_fatal() {
        assert_eq!(
            map_client_error(&ClientError::ThreadSpawn("spawn failed".into())),
            Mapped::Fatal
        );
    }

    #[test]
    fn rolls_back_send_truth_table() {
        assert!(Mapped::NetworkTransient.rolls_back_send());
        assert!(Mapped::LostAccess.rolls_back_send());
        assert!(Mapped::Tombstone.rolls_back_send());
        assert!(Mapped::Denied.rolls_back_send());

        assert!(!Mapped::ServerTransient.rolls_back_send());
        assert!(!Mapped::ReAuth.rolls_back_send());
        assert!(!Mapped::WrongVersion.rolls_back_send());
        assert!(!Mapped::DecodeDrift.rolls_back_send());
        assert!(!Mapped::Fatal.rolls_back_send());
    }
}
