//! `ados-mcp`: aDOs's own MCP server, so every agent session aDOs starts can act in aDOs
//! (VISION step 3, design in `docs/design/backends.md`).
//!
//! - [`host::start`]: called once by the shell at launch.
//! - [`bridge::run`]: the stdio shim agents run (`ados mcp-bridge`).
//! - [`protocol`] / [`tools`]: MCP JSON-RPC routing and the tool list (pure, tested).
pub mod bridge;
#[cfg(feature = "browser")]
mod browser_tools;
mod data_tools;
pub mod host;
pub mod protocol;
pub mod server;
mod session_tools;
mod shell_tools;
pub mod tools;

pub use host::{start, Hooks};
