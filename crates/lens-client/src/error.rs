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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_mismatch_displays_expected_and_actual() {
        let e = ClientError::ContractMismatch {
            expected: "0.3.0.dev0",
            actual: "0.2.0".into(),
        };
        let s = e.to_string();
        assert!(s.contains("0.3.0.dev0"), "got: {s}");
        assert!(s.contains("0.2.0"), "got: {s}");
    }
}
