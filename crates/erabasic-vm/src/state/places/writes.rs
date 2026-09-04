#[allow(clippy::wildcard_imports)]
use super::*;

impl Vm {
    pub(crate) fn fill_place_array_range(
        &mut self,
        fiber: &mut Fiber,
        place: &PlaceDescriptor,
        start: usize,
        end: usize,
        value: VmValue,
    ) -> Result<(), VmError> {
        if !place.indices.is_empty() {
            return Err(VmError::InvalidArguments(
                "array place must be unindexed".into(),
            ));
        }
        if place.backing.is_some() {
            let (lease, cell) = self.checked_array_backing(fiber, place)?;
            if start > end || end > cell.len() || value.value_type() != cell.value_type {
                return Err(VmError::InvalidArguments(
                    "array fill range or scalar type differs".into(),
                ));
            }
            let location = lease.location;
            self.invalidate_path_memo(fiber.id);
            return self
                .memory
                .array_cell_mut(fiber, location)?
                .fill_range(start, end, value)
                .map_err(VmError::InvalidArguments);
        }
        let definition = self.resolve_place_write(fiber, place)?;
        if !definition.mutable {
            return Err(VmError::InvalidState("place is immutable".into()));
        }
        if definition.storage == BytecodeStorage::FunctionLocal {
            let bound = find_frame(fiber, place.frame, definition.owner)?
                .locals
                .get(&definition.key)
                .and_then(VariableCell::first)
                .and_then(|value| match value {
                    VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => Some(*place),
                    VmValue::Integer(_) | VmValue::String(_) => None,
                });
            if let Some(bound) = bound {
                return self.fill_place_array_range(fiber, &bound, start, end, value);
            }
            return find_frame_mut(fiber, place.frame, definition.owner)?
                .locals
                .get_mut(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?
                .fill_range(start, end, value)
                .map_err(VmError::InvalidArguments);
        }
        let character = if definition.storage == BytecodeStorage::Character {
            place.character.map_or_else(
                || self.target_character_for_generation(definition.generation),
                |value| usize::try_from(value).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.validate_script_character(definition.storage, character)?;
        let implicit_target = definition.storage == BytecodeStorage::Character
            && character == self.target_character_for_generation(definition.generation);
        self.memory
            .cell_mut(
                definition.generation,
                definition.key,
                definition.storage,
                character,
            )
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?
            .fill_range(start, end, value.clone())
            .map_err(VmError::InvalidArguments)?;
        let global = self
            .generations
            .get(&definition.generation)
            .and_then(|program| program.global(definition.key))
            .expect("resolved place definition remains available");
        self.observe_path_memo_fill(
            fiber.id,
            definition.generation,
            global,
            character,
            implicit_target,
            start,
            end,
            &value,
        );
        Ok(())
    }

    pub(crate) fn write_place(
        &mut self,
        fiber: &mut Fiber,
        place: &PlaceDescriptor,
        value: VmValue,
    ) -> Result<(), VmError> {
        self.write_place_internal(fiber, place, value, false)
    }

    pub(crate) fn write_place_array(
        &mut self,
        fiber: &mut Fiber,
        place: &PlaceDescriptor,
        values: Vec<VmValue>,
    ) -> Result<(), VmError> {
        if !place.indices.is_empty() {
            return Err(VmError::InvalidArguments(
                "array place must be unindexed".into(),
            ));
        }
        if place.backing.is_some() {
            let (lease, cell) = self.checked_array_backing(fiber, place)?;
            if values.len() != cell.len()
                || values
                    .iter()
                    .any(|value| value.value_type() != cell.value_type)
            {
                return Err(VmError::InvalidArguments(
                    "array replacement length or scalar type differs".into(),
                ));
            }
            let location = lease.location;
            self.invalidate_path_memo(fiber.id);
            return replace_cell_values(self.memory.array_cell_mut(fiber, location)?, values);
        }
        let definition = self.resolve_place_write(fiber, place)?;
        if !definition.mutable {
            return Err(VmError::InvalidState("place is immutable".into()));
        }
        if definition.storage == BytecodeStorage::FunctionLocal {
            let bound = find_frame(fiber, place.frame, definition.owner)?
                .locals
                .get(&definition.key)
                .and_then(VariableCell::first)
                .and_then(|value| match value {
                    VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => Some(*place),
                    VmValue::Integer(_) | VmValue::String(_) => None,
                });
            if let Some(bound) = bound {
                return self.write_place_array(fiber, &bound, values);
            }
            let cell = find_frame_mut(fiber, place.frame, definition.owner)?
                .locals
                .get_mut(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?;
            return replace_cell_values(cell, values);
        }
        let character = if definition.storage == BytecodeStorage::Character {
            place.character.map_or_else(
                || self.target_character_for_generation(definition.generation),
                |value| usize::try_from(value).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.validate_script_character(definition.storage, character)?;
        let implicit_target = definition.storage == BytecodeStorage::Character
            && character == self.target_character_for_generation(definition.generation);
        let global = self
            .generations
            .get(&definition.generation)
            .and_then(|program| program.global(definition.key))
            .expect("resolved place definition remains available");
        self.observe_path_memo_replace(
            fiber.id,
            definition.generation,
            global,
            character,
            implicit_target,
            &values,
        );
        let cell = self
            .memory
            .cell_mut(
                definition.generation,
                definition.key,
                definition.storage,
                character,
            )
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?;
        replace_cell_values(cell, values)
    }

    pub(super) fn write_place_internal(
        &mut self,
        fiber: &mut Fiber,
        place: &PlaceDescriptor,
        value: VmValue,
        trusted_runtime: bool,
    ) -> Result<(), VmError> {
        if place.backing.is_some() {
            return self.write_array_backing(fiber, place, value);
        }
        if place.fiber.is_some_and(|owner| owner != fiber.id) {
            return Err(VmError::InvalidState(
                "place belongs to another fiber".into(),
            ));
        }
        let definition = self.resolve_place_write(fiber, place)?;
        if !definition.mutable && !trusted_runtime {
            return Err(VmError::InvalidState("place is immutable".into()));
        }
        if definition.storage == BytecodeStorage::FunctionLocal {
            let bound = find_frame(fiber, place.frame, definition.owner)?
                .locals
                .get(&definition.key)
                .and_then(VariableCell::first)
                .and_then(|value| match value {
                    VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => Some(*place),
                    VmValue::Integer(_) | VmValue::String(_) => None,
                });
            if let Some(mut target) = bound {
                target.indices.extend_from_slice(&place.indices);
                return self.write_place_internal(fiber, &target, value, trusted_runtime);
            }
            let frame = find_frame_mut(fiber, place.frame, definition.owner)?;
            return frame
                .locals
                .get_mut(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?
                .write_execution(&place.indices, value)
                .map_err(VmError::ScriptFailure);
        }
        let character = if definition.storage == BytecodeStorage::Character {
            place.character.map_or_else(
                || self.target_character_for_generation(definition.generation),
                |index| usize::try_from(index).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.validate_script_character(definition.storage, character)?;
        let implicit_target = definition.storage == BytecodeStorage::Character
            && character == self.target_character_for_generation(definition.generation);
        self.memory
            .cell_mut(
                definition.generation,
                definition.key,
                definition.storage,
                character,
            )
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?
            .write_execution(&place.indices, value.clone())
            .map_err(VmError::ScriptFailure)?;
        let global = self
            .generations
            .get(&definition.generation)
            .and_then(|program| program.global(definition.key))
            .expect("resolved place definition remains available");
        self.observe_path_memo_write(
            fiber.id,
            definition.generation,
            global,
            character,
            implicit_target,
            &place.indices,
            &value,
        );
        Ok(())
    }

    pub(crate) fn place_definition<'a>(
        &'a self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
    ) -> Result<(GenerationId, &'a erabasic_bytecode::BytecodeGlobal), VmError> {
        let generation = place
            .frame
            .and_then(|frame| {
                fiber
                    .frames
                    .iter()
                    .find(|candidate| candidate.id == frame)
                    .map(|frame| frame.generation)
            })
            .or_else(|| fiber.frames.last().map(|frame| frame.generation))
            .ok_or_else(|| VmError::InvalidState("place fiber has no frames".into()))?;
        let program = self
            .generations
            .get(&generation)
            .ok_or_else(|| VmError::InvalidState("place generation was reclaimed".into()))?;
        let definition = program
            .global(place.variable)
            .ok_or_else(|| VmError::InvalidState("place variable is not defined".into()))?;
        if place.backing.is_none() {
            self.require_bound_local_reference(fiber, generation, definition, place.frame)?;
        }
        Ok((generation, definition))
    }

    pub(super) fn resolve_place_write(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
    ) -> Result<ResolvedPlaceWrite, VmError> {
        let (generation, definition) = self.place_definition(fiber, place)?;
        Ok(ResolvedPlaceWrite {
            generation,
            key: definition.key,
            storage: definition.storage,
            mutable: definition.mutable,
            owner: definition.owner,
        })
    }
}
