#[allow(clippy::wildcard_imports)]
use super::*;

pub(super) fn replace_cell_values(
    cell: &mut VariableCell,
    values: Vec<VmValue>,
) -> Result<(), VmError> {
    cell.replace_values(values)
        .map_err(VmError::InvalidArguments)
}

impl VmRuntimeStatePort for Vm {
    fn read_runtime_state(&self, reads: &[VmRuntimeRead]) -> Result<Vec<VmValue>, VmError> {
        reads
            .iter()
            .map(|read| self.read_variable(read.variable, &read.indices, read.character))
            .collect()
    }

    fn prepare_runtime_state(
        &self,
        transaction: VmRuntimeStateTransaction,
    ) -> Result<PreparedRuntimeState, VmError> {
        let artifact = self.artifact();
        let reset_execution = matches!(
            &transaction,
            VmRuntimeStateTransaction::ResetNewGame
                | VmRuntimeStateTransaction::RestoreOrdinary(_)
                | VmRuntimeStateTransaction::RestoreOrdinaryWithLastLoad { .. }
        );
        let mut memory = prepare_transaction_memory(artifact, &self.memory, &transaction)?;
        if let VmRuntimeStateTransaction::Mutate {
            writes,
            fills,
            clear_characters,
            add_characters_from_csv,
        } = transaction
        {
            if clear_characters {
                memory.characters.clear();
            }
            for csv_number in add_characters_from_csv {
                let template = artifact
                    .project_data
                    .static_data
                    .characters
                    .iter()
                    .find(|template| template.csv_no == csv_number)
                    .ok_or_else(|| {
                        VmError::InvalidArguments(format!(
                            "character CSV number {csv_number} does not exist"
                        ))
                    })?;
                memory.push_character(artifact, Some(template));
            }
            for fill in fills {
                let definition = find_global(artifact, fill.variable)?;
                if !definition.mutable || definition.storage == BytecodeStorage::FunctionLocal {
                    return Err(VmError::InvalidState(
                        "runtime state transaction cannot fill this variable".into(),
                    ));
                }
                if definition.value_type != fill.value.value_type() {
                    return Err(VmError::InvalidArguments(
                        "runtime fill value type differs from its variable".into(),
                    ));
                }
                let characters: Box<dyn Iterator<Item = usize>> =
                    if definition.storage == BytecodeStorage::Character && fill.all_characters {
                        Box::new(0..memory.characters.len())
                    } else {
                        Box::new(std::iter::once(
                            memory.target_character(artifact, self.current_generation),
                        ))
                    };
                for character in characters {
                    let cell = memory
                        .cell_mut(
                            self.current_generation,
                            definition.key,
                            definition.storage,
                            character,
                        )
                        .ok_or_else(|| {
                            VmError::InvalidState("variable storage is unavailable".into())
                        })?;
                    cell.fill(fill.value.clone())
                        .map_err(VmError::InvalidArguments)?;
                }
            }
            for write in writes {
                let definition = find_global(artifact, write.variable)?;
                if !definition.mutable || definition.storage == BytecodeStorage::FunctionLocal {
                    return Err(VmError::InvalidState(
                        "runtime state transaction cannot write this variable".into(),
                    ));
                }
                let character = write.character.map_or_else(
                    || memory.target_character(artifact, self.current_generation),
                    |value| usize::try_from(value).unwrap_or(usize::MAX),
                );
                memory
                    .cell_mut(
                        self.current_generation,
                        definition.key,
                        definition.storage,
                        character,
                    )
                    .ok_or_else(|| VmError::InvalidState("variable storage is unavailable".into()))?
                    .write(&write.indices, write.value)
                    .map_err(VmError::InvalidState)?;
            }
        }
        Ok(PreparedRuntimeState {
            generation: self.current_generation,
            memory,
            reset_execution,
            structured_state: None,
            base_column_stamp: None,
        })
    }

    fn commit_runtime_state(&mut self, prepared: PreparedRuntimeState) -> Result<(), VmError> {
        if prepared.generation != self.current_generation {
            return Err(VmError::InvalidState(
                "runtime state transaction belongs to a stale generation".into(),
            ));
        }
        self.memory = prepared.memory;
        if prepared.reset_execution {
            self.clear_execution();
        }
        Ok(())
    }
}

fn prepare_transaction_memory(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    current: &Memory,
    transaction: &VmRuntimeStateTransaction,
) -> Result<Memory, VmError> {
    Ok(match transaction {
        VmRuntimeStateTransaction::ResetNewGame => {
            crate::save::prepare_new_game_memory(artifact, current)
        }
        VmRuntimeStateTransaction::ResetGameData => {
            crate::save::prepare_reset_game_memory(artifact, current)
        }
        VmRuntimeStateTransaction::ResetGlobalData => {
            crate::save::prepare_reset_global_memory(artifact, current)
        }
        VmRuntimeStateTransaction::RestoreOrdinary(state) => {
            crate::save::prepare_era_memory(artifact, current, state)?.0
        }
        VmRuntimeStateTransaction::RestoreOrdinaryWithLastLoad { state, slot, text } => {
            let mut memory = crate::save::prepare_era_memory(artifact, current, state)?.0;
            memory.set_last_load(artifact, state.version, *slot, text);
            memory
        }
        VmRuntimeStateTransaction::OverlayGlobal(state) => {
            crate::save::prepare_global_memory(artifact, current, state)?.0
        }
        VmRuntimeStateTransaction::AppendCharacters(state) => {
            crate::save::prepare_appended_characters(artifact, current, state)?.0
        }
        VmRuntimeStateTransaction::SetLastLoad {
            version,
            slot,
            text,
        } => {
            let mut memory = current.clone();
            memory.set_last_load(artifact, *version, *slot, text);
            memory
        }
        VmRuntimeStateTransaction::Mutate { .. } => current.clone(),
    })
}
