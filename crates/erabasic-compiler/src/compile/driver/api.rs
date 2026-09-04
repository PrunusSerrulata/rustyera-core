use erabasic_analyzer::AnalyzedProject;
use erabasic_bytecode::BytecodeArtifact;

use super::{CompilePolicy, ProjectInput, compile_project_inner};
use crate::{
    CompileProgressCallback, CompileReport, CompilerOptions, HostRegistry, IncrementalState,
    OwnedValidatedCompileReport, ValidatedCompileReport,
};
#[must_use]
/// Compile one analyzed, in-memory project into a self-contained artifact.
///
/// # Panics
///
/// Panics only if the crate's own fixed, Serde-derived identity tuples stop being
/// serializable. User-provided source and project values are reported as diagnostics.
pub fn compile_project(
    project: &AnalyzedProject,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
) -> CompileReport {
    compile_project_inner(
        ProjectInput::Borrowed(project),
        options,
        host_registry,
        previous,
        None,
        CompilePolicy::RETAINED_INCREMENTAL,
        None,
    )
    .report
    .into()
}

/// Compile with an exact previous artifact backing a compact incremental cache.
///
/// Runtime owners use this entry point because they already retain the executable
/// artifact. The returned cache is compact and therefore must again be paired with
/// its exact artifact on the next incremental build.
#[must_use]
pub fn compile_project_with_artifact(
    project: &AnalyzedProject,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
) -> CompileReport {
    compile_project_inner(
        ProjectInput::Borrowed(project),
        options,
        host_registry,
        previous,
        previous_artifact,
        CompilePolicy::COMPACT_INCREMENTAL,
        None,
    )
    .report
    .into()
}

#[must_use]
pub fn compile_project_with_artifact_and_progress(
    project: &AnalyzedProject,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    progress: &dyn CompileProgressCallback,
) -> CompileReport {
    compile_project_inner(
        ProjectInput::Borrowed(project),
        options,
        host_registry,
        previous,
        previous_artifact,
        CompilePolicy::COMPACT_INCREMENTAL,
        Some(progress),
    )
    .report
    .into()
}

/// Compile for a runtime that must preserve the compiler's validation provenance.
#[must_use]
pub fn compile_validated_project_with_artifact(
    project: &AnalyzedProject,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
) -> ValidatedCompileReport {
    compile_project_inner(
        ProjectInput::Borrowed(project),
        options,
        host_registry,
        previous,
        previous_artifact,
        CompilePolicy::COMPACT_INCREMENTAL,
        None,
    )
    .report
}

/// Compile with progress while preserving validator provenance for the runtime.
#[must_use]
pub fn compile_validated_project_with_artifact_and_progress(
    project: &AnalyzedProject,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    progress: &dyn CompileProgressCallback,
) -> ValidatedCompileReport {
    compile_project_inner(
        ProjectInput::Borrowed(project),
        options,
        host_registry,
        previous,
        previous_artifact,
        CompilePolicy::COMPACT_INCREMENTAL,
        Some(progress),
    )
    .report
}

/// Compile an owned analyzed project while moving its large data tables into the artifact.
///
/// Runtime cold loads use this path after analyzer diagnostics no longer need the HIR owner.
#[must_use]
pub fn compile_owned_validated_project_with_artifact(
    project: AnalyzedProject,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
) -> OwnedValidatedCompileReport {
    compile_project_inner(
        ProjectInput::Owned(Box::new(project)),
        options,
        host_registry,
        previous,
        previous_artifact,
        CompilePolicy::compact_owned(),
        None,
    )
}

/// Compile an owned analyzed project with progress and without cloning artifact-owned tables.
#[must_use]
pub fn compile_owned_validated_project_with_artifact_and_progress(
    project: AnalyzedProject,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    progress: &dyn CompileProgressCallback,
) -> OwnedValidatedCompileReport {
    compile_project_inner(
        ProjectInput::Owned(Box::new(project)),
        options,
        host_registry,
        previous,
        previous_artifact,
        CompilePolicy::compact_owned(),
        Some(progress),
    )
}
