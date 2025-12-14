//! Docker Compose provider communication and error handling.
//!
//! This module implements the communication protocol
//! used by Docker Compose to interact with provider plugins.
//! It defines structured messages for info, error, debug, and environment variable setting.
//! It also defines a `ComposeError` enum for error handling
//! and a `ComposeMsg` struct for emitting messages to stdout in the expected JSON format.
use crate::provider::ProviderError;
use crate::secrets::SecretError;
use serde::Serialize;
use std::io::Write;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ComposeError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),

    #[error("secret error: {0}")]
    Secret(#[from] SecretError),

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Invalid Args: {0}")]
    Argument(String),
}

impl ComposeError {
    pub fn report(&self) -> sysexits::ExitCode {
        ComposeMsg::error(self);
        eprintln!("[ERROR] Details: {:?}", self);
        sysexits::ExitCode::DataErr
    }
}

#[derive(Debug, Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum MessageType {
    Info,
    Error,
    Debug,
    SetEnv,
}

#[derive(Serialize)]
struct ComposeResponse {
    #[serde(rename = "type")]
    msg_type: MessageType,
    message: String,
}

pub struct ComposeMsg;

impl ComposeMsg {
    fn emit(msg_type: MessageType, message: impl Into<String>) {
        let payload = ComposeResponse {
            msg_type,
            message: message.into(),
        };

        let stdout = std::io::stdout();
        let mut handle = stdout.lock();

        if let Ok(json) = serde_json::to_string(&payload) {
            let _ = writeln!(handle, "{}", json);
            let _ = handle.flush();
        }
    }

    pub fn info(msg: impl std::fmt::Display) {
        Self::emit(MessageType::Info, msg.to_string());
    }

    pub fn debug(msg: impl std::fmt::Display) {
        Self::emit(MessageType::Debug, msg.to_string());
    }

    pub fn error(msg: impl std::fmt::Display) {
        Self::emit(MessageType::Error, msg.to_string());
    }

    pub fn set_env(key: &str, value: &str) {
        Self::emit(MessageType::SetEnv, format!("{}={}", key, value));
    }
}
