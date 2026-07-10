use crate::error::{ClientError, Result};
use serde::de::DeserializeOwned;

/// Decode a JSON response body by HTTP status, mapping to the typed error model.
/// Pure over (stage, status, body) so it is unit-testable without a live server.
/// - 2xx: parse `body` as `T` (a JSON-decode failure is a real `Parse` error)
/// - 401/403: `Auth { status }`
/// - anything else: `Server { status, body }` (body kept as JSON if it parses, else wrapped)
pub(crate) fn decode_json<T: DeserializeOwned>(stage: &str, status: u16, body: &str) -> Result<T> {
    match status {
        200..=299 => serde_json::from_str(body).map_err(ClientError::Parse),
        401 | 403 => Err(ClientError::Auth { status }),
        _ => {
            let body = serde_json::from_str::<serde_json::Value>(body)
                .unwrap_or_else(|_| serde_json::json!({ "stage": stage, "body": body }));
            Err(ClientError::Server { status, body })
        }
    }
}

/// Map a non-body stage's status to the typed error model. Ok(()) on 2xx.
pub(crate) fn check_status(stage: &str, status: u16) -> Result<()> {
    match status {
        200..=299 => Ok(()),
        401 | 403 => Err(ClientError::Auth { status }),
        _ => Err(ClientError::Server {
            status,
            body: serde_json::json!({ "stage": stage }),
        }),
    }
}

/// Exact-match contract gate against the pinned package semver (e.g. `0.5.1`).
/// Coarse by design: it catches a release-level mismatch but not intra-release
/// drift (commits sharing a version string — see typed-client-implementation.md
/// D4). Real shape-level drift detection is the startup taxonomy diff +
/// `xtask drift`.
pub fn check_contract(expected: &'static str, actual: &str) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(ClientError::ContractMismatch {
            expected,
            actual: actual.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::info::VersionResponse;

    #[test]
    fn decode_json_ok_on_2xx() {
        let ver: VersionResponse = decode_json("version", 200, r#"{"version":"0.5.1"}"#).unwrap();
        assert_eq!(ver.version, "0.5.1");
    }

    #[test]
    fn decode_json_parse_error_on_malformed_2xx_body() {
        let err = decode_json::<VersionResponse>("version", 200, "not json").unwrap_err();
        match err {
            ClientError::Parse(_) => {}
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn decode_json_auth_on_401_and_403() {
        let err401 = decode_json::<VersionResponse>("version", 401, "").unwrap_err();
        match err401 {
            ClientError::Auth { status: 401 } => {}
            other => panic!("wrong error: {other:?}"),
        }
        let err403 = decode_json::<VersionResponse>("version", 403, "").unwrap_err();
        match err403 {
            ClientError::Auth { status: 403 } => {}
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn decode_json_server_with_json_body() {
        let err =
            decode_json::<VersionResponse>("version", 500, r#"{"detail":"boom"}"#).unwrap_err();
        match err {
            ClientError::Server { status: 500, body } => {
                assert_eq!(body["detail"], "boom");
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn decode_json_server_wraps_non_json_body() {
        let err = decode_json::<VersionResponse>("version", 500, "<html>oops").unwrap_err();
        match err {
            ClientError::Server { status: 500, body } => {
                assert_eq!(body["stage"], "version");
                assert_eq!(body["body"], "<html>oops");
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn check_status_ok_on_2xx() {
        assert!(check_status("health", 200).is_ok());
    }

    #[test]
    fn check_status_server_on_non_2xx() {
        let err = check_status("health", 503).unwrap_err();
        match err {
            ClientError::Server { status: 503, body } => {
                assert_eq!(body["stage"], "health");
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn check_status_auth_on_401() {
        let err = check_status("health", 401).unwrap_err();
        match err {
            ClientError::Auth { status: 401 } => {}
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn contract_gate_accepts_exact_match() {
        assert!(check_contract("0.5.1", "0.5.1").is_ok());
    }

    #[test]
    fn contract_gate_rejects_mismatch() {
        let err = check_contract("0.5.1", "0.3.0").unwrap_err();
        match err {
            crate::error::ClientError::ContractMismatch { expected, actual } => {
                assert_eq!(expected, "0.5.1");
                assert_eq!(actual, "0.3.0");
            }
            other => panic!("wrong error: {other:?}"),
        }
    }
}
