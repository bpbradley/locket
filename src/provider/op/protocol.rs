//! Wire types for protocol between locket and op bridge.

use crate::provider::ProviderError;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub(super) const PROTOCOL_VERSION: u32 = 1;

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(super) enum Request<'a> {
    Init {
        id: u64,
        protocol: u32,
        token: &'a str,
    },
    Resolve {
        id: u64,
        refs: &'a [&'a str],
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(super) enum Response {
    InitOk {
        id: u64,
        protocol: u32,
        bridge_version: String,
    },
    ResolveOk {
        id: u64,
        results: HashMap<String, ResolveResult>,
    },
    Error {
        id: u64,
        code: ErrorCode,
        message: String,
    },
}

impl Response {
    pub(super) fn id(&self) -> u64 {
        match self {
            Response::InitOk { id, .. }
            | Response::ResolveOk { id, .. }
            | Response::Error { id, .. } => *id,
        }
    }
}

/// Exactly one of `secret` or `error` is present per reference.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum ResolveResult {
    Resolved { secret: SecretString },
    Failed { error: BridgeError },
}

#[derive(Debug, Deserialize)]
pub(super) struct BridgeError {
    pub code: ErrorCode,
    pub message: String,
}

impl BridgeError {
    pub(super) fn into_provider_error(self) -> ProviderError {
        self.code.into_provider_error(self.message)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ErrorCode {
    NotFound,
    RateLimited,
    InvalidReference,
    UnsupportedProtocol,
    BadRequest,
    Internal,
    #[serde(other)]
    Other,
}

impl ErrorCode {
    pub(super) fn into_provider_error(self, message: String) -> ProviderError {
        match self {
            ErrorCode::NotFound => ProviderError::NotFound(message),
            ErrorCode::RateLimited => ProviderError::RateLimit,
            ErrorCode::UnsupportedProtocol => ProviderError::InvalidConfig(message),
            ErrorCode::InvalidReference
            | ErrorCode::BadRequest
            | ErrorCode::Internal
            | ErrorCode::Other => ProviderError::Other(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn init_request_serializes_to_wire_format() {
        let json = serde_json::to_string(&Request::Init {
            id: 1,
            protocol: 1,
            token: "ops_abc",
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"type":"init","id":1,"protocol":1,"token":"ops_abc"}"#
        );
    }

    #[test]
    fn resolve_request_serializes_to_wire_format() {
        let refs = ["op://v/i/f", "op://v/i/s/f?ssh-format=openssh"];
        let json = serde_json::to_string(&Request::Resolve { id: 2, refs: &refs }).unwrap();
        assert_eq!(
            json,
            r#"{"type":"resolve","id":2,"refs":["op://v/i/f","op://v/i/s/f?ssh-format=openssh"]}"#
        );
    }

    #[test]
    fn init_ok_deserializes() {
        let resp: Response = serde_json::from_str(
            r#"{"type":"init-ok","id":1,"protocol":1,"bridge_version":"1.2.3"}"#,
        )
        .unwrap();
        match resp {
            Response::InitOk {
                id,
                protocol,
                bridge_version,
            } => {
                assert_eq!((id, protocol, bridge_version.as_str()), (1, 1, "1.2.3"));
            }
            other => panic!("expected InitOk, got {other:?}"),
        }
    }

    #[test]
    fn resolve_ok_deserializes_mixed_results() {
        let resp: Response = serde_json::from_str(
            r#"{"type":"resolve-ok","id":2,"results":{
                "op://v/i/f":{"secret":"hunter2"},
                "op://v/missing/f":{"error":{"code":"not_found","message":"nope"}}
            }}"#,
        )
        .unwrap();
        let Response::ResolveOk { id, results } = resp else {
            panic!("expected ResolveOk");
        };
        assert_eq!(id, 2);
        match &results["op://v/i/f"] {
            ResolveResult::Resolved { secret } => assert_eq!(secret.expose_secret(), "hunter2"),
            other => panic!("expected Resolved, got {other:?}"),
        }
        match &results["op://v/missing/f"] {
            ResolveResult::Failed { error } => {
                assert_eq!(error.code, ErrorCode::NotFound);
                assert_eq!(error.message, "nope");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn unknown_error_code_deserializes_to_other() {
        let resp: Response = serde_json::from_str(
            r#"{"type":"error","id":3,"code":"some_future_code","message":"m"}"#,
        )
        .unwrap();
        let Response::Error { code, .. } = resp else {
            panic!("expected Error");
        };
        assert_eq!(code, ErrorCode::Other);
    }

    #[test]
    fn error_codes_map_to_provider_errors() {
        let cases = [
            (ErrorCode::NotFound, "secret not found: m"),
            (ErrorCode::RateLimited, "rate limited"),
            (ErrorCode::UnsupportedProtocol, "invalid config: m"),
            (ErrorCode::InvalidReference, "m"),
            (ErrorCode::BadRequest, "m"),
            (ErrorCode::Internal, "m"),
            (ErrorCode::Other, "m"),
        ];
        for (code, rendered) in cases {
            assert_eq!(
                code.into_provider_error("m".into()).to_string(),
                rendered,
                "wrong mapping for {code:?}"
            );
        }
    }
}
