//! Private helpers shared by the runtime session protocol and VM-driving paths.
//!
//! Keeping these stateless operations separate makes the state machine in the
//! parent module easier to review without exposing new public API.

#[cfg(test)]
mod color_tests;
mod host;
mod input;
mod protocol;
mod text;
mod variables;

pub(super) use host::*;
pub(super) use input::*;
pub(super) use protocol::*;
pub(super) use text::*;
pub(super) use variables::*;
