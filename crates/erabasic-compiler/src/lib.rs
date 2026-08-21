//! Deterministic lowering from typed HIR to validated VM bytecode.

mod compile;
mod diagnostic;
mod lowering;
mod options;
mod registry;

pub use compile::{
    CompileProgress, CompileProgressCallback, CompileProgressStage, CompileReport, CompileStats,
    IncrementalState, OwnedValidatedCompileReport, ValidatedCompileReport,
    compile_owned_validated_project_with_artifact,
    compile_owned_validated_project_with_artifact_and_progress, compile_project,
    compile_project_with_artifact, compile_project_with_artifact_and_progress,
    compile_validated_project_with_artifact, compile_validated_project_with_artifact_and_progress,
};
pub use diagnostic::{CompilerDiagnostic, CompilerDiagnosticCode, CompilerDiagnosticSeverity};
pub use options::{CompilerOptions, OptimizationLevel};
pub use registry::{
    ExecutionBinding, HostBinding, HostRegistry, default_host_registry, extension_binding,
};
