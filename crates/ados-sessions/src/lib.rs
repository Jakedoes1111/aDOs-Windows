//! Sessions module for aDOs: one list of every session from every agent backend in
//! `backends.toml`, opened, continued and watched from one panel.
//! Dock panel name: `ados_core::ids::SESSIONS`.

mod backends;
mod list;
pub mod model;
mod panel;
mod questions;
pub mod state;
mod view;

pub use panel::{init, SessionsPanel};
