#[allow(clippy::wildcard_imports)]
use super::*;

impl Vm {
    pub(crate) fn apply_host_ready(
        &mut self,
        fiber: &mut Fiber,
        expected: Option<BytecodeType>,
        ready: HostReady,
    ) -> Result<(), VmError> {
        let result = match (expected, ready.value) {
            (None, None) => None,
            (Some(expected), Some(value)) if value.value_type() == expected => Some(value),
            (expected, value) => {
                return Err(VmError::InvalidArguments(format!(
                    "host result mismatch: expected {expected:?}, found {:?}",
                    value.as_ref().map(VmValue::value_type)
                )));
            }
        };
        if result.is_some() && fiber.frames.is_empty() {
            return Err(VmError::InvalidState("host fiber has no frame".into()));
        }

        let writes = ready
            .writes
            .into_iter()
            .map(|write| self.prepare_host_write(fiber, write))
            .collect::<Result<Vec<_>, _>>()?;
        for write in writes {
            self.write_place_internal(fiber, &write.target, write.value, true)
                .expect("the complete Host write batch was validated before mutation");
        }
        if let Some(value) = result {
            fiber
                .frames
                .last_mut()
                .expect("the Host result frame was validated before mutation")
                .stack
                .push(value);
        }
        Ok(())
    }

    fn prepare_host_write(&self, fiber: &Fiber, write: HostWrite) -> Result<HostWrite, VmError> {
        if write.target.backing.is_some() {
            return Err(VmError::InvalidState(
                "Host cannot inject an array backing identity".into(),
            ));
        }
        self.prepare_resolved_host_write(fiber, write)
    }

    /// Follow only VM-owned REF descriptors after the external Host boundary has
    /// rejected caller-supplied backing identities.
    fn prepare_resolved_host_write(
        &self,
        fiber: &Fiber,
        write: HostWrite,
    ) -> Result<HostWrite, VmError> {
        if write.target.fiber.is_some_and(|owner| owner != fiber.id) {
            return Err(VmError::InvalidState(
                "place belongs to another fiber".into(),
            ));
        }
        if write.target.backing.is_some() {
            let (_, cell) = self.checked_array_backing(fiber, &write.target)?;
            let mut staged = cell.clone();
            staged
                .write_execution(&write.target.indices, write.value.clone())
                .map_err(VmError::ScriptFailure)?;
            return Ok(write);
        }
        let definition = self.resolve_place_write(fiber, &write.target)?;
        if definition.storage == BytecodeStorage::FunctionLocal {
            let frame = find_frame(fiber, write.target.frame, definition.owner)?;
            let cell = frame
                .locals
                .get(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?;
            if let Some(VmValue::IntegerPlace(bound) | VmValue::StringPlace(bound)) = cell.first() {
                let mut target = (*bound).clone();
                target.indices.extend_from_slice(&write.target.indices);
                return self.prepare_resolved_host_write(
                    fiber,
                    HostWrite {
                        target,
                        value: write.value,
                    },
                );
            }
            let mut staged = cell.clone();
            staged
                .write_execution(&write.target.indices, write.value.clone())
                .map_err(VmError::ScriptFailure)?;
            let mut target = write.target;
            target.fiber = Some(fiber.id);
            target.frame = Some(frame.id);
            return Ok(HostWrite {
                target,
                value: write.value,
            });
        }

        let character = if definition.storage == BytecodeStorage::Character {
            write.target.character.map_or_else(
                || self.target_character_for_generation(definition.generation),
                |index| usize::try_from(index).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.validate_script_character(definition.storage, character)?;
        let cell = self
            .memory
            .cell(
                definition.generation,
                self.generations
                    .get(&definition.generation)
                    .and_then(|program| program.global(definition.key))
                    .ok_or_else(|| {
                        VmError::InvalidState("place definition is unavailable".into())
                    })?,
                character,
            )
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?;
        let mut staged = cell.clone();
        staged
            .write_execution(&write.target.indices, write.value.clone())
            .map_err(VmError::ScriptFailure)?;
        let mut target = write.target;
        target.fiber = Some(fiber.id);
        if definition.storage == BytecodeStorage::Character {
            target.character = Some(u64::try_from(character).unwrap_or(u64::MAX));
        }
        Ok(HostWrite {
            target,
            value: write.value,
        })
    }

    pub(crate) fn validate_script_character(
        &self,
        storage: BytecodeStorage,
        character: usize,
    ) -> Result<(), VmError> {
        if storage == BytecodeStorage::Character && character >= self.memory.characters.len() {
            return Err(VmError::ScriptFailure(crate::ExecutionFailure::script(
                crate::ScriptFaultKind::Bounds,
                crate::VmFaultCode::Bounds,
                format!(
                    "character index {character} is outside {} characters",
                    self.memory.characters.len()
                ),
            )));
        }
        Ok(())
    }
}
