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
            .cell_mut(generation, &definition, character)
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
                    .cell_mut(generation, definition, character)
                    .expect("the complete fill batch was validated before mutation")
                    .fill(plan.value.clone())
                    .expect("the complete fill batch type was validated before mutation");
            }
        }
        Ok(())
    }

    pub(crate) fn allocate_frame_id(&mut self) -> FrameId {
        let id = FrameId(self.next_frame);
        self.next_frame = self.next_frame.saturating_add(1);
        id
    }

    pub(crate) fn allocate_request_id(&mut self) -> HostRequestId {
        let id = HostRequestId(self.next_request);
        self.next_request = self.next_request.saturating_add(1);
        id
    }

    pub(super) fn next_available_fiber_id(&self) -> FiberId {
        let mut candidate = 1_u64;
        for id in self.fibers.keys() {
            if id.0 < candidate {
                continue;
            }
            if id.0 != candidate {
                break;
            }
            candidate = candidate
                .checked_add(1)
                .expect("the fiber map cannot contain every positive u64 id");
        }
        FiberId(candidate)
    }

    pub(crate) fn live_fiber_count(&self) -> usize {
        self.fibers
            .values()
            .filter(|fiber| {
                !matches!(
                    fiber.state,
                    FiberState::Completed(_) | FiberState::Cancelled | FiberState::Faulted(_)
                )
            })
            .count()
    }

    pub(crate) fn active_generations(&self) -> BTreeSet<GenerationId> {
        self.fibers
            .values()
            .flat_map(|fiber| fiber.frames.iter().map(|frame| frame.generation))
            .collect()
    }

    pub(crate) fn reclaim_generations(&mut self) {
        let active = self.active_generations();
        let obsolete: Vec<_> = self
            .generations
            .keys()
            .copied()
            .filter(|generation| {
                *generation != self.current_generation && !active.contains(generation)
            })
            .collect();
        for generation in obsolete {
            self.generations.remove(&generation);
            self.memory.reclaim_generation(generation);
        }
    }

    pub(crate) fn apply_host_ready(
        &mut self,
        fiber: &mut Fiber,
        expected: Option<BytecodeType>,
        ready: HostReady,
    ) -> Result<(), VmError> {
        match (expected, ready.value) {
            (None, None) => {}
            (Some(expected), Some(value)) if value.value_type() == expected => fiber
                .frames
                .last_mut()
                .ok_or_else(|| VmError::InvalidState("host fiber has no frame".into()))?
                .stack
                .push(value),
            (expected, value) => {
                return Err(VmError::InvalidArguments(format!(
                    "host result mismatch: expected {expected:?}, found {:?}",
                    value.as_ref().map(VmValue::value_type)
                )));
            }
        }
        for write in ready.writes {
            self.write_place_internal(fiber, &write.target, write.value, true)?;
        }
        Ok(())
    }

    pub(crate) fn read_place(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
    ) -> Result<VmValue, VmError> {
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
            return cell.read(&place.indices).map_err(VmError::InvalidState);
        }
        let character = if definition.storage == BytecodeStorage::Character {
            place.character.map_or_else(
                || self.target_character_for_generation(generation),
                |value| usize::try_from(value).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.memory
            .cell(generation, definition, character)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?
            .read(&place.indices)
            .map_err(VmError::InvalidState)
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
        self.memory
            .cell(generation, definition, character)
            .map(VariableCell::len)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))
    }

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
        let (generation, definition) = self.place_definition(fiber, place)?;
        let definition = definition.clone();
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
                || self.target_character_for_generation(generation),
                |value| usize::try_from(value).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.memory
            .cell_mut(generation, &definition, character)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?
            .fill_range(start, end, value)
            .map_err(VmError::InvalidArguments)
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
        let (generation, definition) = self.place_definition(fiber, place)?;
        let definition = definition.clone();
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
                || self.target_character_for_generation(generation),
                |value| usize::try_from(value).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        let cell = self
            .memory
            .cell_mut(generation, &definition, character)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?;
        replace_cell_values(cell, values)
    }

    fn write_place_internal(
        &mut self,
        fiber: &mut Fiber,
        place: &PlaceDescriptor,
        value: VmValue,
        trusted_runtime: bool,
    ) -> Result<(), VmError> {
        if place.fiber.is_some_and(|owner| owner != fiber.id) {
            return Err(VmError::InvalidState(
                "place belongs to another fiber".into(),
            ));
        }
        let (generation, definition) = self.place_definition(fiber, place)?;
        let definition = definition.clone();
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
                .write(&place.indices, value)
                .map_err(VmError::InvalidState);
        }
        let character = if definition.storage == BytecodeStorage::Character {
            place.character.map_or_else(
                || self.target_character_for_generation(generation),
                |index| usize::try_from(index).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.memory
            .cell_mut(generation, &definition, character)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?
            .write(&place.indices, value)
            .map_err(VmError::InvalidState)
    }

    pub(super) fn place_definition<'a>(
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
        Ok((
            generation,
            program.global(place.variable).ok_or_else(|| {
                VmError::InvalidState(format!("variable {:?} is not defined", place.variable))
            })?,
        ))
    }
}
