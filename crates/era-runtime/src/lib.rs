//! Caller-pumped, transport-neutral Era game runtime.
//!
//! The runtime owns the aggregate game state but performs no filesystem, clock,
//! rendering, audio, or operating-system input work. Those operations cross the
//! versioned frontend protocol and are committed only after correlated responses.

mod host;
mod presentation;
mod project;
mod session;

pub use session::{
    RuntimeDriveBudget, RuntimeDriveReport, RuntimeDriveState, RuntimeError, RuntimeOptions,
    RuntimeSession,
};
