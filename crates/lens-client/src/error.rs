use thiserror::Error;

pub type Result<T> = std::result::Result<T, ClientError>;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("auth failed (status {status})")]
    Auth { status: u16 },

    #[error("not found: {what}")]
    NotFound { what: String },

    #[error("server error (status {status}): {body}")]
    Server {
        status: u16,
        body: serde_json::Value,
    },

    #[error("contract mismatch: expected {expected}, server reports {actual}")]
    ContractMismatch {
        expected: &'static str,
        actual: String,
    },

    #[error("parse error: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("reader thread spawn failed: {0}")]
    ThreadSpawn(String),
}

impl ClientError {
    /// A transport/connection failure — the server was unreachable (vs. it
    /// answering with a domain error).
    pub fn is_transport(&self) -> bool {
        matches!(self, ClientError::Network(_))
    }

    /// A response the client could not decode against the pinned contract —
    /// itself a drift signal (vs. an unreachable server).
    pub fn is_decode(&self) -> bool {
        matches!(
            self,
            ClientError::Parse(_) | ClientError::ContractMismatch { .. }
        )
    }
}

#[cfg(feature = "test-util")]
impl ClientError {
    /// Build a `Network` error WITHOUT any network I/O — for downstream tests
    /// that need to exercise the transport-error branch. Uses a URL that fails
    /// at request-build time (no socket opened).
    pub fn network_for_test() -> Self {
        // A relative/unparseable URL errors synchronously in the builder — no I/O.
        let err = reqwest::blocking::Client::new()
            .get("http://")
            .build()
            .unwrap_err();
        ClientError::Network(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_and_decode_predicates() {
        assert!(!ClientError::Auth { status: 401 }.is_transport());
        assert!(!ClientError::NotFound { what: "x".into() }.is_decode());
        assert!(
            ClientError::ContractMismatch {
                expected: "0.5.1",
                actual: "0.3.0".into(),
            }
            .is_decode()
        );
        assert!(ClientError::from(serde_json::from_str::<i32>("x").unwrap_err()).is_decode());
    }

    #[test]
    fn contract_mismatch_displays_expected_and_actual() {
        let e = ClientError::ContractMismatch {
            expected: "0.5.1",
            actual: "0.3.0".into(),
        };
        let s = e.to_string();
        assert!(s.contains("0.5.1"), "got: {s}");
        assert!(s.contains("0.3.0"), "got: {s}");
    }
}
