#[allow(clippy::wildcard_imports)]
use super::*;

impl Vm {
    pub(crate) fn read_place(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
    ) -> Result<VmValue, VmError> {
        if place.backing.is_some() {
            return self
                .checked_array_backing(fiber, place)?
                .1
                .read_execution(&place.indices)
                .map_err(VmError::ScriptFailure);
        }
        let (generation, definition) = self.place_definition(fiber, place)?;
        self.read_place_resolved(fiber, place, generation, definition)
    }

    pub(crate) fn read_place_resolved(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
        generation: GenerationId,
        definition: &erabasic_bytecode::BytecodeGlobal,
    ) -> Result<VmValue, VmError> {
        if place.backing.is_some() {
            return self
                .checked_array_backing(fiber, place)?
                .1
                .read_execution(&place.indices)
                .map_err(VmError::ScriptFailure);
        }
        if definition.storage == BytecodeStorage::FunctionLocal {
            let frame = find_frame(fiber, place.frame, definition.owner)?;
            let cell = frame
                .locals
                .get(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?;
            if let Some(VmValue::IntegerPlace(bound) | VmValue::StringPlace(bound)) = cell.first() {
                let mut target = *bound;
                target.indices.extend_from_slice(&place.indices);
                return self.read_place(fiber, &target);
            }
            return cell
                .read_execution(&place.indices)
                .map_err(VmError::ScriptFailure);
        }
        let character = if definition.storage == BytecodeStorage::Character {
            place.character.map_or_else(
                || self.target_character_for_generation(generation),
                |value| usize::try_from(value).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        // Bytecode resolves omitted character selectors before reaching the storage layer. Treat
        // a selector equal to the current TARGET as target-dependent. This is conservative for an
        // explicit selector with the same value, but prevents an unsafe memo hit after TARGET
        // changes without altering captured-place semantics.
        self.validate_script_character(definition.storage, character)?;
        let implicit_target = definition.storage == BytecodeStorage::Character
            && character == self.target_character_for_generation(generation);
        let value = self
            .memory
            .cell(generation, definition, character)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?
            .read_execution(&place.indices)
            .map_err(VmError::ScriptFailure)?;
        self.observe_path_memo_read(
            fiber.id,
            generation,
            definition,
            character,
            implicit_target,
            &place.indices,
            &value,
        );
        Ok(value)
    }

    pub(crate) fn read_variable_resolved(
        &self,
        fiber: &Fiber,
        generation: GenerationId,
        definition: &erabasic_bytecode::BytecodeGlobal,
        indices: &[u64],
        character: Option<u64>,
        frame: Option<FrameId>,
    ) -> Result<VmValue, VmError> {
        self.require_bound_local_reference(fiber, generation, definition, frame)?;
        if definition.storage == BytecodeStorage::FunctionLocal {
            let frame = find_frame(fiber, frame, definition.owner)?;
            let cell = frame
                .locals
                .get(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?;
            if let Some(VmValue::IntegerPlace(bound) | VmValue::StringPlace(bound)) = cell.first() {
                let mut target = *bound;
                target.indices.extend_from_slice(indices);
                return self.read_place(fiber, &target);
            }
            return cell.read_execution(indices).map_err(VmError::ScriptFailure);
        }
        let character = if definition.storage == BytecodeStorage::Character {
            character.map_or_else(
                || self.target_character_for_generation(generation),
                |value| usize::try_from(value).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.validate_script_character(definition.storage, character)?;
        let implicit_target = definition.storage == BytecodeStorage::Character
            && character == self.target_character_for_generation(generation);
        let value = self
            .memory
            .cell(generation, definition, character)
            .ok_or_else(|| VmError::InvalidState("variable storage is unavailable".into()))?
            .read_execution(indices)
            .map_err(VmError::ScriptFailure)?;
        self.observe_path_memo_read(
            fiber.id,
            generation,
            definition,
            character,
            implicit_target,
            indices,
            &value,
        );
        Ok(value)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the hot bytecode path passes decoded variable metadata without allocating a place"
    )]
    pub(crate) fn write_variable_resolved(
        &mut self,
        fiber: &mut Fiber,
        generation: GenerationId,
        definition: &erabasic_bytecode::BytecodeGlobal,
        indices: &[u64],
        character: Option<u64>,
        frame: Option<FrameId>,
        value: VmValue,
    ) -> Result<(), VmError> {
        self.require_bound_local_reference(fiber, generation, definition, frame)?;
        if !definition.mutable {
            return Err(VmError::InvalidState("place is immutable".into()));
        }
        if definition.storage == BytecodeStorage::FunctionLocal {
            let bound = find_frame(fiber, frame, definition.owner)?
                .locals
                .get(&definition.key)
                .and_then(VariableCell::first)
                .and_then(|value| match value {
                    VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => Some(*place),
                    VmValue::Integer(_) | VmValue::String(_) => None,
                });
            if let Some(mut target) = bound {
                target.indices.extend_from_slice(indices);
                return self.write_place_internal(fiber, &target, value, false);
            }
            return find_frame_mut(fiber, frame, definition.owner)?
                .locals
                .get_mut(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?
                .write(indices, value)
                .map_err(VmError::InvalidState);
        }
        let character = if definition.storage == BytecodeStorage::Character {
            character.map_or_else(
                || self.target_character_for_generation(generation),
                |index| usize::try_from(index).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        let implicit_target = definition.storage == BytecodeStorage::Character
            && character == self.target_character_for_generation(generation);
        self.memory
            .cell_mut(generation, definition.key, definition.storage, character)
            .ok_or_else(|| VmError::InvalidState("variable storage is unavailable".into()))?
            .write(indices, value.clone())
            .map_err(VmError::InvalidState)?;
        self.observe_path_memo_write(
            fiber.id,
            generation,
            definition,
            character,
            implicit_target,
            indices,
            &value,
        );
        Ok(())
    }

    pub(crate) fn read_place_array(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
    ) -> Result<Vec<VmValue>, VmError> {
        if !place.indices.is_empty() {
            return Err(VmError::InvalidArguments(
                "array place must be unindexed".into(),
            ));
        }
        if place.backing.is_some() {
            return Ok(self.checked_array_backing(fiber, place)?.1.to_values());
        }
        let (generation, definition) = self.place_definition(fiber, place)?;
        if definition.storage == BytecodeStorage::FunctionLocal {
            let frame = find_frame(fiber, place.frame, definition.owner)?;
            let cell = frame
                .locals
                .get(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?;
            if let Some(VmValue::IntegerPlace(bound) | VmValue::StringPlace(bound)) = cell.first() {
                return self.read_place_array(fiber, &bound);
            }
            return Ok(cell.to_values());
        }
        let character = if definition.storage == BytecodeStorage::Character {
            place.character.map_or_else(
                || self.target_character_for_generation(generation),
                |value| usize::try_from(value).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.validate_script_character(definition.storage, character)?;
        self.memory
            .cell(generation, definition, character)
            .map(VariableCell::to_values)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))
    }

    pub(crate) fn place_array_len(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
    ) -> Result<usize, VmError> {
        if !place.indices.is_empty() {
            return Err(VmError::InvalidArguments(
                "array place must be unindexed".into(),
            ));
        }
        if place.backing.is_some() {
            return Ok(self.checked_array_backing(fiber, place)?.1.len());
        }
        let (generation, definition) = self.place_definition(fiber, place)?;
        if definition.storage == BytecodeStorage::FunctionLocal {
            let frame = find_frame(fiber, place.frame, definition.owner)?;
            let cell = frame
                .locals
                .get(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?;
            if let Some(VmValue::IntegerPlace(bound) | VmValue::StringPlace(bound)) = cell.first() {
                return self.place_array_len(fiber, &bound);
            }
            return Ok(cell.len());
        }
        let character = if definition.storage == BytecodeStorage::Character {
            place.character.map_or_else(
                || self.target_character_for_generation(generation),
                |value| usize::try_from(value).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.validate_script_character(definition.storage, character)?;
        self.memory
            .cell(generation, definition, character)
            .map(VariableCell::len)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))
    }

    pub(crate) fn place_array_revision(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
    ) -> Result<Option<(GenerationId, SymbolKey, u64)>, VmError> {
        if place.backing.is_some() {
            self.checked_array_backing(fiber, place)?;
            return Ok(None);
        }
        let (generation, definition) = self.place_definition(fiber, place)?;
        if matches!(
            definition.storage,
            BytecodeStorage::FunctionLocal | BytecodeStorage::Character
        ) {
            return Ok(None);
        }
        let revision = self
            .memory
            .cell(generation, definition, 0)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?
            .revision();
        Ok(Some((generation, definition.key, revision)))
    }

    pub(crate) fn read_place_array_range(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
        start: usize,
        end: usize,
    ) -> Result<Vec<VmValue>, VmError> {
        self.read_place_array_range_internal(fiber, place, start, end, true)
    }

    pub(crate) fn read_place_array_range_unobserved(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
        start: usize,
        end: usize,
    ) -> Result<Vec<VmValue>, VmError> {
        self.read_place_array_range_internal(fiber, place, start, end, false)
    }

    fn read_place_array_range_internal(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
        start: usize,
        end: usize,
        observe_path_memo: bool,
    ) -> Result<Vec<VmValue>, VmError> {
        if !place.indices.is_empty() {
            return Err(VmError::InvalidArguments(
                "array place must be unindexed".into(),
            ));
        }
        if place.backing.is_some() {
            return self
                .checked_array_backing(fiber, place)?
                .1
                .to_values_range(start, end)
                .ok_or_else(|| {
                    VmError::InvalidArguments("array range exceeds the variable".into())
                });
        }
        let (generation, definition) = self.place_definition(fiber, place)?;
        if definition.storage == BytecodeStorage::FunctionLocal {
            let frame = find_frame(fiber, place.frame, definition.owner)?;
            let cell = frame
                .locals
                .get(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?;
            if let Some(VmValue::IntegerPlace(bound) | VmValue::StringPlace(bound)) = cell.first() {
                return self.read_place_array_range_internal(
                    fiber,
                    &bound,
                    start,
                    end,
                    observe_path_memo,
                );
            }
            return cell.to_values_range(start, end).ok_or_else(|| {
                VmError::InvalidArguments("array range exceeds the variable".into())
            });
        }
        let character = if definition.storage == BytecodeStorage::Character {
            place.character.map_or_else(
                || self.target_character_for_generation(generation),
                |value| usize::try_from(value).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.validate_script_character(definition.storage, character)?;
        let implicit_target = definition.storage == BytecodeStorage::Character
            && character == self.target_character_for_generation(generation);
        let values = self
            .memory
            .cell(generation, definition, character)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?
            .to_values_range(start, end)
            .ok_or_else(|| VmError::InvalidArguments("array range exceeds the variable".into()))?;
        if observe_path_memo {
            self.observe_path_memo_range_read(
                fiber.id,
                generation,
                definition,
                character,
                implicit_target,
                start,
                &values,
            );
        }
        Ok(values)
    }
}
