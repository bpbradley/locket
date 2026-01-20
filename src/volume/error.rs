use crate::error::LocketError;
use http_body_util::Full;
use hyper::{Response, StatusCode, body::Bytes};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error(transparent)]
    Locket(#[from] LocketError),

    #[error("json parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("volume not found")]
    NotFound,

    #[error("volume in use")]
    InUse,

    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Serialize)]
struct DockerErrorResponse {
    #[serde(rename = "Err")]
    err: String,
}

impl PluginError {
    pub fn into_response(self) -> Response<Full<Bytes>> {
        let err_msg = self.to_string();

        tracing::error!(error = %err_msg, "plugin request failed");

        let body = DockerErrorResponse { err: err_msg };
        let json = serde_json::to_vec(&body)
            .unwrap_or_else(|_| b"{\"Err\":\"Internal Serialization Error\"}".to_vec());

        Response::builder()
            .header("Content-Type", "application/json")
            .status(StatusCode::OK)
            .body(Full::new(Bytes::from(json)))
            .unwrap()
    }
}

impl From<std::io::Error> for PluginError {
    fn from(e: std::io::Error) -> Self {
        PluginError::Locket(LocketError::Io(e))
    }
}
