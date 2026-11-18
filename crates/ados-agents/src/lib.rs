//! `ados-agents`: aDOs as an ACP client (see `docs/design/backends.md`).
//!
//! - [`config`]: the backend list in `ADOS_HOME/backends.toml`.
//! - [`Backend`]: one running agent process and its ACP connection.
//! - [`Registry`] / [`shared`]: one shared, idle-stopped process per backend for all of aDOs.
//! - [`AgentEvent`]: what the agent streams back, as plain Rust values.

pub mod config;
pub mod launch;
pub mod mcp;
pub mod registry;
pub mod started;
mod backend;
mod event;
mod files;
mod shared;

pub use backend::{is_not_logged_in, Backend, BackendInfo, EventHook, NotLoggedIn, SessionSummary};
pub use config::BackendConfig;
pub use mcp::{ados_servers, McpEntry};
pub use registry::{global, shared, Lease, Page, Permission, Registry, Sink, Status, Subscription};
pub use shared::SharedBackend;
pub use event::{
    deny, AgentEvent, PermissionCallback, PermissionChoice, PermissionKind, PermissionRequest,
    StopReason, ToolStatus,
};
