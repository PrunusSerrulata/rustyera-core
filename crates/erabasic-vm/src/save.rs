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
        let artifact = self.artifact();
        let variables = artifact
            .globals
            .iter()
            .filter(|definition| {
                definition.persistence != BytecodePersistence::None
                    && !matches!(
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
        let characters = self
            .memory
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
            .collect();
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
        overlay_shared(&artifact, &mut self.memory, &preserved, &mut report);
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
        let context = artifact.project_data.save_load_context();
        if !context
            .compatibility
            .accepts(state.unique_code, state.version)
        {
            return Err(VmError::Save(
                "save unique code or version is incompatible with this project".into(),
            ));
        }
        let mut memory = Memory::new_game(&artifact);
        let mut report = EraStateReport::default();
        overlay_shared(&artifact, &mut memory, &state.variables, &mut report);
        if context.clear_characters_before_overlay {
            memory.characters.clear();
        }
        for saved_character in &state.characters {
            memory.push_character(&artifact, None);
            let index = memory.characters.len() - 1;
            overlay_character(
                &artifact,
                &mut memory.characters[index],
                saved_character,
                &mut report,
            );
        }
        report.restored_characters = state.characters.len();
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
        values: cell.values.clone(),
    }
}

fn overlay_shared(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    memory: &mut Memory,
    saved: &BTreeMap<SymbolKey, EraVariableState>,
    report: &mut EraStateReport,
) {
    for (key, variable) in saved {
        let Some(definition) = artifact
            .globals
            .iter()
            .find(|definition| {
                definition.key == *key
                    && matches!(
                        definition.storage,
                        BytecodeStorage::Project | BytecodeStorage::FunctionStatic
                    )
                    && definition.persistence != BytecodePersistence::None
            })
            .or_else(|| {
                artifact.globals.iter().find(|definition| {
                    definition.name.eq_ignore_ascii_case(&variable.name)
                        && matches!(
                            definition.storage,
                            BytecodeStorage::Project | BytecodeStorage::FunctionStatic
                        )
                        && definition.persistence != BytecodePersistence::None
                })
            })
        else {
            report.skipped_variables += 1;
            continue;
        };
        let cell = match definition.storage {
            BytecodeStorage::FunctionStatic => memory.statics.get_mut(&definition.key),
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

fn overlay_character(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    character: &mut BTreeMap<SymbolKey, crate::VariableCell>,
    saved: &BTreeMap<SymbolKey, EraVariableState>,
    report: &mut EraStateReport,
) {
    for (key, variable) in saved {
        let Some(definition) = artifact
            .globals
            .iter()
            .find(|definition| {
                definition.key == *key
                    && definition.storage == BytecodeStorage::Character
                    && definition.persistence != BytecodePersistence::None
            })
            .or_else(|| {
                artifact.globals.iter().find(|definition| {
                    definition.name.eq_ignore_ascii_case(&variable.name)
                        && definition.storage == BytecodeStorage::Character
                        && definition.persistence != BytecodePersistence::None
                })
            })
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
