use std::collections::BTreeMap;

use erabasic_bytecode::{BytecodePersistence, BytecodeStorage, BytecodeType, SymbolKey};
use serde::{Deserialize, Serialize};

use crate::{FiberState, Memory, Vm, VmError, VmValue};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EraVariableState {
    pub name: String,
    pub value_type: BytecodeType,
    pub dimensions: Vec<u64>,
    pub persistence: BytecodePersistence,
    pub storage: BytecodeStorage,
    pub values: Vec<VmValue>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EraState {
    pub unique_code: i64,
    pub version: i64,
    pub variables: BTreeMap<SymbolKey, EraVariableState>,
    pub characters: Vec<BTreeMap<SymbolKey, EraVariableState>>,
}

/// Selects the independent persistence domains used by current Emuera saves.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EraSaveScope {
    /// Ordinary slot data. Global variables are deliberately excluded.
    Ordinary,
    /// The process-wide `global.sav` variables only.
    Global,
    /// Character variables only, for character DAT files.
    Characters,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EraStateReport {
    pub restored_variables: usize,
    pub skipped_variables: usize,
    pub restored_characters: usize,
}

impl Vm {
    /// Export the state required by a traditional Era save. The runtime owns the
    /// actual on-disk format and can translate this deterministic structure to the
    /// reference-compatible binary or text representation.
    #[must_use]
    pub fn export_era_state(&self) -> EraState {
        self.export_era_state_for(EraSaveScope::Ordinary)
    }

    /// Export one persistence domain without leaking data from another domain.
    #[must_use]
    pub fn export_era_state_for(&self, scope: EraSaveScope) -> EraState {
        let artifact = self.artifact();
        let variables = artifact
            .globals
            .iter()
            .filter(|definition| {
                matches!(
                    (scope, definition.persistence),
                    (
                        EraSaveScope::Ordinary,
                        BytecodePersistence::GameSave | BytecodePersistence::ExtendedSave
                    ) | (EraSaveScope::Global, BytecodePersistence::GlobalSave)
                ) && !matches!(
                    definition.storage,
                    BytecodeStorage::FunctionLocal
                        | BytecodeStorage::FunctionPersistent
                        | BytecodeStorage::Character
                        | BytecodeStorage::Constant
                        | BytecodeStorage::Calculated
                )
            })
            .filter_map(|definition| {
                let cell = match definition.storage {
                    BytecodeStorage::FunctionStatic => self.memory.statics.get(&definition.key),
                    _ => self.memory.shared.get(&definition.key),
                };
                cell.map(|cell| (definition.key, saved_variable(definition, cell)))
            })
            .collect();
        let characters = if matches!(scope, EraSaveScope::Ordinary | EraSaveScope::Characters) {
            self.memory
                .characters
                .iter()
                .map(|character| {
                    artifact
                        .globals
                        .iter()
                        .filter(|definition| {
                            definition.storage == BytecodeStorage::Character
                                && definition.persistence != BytecodePersistence::None
                        })
                        .filter_map(|definition| {
                            character
                                .get(&definition.key)
                                .map(|cell| (definition.key, saved_variable(definition, cell)))
                        })
                        .collect()
                })
                .collect()
        } else {
            Vec::new()
        };
        EraState {
            unique_code: artifact.project_data.static_data.game_base.unique_code,
            version: artifact.project_data.static_data.game_base.version,
            variables,
            characters,
        }
    }

    /// Reset execution and initialize a new game while retaining `GlobalSave` data,
    /// matching the project-loading contract's `preserve_globals` rule.
    pub fn reset_new_game(&mut self) -> EraStateReport {
        let preserved: BTreeMap<_, _> = self
            .export_era_state()
            .variables
            .into_iter()
            .filter(|(_, variable)| variable.persistence == BytecodePersistence::GlobalSave)
            .collect();
        let artifact = self.artifact().clone();
        self.memory = Memory::new_game(&artifact);
        let mut report = EraStateReport::default();
        overlay_shared(
            &artifact,
            &mut self.memory,
            &preserved,
            EraSaveScope::Global,
            &mut report,
        );
        self.clear_execution();
        report
    }

    /// Apply a traditional save overlay after defaults. Call stacks and waits are
    /// intentionally discarded; the runtime chooses which entry point to start.
    ///
    /// # Errors
    ///
    /// Returns an error when the save's game code or version is incompatible.
    pub fn reset_with_era_state(&mut self, state: &EraState) -> Result<EraStateReport, VmError> {
        let artifact = self.artifact().clone();
        let (memory, report) = prepare_ordinary_memory(&artifact, &self.memory, state)?;
        self.memory = memory;
        self.clear_execution();
        Ok(report)
    }

    pub(crate) fn clear_execution(&mut self) {
        for fiber in self.fibers.values_mut() {
            fiber.state = FiberState::Cancelled;
        }
        self.fibers.clear();
        self.runnable.clear();
        self.primary_fiber = None;
        self.reclaim_generations();
    }
}

pub(crate) fn prepare_era_memory(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    current: &Memory,
    state: &EraState,
) -> Result<(Memory, EraStateReport), VmError> {
    prepare_ordinary_memory(artifact, current, state)
}

pub(crate) fn prepare_ordinary_memory(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    current: &Memory,
    state: &EraState,
) -> Result<(Memory, EraStateReport), VmError> {
    let context = artifact.project_data.save_load_context();
    if !context
        .compatibility
        .accepts(state.unique_code, state.version)
    {
        return Err(VmError::Save(
            "save unique code or version is incompatible with this project".into(),
        ));
    }
    let mut memory = Memory::new_game(artifact);
    let mut report = EraStateReport::default();
    let globals = exported_shared(artifact, current, EraSaveScope::Global);
    overlay_shared(
        artifact,
        &mut memory,
        &globals,
        EraSaveScope::Global,
        &mut report,
    );
    overlay_shared(
        artifact,
        &mut memory,
        &state.variables,
        EraSaveScope::Ordinary,
        &mut report,
    );
    if context.clear_characters_before_overlay {
        memory.characters.clear();
    }
    for saved_character in &state.characters {
        memory.push_character(artifact, None);
        let index = memory.characters.len() - 1;
        overlay_character(
            artifact,
            &mut memory.characters[index],
            saved_character,
            &mut report,
        );
    }
    memory.refresh_character_count(artifact);
    report.restored_characters = state.characters.len();
    Ok((memory, report))
}

pub(crate) fn prepare_global_memory(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    current: &Memory,
    state: &EraState,
) -> Result<(Memory, EraStateReport), VmError> {
    validate_compatibility(artifact, state)?;
    let mut memory = current.clone();
    let mut report = EraStateReport::default();
    overlay_shared(
        artifact,
        &mut memory,
        &state.variables,
        EraSaveScope::Global,
        &mut report,
    );
    Ok((memory, report))
}

pub(crate) fn prepare_appended_characters(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    current: &Memory,
    state: &EraState,
) -> Result<(Memory, EraStateReport), VmError> {
    validate_compatibility(artifact, state)?;
    let mut memory = current.clone();
    let mut report = EraStateReport::default();
    for saved_character in &state.characters {
        memory.push_character(artifact, None);
        let index = memory.characters.len() - 1;
        overlay_character(
            artifact,
            &mut memory.characters[index],
            saved_character,
            &mut report,
        );
    }
    memory.refresh_character_count(artifact);
    report.restored_characters = state.characters.len();
    Ok((memory, report))
}

pub(crate) fn prepare_reset_game_memory(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    current: &Memory,
) -> Memory {
    // RESETDATA clears every character. Initial CSV characters are inserted by
    // Emuera's surrounding title flow (or explicitly by the script), not by the
    // instruction itself.
    let mut memory = Memory::title(artifact);
    let globals = exported_shared(artifact, current, EraSaveScope::Global);
    overlay_shared(
        artifact,
        &mut memory,
        &globals,
        EraSaveScope::Global,
        &mut EraStateReport::default(),
    );
    // Emuera clears the backing arrays of local/static variables that have
    // already been created. It does not discard them while the function that
    // issued RESETDATA is still running.
    for definition in &artifact.globals {
        if matches!(
            definition.storage,
            BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent
        ) && current.statics.contains_key(&definition.key)
        {
            memory
                .statics
                .insert(definition.key, crate::VariableCell::new(definition));
        }
    }
    memory
}

pub(crate) fn prepare_new_game_memory(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    current: &Memory,
) -> Memory {
    let mut memory = Memory::new_game(artifact);
    let globals = exported_shared(artifact, current, EraSaveScope::Global);
    overlay_shared(
        artifact,
        &mut memory,
        &globals,
        EraSaveScope::Global,
        &mut EraStateReport::default(),
    );
    memory
}

pub(crate) fn prepare_reset_global_memory(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    current: &Memory,
) -> Memory {
    let defaults = Memory::new_game(artifact);
    let globals = exported_shared(artifact, &defaults, EraSaveScope::Global);
    let mut memory = current.clone();
    overlay_shared(
        artifact,
        &mut memory,
        &globals,
        EraSaveScope::Global,
        &mut EraStateReport::default(),
    );
    memory
}

fn validate_compatibility(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    state: &EraState,
) -> Result<(), VmError> {
    if artifact
        .project_data
        .save_load_context()
        .compatibility
        .accepts(state.unique_code, state.version)
    {
        Ok(())
    } else {
        Err(VmError::Save(
            "save unique code or version is incompatible with this project".into(),
        ))
    }
}

fn exported_shared(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    memory: &Memory,
    scope: EraSaveScope,
) -> BTreeMap<SymbolKey, EraVariableState> {
    artifact
        .globals
        .iter()
        .filter(|definition| {
            matches!(
                (scope, definition.persistence),
                (
                    EraSaveScope::Ordinary,
                    BytecodePersistence::GameSave | BytecodePersistence::ExtendedSave
                ) | (EraSaveScope::Global, BytecodePersistence::GlobalSave)
            )
        })
        .filter_map(|definition| {
            let cell = match definition.storage {
                BytecodeStorage::FunctionStatic => memory.statics.get(&definition.key),
                BytecodeStorage::Project => memory.shared.get(&definition.key),
                _ => None,
            }?;
            Some((definition.key, saved_variable(definition, cell)))
        })
        .collect()
}

fn saved_variable(
    definition: &erabasic_bytecode::BytecodeGlobal,
    cell: &crate::VariableCell,
) -> EraVariableState {
    EraVariableState {
        name: definition.name.clone(),
        value_type: cell.value_type,
        dimensions: cell.dimensions.clone(),
        persistence: definition.persistence,
        storage: definition.storage,
        values: cell.to_values(),
    }
}

fn overlay_shared(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    memory: &mut Memory,
    saved: &BTreeMap<SymbolKey, EraVariableState>,
    scope: EraSaveScope,
    report: &mut EraStateReport,
) {
    let eligible = artifact.globals.iter().filter(|definition| {
        matches!(
            definition.storage,
            BytecodeStorage::Project | BytecodeStorage::FunctionStatic
        ) && persistence_in_scope(definition.persistence, scope)
    });
    let mut by_key = BTreeMap::new();
    let mut by_name = BTreeMap::new();
    for definition in eligible {
        by_key.insert(definition.key, definition);
        by_name
            .entry(definition.name.to_ascii_uppercase())
            .or_insert(definition);
    }
    for (key, variable) in saved {
        let Some(definition) = by_key
            .get(key)
            .copied()
            .or_else(|| by_name.get(&variable.name.to_ascii_uppercase()).copied())
        else {
            report.skipped_variables += 1;
            continue;
        };
        let cell = match definition.storage {
            BytecodeStorage::FunctionStatic => Some(
                memory
                    .statics
                    .entry(definition.key)
                    .or_insert_with(|| crate::VariableCell::new(definition)),
            ),
            _ => memory.shared.get_mut(&definition.key),
        };
        let Some(cell) = cell else {
            report.skipped_variables += 1;
            continue;
        };
        if cell.value_type != variable.value_type
            || cell
                .overlay(&variable.dimensions, &variable.values)
                .is_err()
        {
            report.skipped_variables += 1;
        } else {
            report.restored_variables += 1;
        }
    }
}

fn persistence_in_scope(persistence: BytecodePersistence, scope: EraSaveScope) -> bool {
    matches!(
        (scope, persistence),
        (
            EraSaveScope::Ordinary,
            BytecodePersistence::GameSave | BytecodePersistence::ExtendedSave
        ) | (EraSaveScope::Global, BytecodePersistence::GlobalSave)
    )
}

fn overlay_character(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    character: &mut BTreeMap<SymbolKey, crate::VariableCell>,
    saved: &BTreeMap<SymbolKey, EraVariableState>,
    report: &mut EraStateReport,
) {
    let eligible = artifact.globals.iter().filter(|definition| {
        definition.storage == BytecodeStorage::Character
            && definition.persistence != BytecodePersistence::None
    });
    let mut by_key = BTreeMap::new();
    let mut by_name = BTreeMap::new();
    for definition in eligible {
        by_key.insert(definition.key, definition);
        by_name
            .entry(definition.name.to_ascii_uppercase())
            .or_insert(definition);
    }
    for (key, variable) in saved {
        let Some(definition) = by_key
            .get(key)
            .copied()
            .or_else(|| by_name.get(&variable.name.to_ascii_uppercase()).copied())
        else {
            report.skipped_variables += 1;
            continue;
        };
        let Some(cell) = character.get_mut(&definition.key) else {
            report.skipped_variables += 1;
            continue;
        };
        if cell.value_type != variable.value_type
            || cell
                .overlay(&variable.dimensions, &variable.values)
                .is_err()
        {
            report.skipped_variables += 1;
        } else {
            report.restored_variables += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use erabasic_bytecode::{
        ArtifactManifest, BytecodeArtifact, BytecodeGlobal, Digest, SourceMap,
    };
    use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};

    use super::*;
    use crate::GenerationId;

    #[test]
    fn reset_memories_only_recreate_function_state_that_was_reached() {
        let function = SymbolKey::derive("save.test.function", b"lazy");
        let definition = BytecodeGlobal {
            key: SymbolKey::derive("save.test.variable", b"lazy"),
            name: "LOCAL".into(),
            value_type: BytecodeType::Integer,
            dimensions: vec![4],
            mutable: true,
            storage: BytecodeStorage::FunctionPersistent,
            persistence: BytecodePersistence::None,
            initial_values: Vec::new(),
            owner: Some(function),
        };
        let artifact = BytecodeArtifact {
            manifest: ArtifactManifest::new(Digest::default()),
            call_compatibility: erabasic_bytecode::BytecodeCallCompatibility::default(),
            project_data: load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
                .data
                .unwrap(),
            globals: vec![definition.clone()],
            native_imports: Vec::new(),
            host_imports: Vec::new(),
            functions: Vec::new(),
            event_groups: Vec::new(),
            source_map: SourceMap::default(),
        };
        let mut current = Memory::title(&artifact);
        current.ensure_function_statics(GenerationId(1), function, [&definition]);
        assert_eq!(current.statics.len(), 1);
        current
            .statics
            .get_mut(&definition.key)
            .unwrap()
            .set(0, VmValue::Integer(42))
            .unwrap();

        let reset = prepare_reset_game_memory(&artifact, &current);
        assert_eq!(
            reset.statics[&definition.key].first(),
            Some(VmValue::Integer(0))
        );

        let mut new_game = prepare_new_game_memory(&artifact, &current);
        assert!(new_game.statics.is_empty());
        new_game.ensure_function_statics(GenerationId(1), function, [&definition]);
        assert_eq!(
            new_game.statics[&definition.key].first(),
            Some(VmValue::Integer(0))
        );
    }
}
