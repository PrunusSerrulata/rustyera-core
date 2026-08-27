use std::{collections::BTreeMap, fmt};

use erabasic_data::ProjectData;
use serde::{Deserialize, Serialize};

use crate::{
    ArtifactManifest, BytecodeArtifact, BytecodeCallCompatibility, BytecodeEventGroup,
    BytecodeFunction, BytecodeGlobal, Digest, HostImport, NativeImport, SourceMap, SymbolKey,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BytecodePatch {
    pub base_artifact_id: Digest,
    pub base_execution_id: Digest,
    pub target_manifest: ArtifactManifest,
    pub call_compatibility: Option<BytecodeCallCompatibility>,
    pub project_data: Option<ProjectData>,
    pub globals: Option<Vec<BytecodeGlobal>>,
    pub native_imports: Option<Vec<NativeImport>>,
    pub host_imports: Option<Vec<HostImport>>,
    pub changed_functions: Vec<BytecodeFunction>,
    pub removed_functions: Vec<SymbolKey>,
    pub event_groups: Option<Vec<BytecodeEventGroup>>,
    pub source_map: SourceMap,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatchError {
    BaseMismatch,
    InvalidTarget(String),
}

impl fmt::Display for PatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for PatchError {}

#[must_use]
pub fn create_patch(base: &BytecodeArtifact, target: &BytecodeArtifact) -> BytecodePatch {
    let base_functions: BTreeMap<_, _> = base
        .functions
        .iter()
        .map(|function| (function.key, function))
        .collect();
    let target_keys: BTreeMap<_, _> = target
        .functions
        .iter()
        .map(|function| (function.key, function))
        .collect();
    BytecodePatch {
        base_artifact_id: base.manifest.artifact_id,
        base_execution_id: base.manifest.program_version.execution_id,
        target_manifest: target.manifest.clone(),
        call_compatibility: (base.call_compatibility != target.call_compatibility)
            .then_some(target.call_compatibility),
        project_data: (base.project_data != target.project_data)
            .then(|| target.project_data.clone()),
        globals: (base.globals != target.globals).then(|| target.globals.clone()),
        native_imports: (base.native_imports != target.native_imports)
            .then(|| target.native_imports.clone()),
        host_imports: (base.host_imports != target.host_imports)
            .then(|| target.host_imports.clone()),
        changed_functions: target
            .functions
            .iter()
            .filter(|function| base_functions.get(&function.key) != Some(function))
            .cloned()
            .collect(),
        removed_functions: base
            .functions
            .iter()
            .filter(|function| !target_keys.contains_key(&function.key))
            .map(|function| function.key)
            .collect(),
        event_groups: (base.event_groups != target.event_groups)
            .then(|| target.event_groups.clone()),
        source_map: target.source_map.clone(),
    }
}

/// Apply a patch only to the exact base artifact and execution identity it names.
///
/// # Errors
///
/// Returns an error when the base identity or reconstructed target identity differs.
pub fn apply_patch(
    base: &BytecodeArtifact,
    patch: &BytecodePatch,
) -> Result<BytecodeArtifact, PatchError> {
    if base.manifest.artifact_id != patch.base_artifact_id
        || base.manifest.program_version.execution_id != patch.base_execution_id
    {
        return Err(PatchError::BaseMismatch);
    }
    if base.manifest.compatibility != patch.target_manifest.compatibility {
        return Err(PatchError::InvalidTarget(
            "compatibility change requires a cold load".into(),
        ));
    }
    let mut functions: BTreeMap<_, _> = base
        .functions
        .iter()
        .cloned()
        .map(|function| (function.key, function))
        .collect();
    for key in &patch.removed_functions {
        functions.remove(key);
    }
    for function in &patch.changed_functions {
        functions.insert(function.key, function.clone());
    }
    let mut target = BytecodeArtifact {
        manifest: patch.target_manifest.clone(),
        call_compatibility: patch.call_compatibility.unwrap_or(base.call_compatibility),
        project_data: patch
            .project_data
            .clone()
            .unwrap_or_else(|| base.project_data.clone()),
        globals: patch
            .globals
            .clone()
            .unwrap_or_else(|| base.globals.clone()),
        native_imports: patch
            .native_imports
            .clone()
            .unwrap_or_else(|| base.native_imports.clone()),
        host_imports: patch
            .host_imports
            .clone()
            .unwrap_or_else(|| base.host_imports.clone()),
        functions: functions.into_values().collect(),
        event_groups: patch
            .event_groups
            .clone()
            .unwrap_or_else(|| base.event_groups.clone()),
        source_map: patch.source_map.clone(),
    };
    target
        .refresh_ids()
        .map_err(|error| PatchError::InvalidTarget(error.to_string()))?;
    if target.manifest != patch.target_manifest {
        return Err(PatchError::InvalidTarget(
            "patched artifact identity does not match its target".into(),
        ));
    }
    Ok(target)
}
