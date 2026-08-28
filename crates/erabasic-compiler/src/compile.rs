use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use erabasic_analyzer::AnalyzedProject;
use erabasic_bytecode::{
    ArtifactManifest, BytecodeArtifact, BytecodeEventGroup, BytecodeFunction, BytecodeGlobal,
    BytecodePatch, Digest, HostImport, NativeImport, SourceMap, SourceMapEntry, SourceRecord,
    SymbolKey,
};
use erabasic_hir::{Function, SourceId};
use erabasic_validator::{
    ValidatedArtifact, ValidationContext, validate_compiler_output, validate_hir,
};
#[cfg(any(not(target_arch = "wasm32"), target_feature = "atomics"))]
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    CompilerDiagnostic, CompilerDiagnosticCode, CompilerOptions, HostRegistry,
    lowering::{
        FunctionSignature, LoweredFunction, LoweredSourceMapEntry, LoweringContext,
        LoweringProgram, lower_function,
    },
};

mod artifact;
mod incremental;

use artifact::{canonical_digest, event_groups, function_keys, globals, variable_keys};
use incremental::{
    PreviousArtifactIndex, cached_function, compact_cached_function, create_incremental_patch,
    materialize_cached_function, materialized_function,
};

mod call_dependencies;
mod driver;

pub use driver::{
    compile_owned_validated_project_with_artifact,
    compile_owned_validated_project_with_artifact_and_progress, compile_project,
    compile_project_with_artifact, compile_project_with_artifact_and_progress,
    compile_validated_project_with_artifact, compile_validated_project_with_artifact_and_progress,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct CachedFunction {
    cache_key: Digest,
    function_key: SymbolKey,
    function: Option<erabasic_bytecode::BytecodeFunction>,
    source_entries: Vec<LoweredSourceMapEntry>,
    native_imports: Vec<NativeImport>,
    host_imports: Vec<HostImport>,
}

enum FunctionBuild {
    Cached(MaterializedFunction),
    Lowered(LoweredFunction),
}

pub(crate) struct DenseIdIndex<T> {
    len: usize,
    values: Vec<Option<T>>,
}

impl<T> DenseIdIndex<T> {
    pub(crate) fn new(len: usize) -> Self {
        Self {
            len,
            values: Vec::new(),
        }
    }

    pub(crate) fn insert(&mut self, id: u32, value: T) {
        if let Some(slot) = self.slot_mut(id) {
            *slot = Some(value);
        }
    }

    pub(crate) fn get(&self, id: u32) -> Option<&T> {
        self.values.get(usize::try_from(id).ok()?)?.as_ref()
    }

    pub(crate) fn get_mut(&mut self, id: u32) -> Option<&mut T> {
        self.slot_mut(id)?.as_mut()
    }

    pub(crate) fn get_or_insert_with(
        &mut self,
        id: u32,
        value: impl FnOnce() -> T,
    ) -> Option<&mut T> {
        Some(self.slot_mut(id)?.get_or_insert_with(value))
    }

    pub(crate) fn take(&mut self, id: u32) -> Option<T> {
        self.slot_mut(id)?.take()
    }

    fn slot_mut(&mut self, id: u32) -> Option<&mut Option<T>> {
        let index = usize::try_from(id).ok()?;
        if index >= self.len {
            return None;
        }
        if self.values.is_empty() {
            self.values = std::iter::repeat_with(|| None).take(self.len).collect();
        }
        self.values.get_mut(index)
    }
}

impl FunctionBuild {
    fn source_entry_count(&self) -> usize {
        match self {
            Self::Cached(entry) => entry.source_entries.len(),
            Self::Lowered(entry) => entry.source_entries.len(),
        }
    }
}

fn compile_progress_stride(total: usize) -> usize {
    total.div_ceil(100).clamp(1, 64)
}

struct CompileProgressCounter<'a> {
    stage: CompileProgressStage,
    total: usize,
    completed: AtomicUsize,
    reported: AtomicUsize,
    callback_lock: Mutex<()>,
    callback: Option<&'a dyn CompileProgressCallback>,
}

impl<'a> CompileProgressCounter<'a> {
    fn new(
        stage: CompileProgressStage,
        total: usize,
        callback: Option<&'a dyn CompileProgressCallback>,
    ) -> Self {
        if let Some(callback) = callback {
            callback(CompileProgress {
                stage,
                completed: 0,
                total,
            });
        }
        Self {
            stage,
            total,
            completed: AtomicUsize::new(0),
            reported: AtomicUsize::new(0),
            callback_lock: Mutex::new(()),
            callback,
        }
    }

    fn advance(&self) {
        let completed = self.completed.fetch_add(1, Ordering::Relaxed) + 1;
        self.report(completed, false);
    }

    fn checkpoint(&self) {
        let completed = self.completed.fetch_add(1, Ordering::Relaxed) + 1;
        self.report(completed, true);
    }

    fn finish(&self) {
        self.completed.store(self.total, Ordering::Relaxed);
        self.report(self.total, true);
    }

    fn report(&self, completed: usize, force: bool) {
        let Some(callback) = self.callback else {
            return;
        };
        let previous = self.reported.load(Ordering::Relaxed);
        if !should_report_progress(completed, previous, self.total, force) {
            return;
        }
        let _guard = self
            .callback_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = self.reported.load(Ordering::Relaxed);
        if should_report_progress(completed, previous, self.total, force) {
            self.reported.store(completed, Ordering::Relaxed);
            callback(CompileProgress {
                stage: self.stage,
                completed,
                total: self.total,
            });
        }
    }
}

fn should_report_progress(completed: usize, previous: usize, total: usize, force: bool) -> bool {
    completed > previous
        && (force
            || completed.saturating_sub(previous) >= compile_progress_stride(total)
            || completed == total)
}

struct MaterializedFunction {
    cache_key: Digest,
    function: erabasic_bytecode::BytecodeFunction,
    source_entries: Vec<LoweredSourceMapEntry>,
    native_imports: Vec<NativeImport>,
    host_imports: Vec<HostImport>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct IncrementalBase {
    manifest: ArtifactManifest,
    metadata: Option<Box<IncrementalMetadata>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct IncrementalMetadata {
    call_compatibility: erabasic_bytecode::BytecodeCallCompatibility,
    project_data: erabasic_data::ProjectData,
    globals: Vec<BytecodeGlobal>,
    native_imports: Vec<NativeImport>,
    host_imports: Vec<HostImport>,
    event_groups: Vec<BytecodeEventGroup>,
}

impl IncrementalBase {
    fn from_artifact(artifact: &BytecodeArtifact, include_metadata: bool) -> Self {
        Self {
            manifest: artifact.manifest.clone(),
            metadata: include_metadata.then(|| {
                Box::new(IncrementalMetadata {
                    call_compatibility: artifact.call_compatibility,
                    project_data: artifact.project_data.clone(),
                    globals: artifact.globals.clone(),
                    native_imports: artifact.native_imports.clone(),
                    host_imports: artifact.host_imports.clone(),
                    event_groups: artifact.event_groups.clone(),
                })
            }),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct IncrementalState {
    compiler_abi: u32,
    functions: BTreeMap<SymbolKey, CachedFunction>,
    base: Option<IncrementalBase>,
}

impl IncrementalState {
    #[must_use]
    pub fn cached_function_count(&self) -> usize {
        self.functions.len()
    }

    #[must_use]
    pub fn base_artifact_id(&self) -> Option<Digest> {
        self.base.as_ref().map(|base| base.manifest.artifact_id)
    }

    /// Discard function payloads that already exist in the active artifact.
    ///
    /// Runtime callers retain that artifact independently and can supply it to
    /// [`compile_project_with_artifact`] on the next reload. Keeping only cache
    /// keys avoids a second resident copy of all code and source-map records.
    pub fn compact(&mut self) {
        for cached in self.functions.values_mut() {
            cached.function = None;
            cached.source_entries.clear();
            cached.native_imports.clear();
            cached.host_imports.clear();
        }
        if let Some(base) = &mut self.base {
            base.metadata = None;
        }
    }

    /// Return compact cache keys in the artifact's canonical function order.
    ///
    /// Project files already contain the complete artifact, so serializing the
    /// symbol-keyed map and its duplicate function keys only wastes space.
    ///
    /// # Errors
    ///
    /// Returns an error when the state is not the compact cache for the exact
    /// supplied artifact and current compiler ABI.
    pub fn compact_cache_keys(&self, artifact: &BytecodeArtifact) -> Result<Vec<Digest>, String> {
        self.validate_compact_cache_artifact(artifact)?;
        artifact
            .functions
            .iter()
            .map(|function| self.compact_cache_key(function))
            .collect()
    }

    /// Validate that this compact state belongs to the supplied artifact.
    ///
    /// This separates the constant-time artifact checks from per-function key lookup so
    /// single-threaded hosts can collect cache keys over multiple event-loop turns.
    ///
    /// # Errors
    ///
    /// Returns an error when the state was produced for another artifact or compiler ABI.
    pub fn validate_compact_cache_artifact(
        &self,
        artifact: &BytecodeArtifact,
    ) -> Result<(), String> {
        if self.compiler_abi != erabasic_bytecode::COMPILER_ABI_VERSION {
            return Err("incremental cache compiler ABI differs from the artifact".into());
        }
        if self.base_artifact_id() != Some(artifact.manifest.artifact_id) {
            return Err("incremental cache base differs from the artifact".into());
        }
        if self.functions.len() != artifact.functions.len() {
            return Err("incremental cache function count differs from the artifact".into());
        }
        Ok(())
    }

    /// Return one function's compact cache key after artifact-level validation.
    ///
    /// # Errors
    ///
    /// Returns an error when the compact state lacks the function or stores a mismatched key.
    pub fn compact_cache_key(&self, function: &BytecodeFunction) -> Result<Digest, String> {
        let cached = self
            .functions
            .get(&function.key)
            .ok_or_else(|| "incremental cache is missing an artifact function".to_owned())?;
        if cached.function_key != function.key {
            return Err("incremental cache function key is inconsistent".into());
        }
        Ok(cached.cache_key)
    }

    /// Rebuild a compact incremental cache from canonical artifact function keys.
    ///
    /// # Errors
    ///
    /// Returns an error when the cache-key count differs from the artifact's
    /// canonical function count.
    pub fn from_compact_cache_keys(
        artifact: &BytecodeArtifact,
        cache_keys: Vec<Digest>,
    ) -> Result<Self, String> {
        if cache_keys.len() != artifact.functions.len() {
            return Err("incremental cache key count differs from the artifact".into());
        }
        let functions = artifact
            .functions
            .iter()
            .zip(cache_keys)
            .map(|(function, cache_key)| {
                (
                    function.key,
                    CachedFunction {
                        cache_key,
                        function_key: function.key,
                        function: None,
                        source_entries: Vec::new(),
                        native_imports: Vec::new(),
                        host_imports: Vec::new(),
                    },
                )
            })
            .collect();
        Ok(Self {
            compiler_abi: erabasic_bytecode::COMPILER_ABI_VERSION,
            functions,
            base: Some(IncrementalBase::from_artifact(artifact, false)),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompileStats {
    pub total_functions: usize,
    pub compiled_functions: usize,
    pub reused_functions: usize,
    pub patch_functions: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompileReport {
    pub artifact: Option<BytecodeArtifact>,
    pub patch: Option<BytecodePatch>,
    pub incremental_state: IncrementalState,
    pub diagnostics: Vec<CompilerDiagnostic>,
    pub stats: CompileStats,
}

/// A compile report whose artifact retains validator provenance for in-process consumers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedCompileReport {
    pub artifact: Option<ValidatedArtifact>,
    pub patch: Option<BytecodePatch>,
    pub incremental_state: IncrementalState,
    pub diagnostics: Vec<CompilerDiagnostic>,
    pub stats: CompileStats,
}

/// An owned compile result and source identities needed to project pre-artifact diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedValidatedCompileReport {
    pub report: ValidatedCompileReport,
    /// Source identities in the same order as either the artifact source map or
    /// `diagnostic_sources`. This runtime-only side table is deliberately not
    /// part of the serialized artifact format.
    pub source_ids: Vec<SourceId>,
    pub diagnostic_sources: Vec<SourceRecord>,
}

impl From<ValidatedCompileReport> for CompileReport {
    fn from(report: ValidatedCompileReport) -> Self {
        Self {
            artifact: report.artifact.map(ValidatedArtifact::into_inner),
            patch: report.patch,
            incremental_state: report.incremental_state,
            diagnostics: report.diagnostics,
            stats: report.stats,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompileProgressStage {
    Compiling,
    Finalizing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompileProgress {
    pub stage: CompileProgressStage,
    pub completed: usize,
    pub total: usize,
}

#[cfg(not(target_arch = "wasm32"))]
pub trait CompileProgressCallback: Fn(CompileProgress) + Sync {}

#[cfg(not(target_arch = "wasm32"))]
impl<T> CompileProgressCallback for T where T: Fn(CompileProgress) + Sync {}

#[cfg(target_arch = "wasm32")]
pub trait CompileProgressCallback: Fn(CompileProgress) {}

#[cfg(target_arch = "wasm32")]
impl<T> CompileProgressCallback for T where T: Fn(CompileProgress) {}

#[cfg(test)]
mod tests {
    use super::{compile_progress_stride, should_report_progress};

    #[test]
    fn compile_progress_is_frequent_for_large_projects() {
        assert_eq!(compile_progress_stride(1), 1);
        assert_eq!(compile_progress_stride(100), 1);
        assert_eq!(compile_progress_stride(1_000), 10);
        assert_eq!(compile_progress_stride(58_349), 64);
    }

    #[test]
    fn compile_progress_fast_path_preserves_report_boundaries() {
        assert!(!should_report_progress(0, 0, 1_000, false));
        assert!(!should_report_progress(9, 0, 1_000, false));
        assert!(should_report_progress(10, 0, 1_000, false));
        assert!(!should_report_progress(10, 10, 1_000, true));
        assert!(should_report_progress(11, 10, 1_000, true));
        assert!(should_report_progress(1_000, 991, 1_000, false));
    }
}
