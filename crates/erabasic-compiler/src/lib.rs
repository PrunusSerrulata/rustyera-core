//! Deterministic lowering from typed HIR to validated VM bytecode.

mod compile;
mod diagnostic;
mod lowering;
mod options;
mod registry;

pub use compile::{CompileReport, CompileStats, IncrementalState, compile_project};
pub use diagnostic::{CompilerDiagnostic, CompilerDiagnosticCode};
pub use options::{CompilerOptions, OptimizationLevel};
pub use registry::{HostBinding, HostRegistry, default_host_registry};
