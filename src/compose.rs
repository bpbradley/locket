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
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::{
    FmtContext, FormatFields,
    format::{FormatEvent, Writer},
};
use tracing_subscriber::registry::LookupSpan;

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

    #[error("Metadata generation failed: {0}")]
    Metadata(String),
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

    pub fn set_env(key: &str, value: &str) {
        Self::emit(MessageType::SetEnv, format!("{}={}", key, value));
    }
}

pub struct ComposeFormatter;

impl<S, N> FormatEvent<S, N> for ComposeFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        let meta = event.metadata();
        let level = *meta.level();

        let msg_type = match level {
            Level::ERROR | Level::WARN => MessageType::Error,
            Level::INFO => MessageType::Info,
            _ => MessageType::Debug,
        };

        let mut message = String::new();
        let mut visitor = MessageVisitor(&mut message);
        event.record(&mut visitor);

        let payload = ComposeResponse { msg_type, message };

        let json = serde_json::to_string(&payload).map_err(|_| std::fmt::Error)?;
        writeln!(writer, "{}", json)
    }
}

// Helper visitor to extract the message string from the event
struct MessageVisitor<'a>(&'a mut String);

impl<'a> Visit for MessageVisitor<'a> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            use std::fmt::Write;
            let _ = write!(self.0, "{:?}", value);
        }
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.push_str(value);
        }
    }
}
