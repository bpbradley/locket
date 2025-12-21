//! # locket
//!
//! `locket` is a secret management agent and helper library designed to orchestrate
//! secrets for dependent applications. It creates a bridge between secret providers
//! and applications by injecting secrets into configuration files or environment variables.
//!
//! ## Feature Flags
//!
//! * `op`: Enables the 1Password Service Account provider.
//! * `connect`: Enables the 1Password Connect provider.
//! * `bws`: Enables the Bitwarden Secrets Manager provider.
//! * `compose`: Enables Docker CLI Plugin for use as a Docker Compose Provider service
//! * `exec`: Enables the `exec` command for process environment injection into a child process
pub mod cmd;
#[cfg(feature = "compose")]
pub mod compose;
#[cfg(any(feature = "exec", feature = "compose"))]
pub mod env;
pub mod events;
pub mod health;
pub mod logging;
pub mod path;
#[cfg(feature = "exec")]
pub mod process;
pub mod provider;
pub mod secrets;
pub mod template;
pub mod watch;
pub mod write;
