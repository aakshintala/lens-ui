use crate::error::{ClientError, Result};

/// Exact-match contract gate. Coarse on dev0 (the version string is identical
/// across commits — see typed-client-implementation.md D4); real drift
/// detection is the startup taxonomy diff + `xtask drift`, planned later.
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

    #[test]
    fn contract_gate_accepts_exact_match() {
        assert!(check_contract("0.3.0.dev0", "0.3.0.dev0").is_ok());
    }

    #[test]
    fn contract_gate_rejects_mismatch() {
        let err = check_contract("0.3.0.dev0", "0.2.0").unwrap_err();
        match err {
            crate::error::ClientError::ContractMismatch { expected, actual } => {
                assert_eq!(expected, "0.3.0.dev0");
                assert_eq!(actual, "0.2.0");
            }
            other => panic!("wrong error: {other:?}"),
        }
    }
}
