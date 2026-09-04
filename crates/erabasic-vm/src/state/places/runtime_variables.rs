#[allow(clippy::wildcard_imports)]
use super::*;
impl Vm {
    /// Read non-frame storage in the current generation.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing variable, frame-local storage, or invalid index.
    pub fn read_variable(
        &self,
        variable: SymbolKey,
        indices: &[u64],
        character: Option<u64>,
    ) -> Result<VmValue, VmError> {
        let program = self
            .generations
            .get(&self.current_generation)
            .ok_or_else(|| VmError::InvalidState("current generation is missing".into()))?;
        let definition = program.global(variable).ok_or_else(|| {
            VmError::InvalidState(format!("variable {variable:?} is not defined"))
        })?;
        if definition.storage == BytecodeStorage::FunctionLocal {
            return Err(VmError::InvalidState(
                "frame-local variables require a place descriptor".into(),
            ));
        }
        let character = character.map_or_else(
            || self.target_character_for_generation(self.current_generation),
            |value| usize::try_from(value).unwrap_or(usize::MAX),
        );
        self.memory
            .cell(self.current_generation, definition, character)
            .ok_or_else(|| VmError::InvalidState("variable storage is unavailable".into()))?
            .read(indices)
            .map_err(VmError::InvalidState)
    }

    /// Write mutable, non-frame storage in the current generation.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing or immutable variable, unavailable storage,
    /// type mismatch, or invalid index.
    pub fn write_variable(
        &mut self,
        variable: SymbolKey,
        indices: &[u64],
        character: Option<u64>,
        value: VmValue,
    ) -> Result<(), VmError> {
        let generation = self.current_generation;
        let definition = self
            .generations
            .get(&generation)
            .and_then(|program| program.global(variable))
            .cloned()
            .ok_or_else(|| {
                VmError::InvalidState(format!("variable {variable:?} is not defined"))
            })?;
        if !definition.mutable {
            return Err(VmError::InvalidState("variable is immutable".into()));
        }
        let character = character.map_or_else(
            || self.target_character_for_generation(generation),
            |value| usize::try_from(value).unwrap_or(usize::MAX),
        );
        self.memory
            .cell_mut(generation, definition.key, definition.storage, character)
            .ok_or_else(|| VmError::InvalidState("variable storage is unavailable".into()))?
            .write(indices, value)
            .map_err(VmError::InvalidState)
    }

    /// Validate and fill a batch of runtime-owned variables without cloning VM memory.
    ///
    /// # Errors
    ///
    /// Returns before the first mutation if any variable, character destination, or
    /// value type is invalid.
    ///
    /// # Panics
    ///
    /// Panics only if validated variable storage changes during the commit phase. The
    /// exclusive VM borrow prevents such a change through the public API.
    pub fn fill_runtime_variables(&mut self, fills: &[VmRuntimeFill]) -> Result<(), VmError> {
        struct FillPlan {
            global_index: usize,
            characters: Vec<usize>,
            value: VmValue,
        }

        let generation = self.current_generation;
        let program = self
            .generations
            .get(&generation)
            .ok_or_else(|| VmError::InvalidState("current generation is missing".into()))?;
        let target_key = program.target_global().map(|definition| definition.key);
        let mut target_character = self.target_character_for_generation(generation);
        let mut plans = Vec::with_capacity(fills.len());
        for fill in fills {
            let global_index = *program.global_index(fill.variable).ok_or_else(|| {
                VmError::InvalidState(format!("variable {:?} is not defined", fill.variable))
            })?;
            let definition = &program.artifact.globals[global_index];
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
            let characters = if definition.storage == BytecodeStorage::Character {
                if fill.all_characters {
                    (0..self.memory.characters.len()).collect()
                } else {
                    vec![target_character]
                }
            } else {
                vec![0]
            };
            for character in &characters {
                let cell = self
                    .memory
                    .cell(generation, definition, *character)
                    .ok_or_else(|| {
                        VmError::InvalidState("variable storage is unavailable".into())
                    })?;
                if cell.value_type != fill.value.value_type() {
                    return Err(VmError::InvalidArguments(
                        "runtime fill value type differs from its variable".into(),
                    ));
                }
            }
            if Some(fill.variable) == target_key
                && let VmValue::Integer(value) = &fill.value
            {
                target_character = usize::try_from(*value).unwrap_or(0);
            }
            plans.push(FillPlan {
                global_index,
                characters,
                value: fill.value.clone(),
            });
        }

        let program = self
            .generations
            .get(&generation)
            .ok_or_else(|| VmError::InvalidState("current generation is missing".into()))?;
        for plan in plans {
            let definition = &program.artifact.globals[plan.global_index];
            for character in plan.characters {
                self.memory
                    .cell_mut(generation, definition.key, definition.storage, character)
                    .expect("the complete fill batch was validated before mutation")
                    .fill(plan.value.clone())
                    .expect("the complete fill batch type was validated before mutation");
            }
        }
        Ok(())
    }
}
