//! envsync-core: Windows user environment variables, shared with WSL through WSLENV.
//!
//! Knows nothing about MCP. Values are written and checked here but never printed.

pub mod cli;
pub mod install;
pub mod prompt;
pub mod store;
pub mod ui;

pub use store::{valid_name, Store, TEST_STORE_VAR};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
