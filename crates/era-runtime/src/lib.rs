//! Caller-pumped, transport-neutral Era game runtime.
//!
//! The runtime owns the aggregate game state but performs no filesystem, clock,
//! rendering, audio, or operating-system input work. Those operations cross the
//! versioned frontend protocol and are committed only after correlated responses.

mod compiled_cache;
mod controller;
mod host;
mod key_macro;
mod operation;
mod presentation;
mod project;
mod resource;
mod runtime_snapshot;
mod save_adapter;
mod session;

pub use compiled_cache::{CompiledProjectCacheError, decode_compiled_project_manifest};
pub use session::{
    RuntimeDriveBudget, RuntimeDriveReport, RuntimeDriveState, RuntimeError, RuntimeOptions,
    RuntimeSession,
};
