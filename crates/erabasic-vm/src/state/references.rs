//! Scoped whole-array references reuse `ArrayLeases`; no script index encodes an identity.
use super::array_leases::{
    ArrayLease, ArrayLeaseId, ArrayLeaseOrigin, ArrayLeaseOwner, ArrayLocation,
};
use super::{Fiber, Vm, find_frame};
use crate::{PlaceDescriptor, VariableCell, VmError, VmValue};
use erabasic_bytecode::{BytecodeStorage, BytecodeType};
use std::collections::BTreeSet;

fn invalid(message: &str) -> VmError {
    VmError::InvalidState(message.into())
}
pub(crate) fn unbound_reference() -> VmError {
    VmError::ScriptFailure(crate::ExecutionFailure::script(
        crate::ScriptFaultKind::Operation,
        crate::VmFaultCode::InvalidInstruction,
        "array REF is not bound",
    ))
}
fn argument(message: &str) -> VmError {
    VmError::ScriptFailure(crate::ExecutionFailure::script(
        crate::ScriptFaultKind::Argument,
        crate::VmFaultCode::TypeMismatch,
        message,
    ))
}

impl Vm {
    /// Capture occurs only after compiler/form validation has discarded ordinary indices and
    /// evaluated the character selector once. A nested REF follows its current binding now.
    #[allow(clippy::too_many_lines)] // Capture follows aliases until one backing identity is fixed.
    pub(crate) fn capture_array_reference(
        &mut self,
        fiber: &Fiber,
        token: &PlaceDescriptor,
        origin: ArrayLeaseOrigin,
    ) -> Result<PlaceDescriptor, VmError> {
        let frame = fiber
            .frames
            .last()
            .ok_or_else(|| invalid("array capture has no caller"))?;
        if !token.indices.is_empty() {
            return Err(invalid("whole-array capture received element indices"));
        }
        let owner = ArrayLeaseOwner {
            fiber: fiber.id,
            frame: frame.id,
            generation: frame.generation,
            function: frame.function,
            origin,
        };
        let input = token.variable;
        let mut current = token.clone();
        let mut seen = BTreeSet::new();
        let (location, value_type, dimensions, character_disposal) = loop {
            if current.fiber != Some(fiber.id) || !current.indices.is_empty() {
                return Err(invalid(
                    "array reference belongs to another fiber or an element",
                ));
            }
            if let Some(id) = current.backing {
                let (lease, cell) = self.checked_array_backing(fiber, &current)?;
                let _ = id;
                break (
                    lease.location,
                    cell.value_type,
                    cell.dimensions.clone(),
                    lease.character_disposal,
                );
            }
            let (generation, definition) = self.place_definition(fiber, &current)?;
            if !definition.mutable
                || !matches!(
                    definition.value_type,
                    BytecodeType::Integer | BytecodeType::String
                )
                || !(1..=3).contains(&definition.dimensions.len())
                || definition.storage == BytecodeStorage::Calculated
            {
                return Err(argument(
                    "REF requires a mutable scalar array of the declared rank",
                ));
            }
            let program = self
                .generations
                .get(&generation)
                .ok_or_else(|| invalid("array generation is missing"))?;
            let _metadata = program
                .runtime_variable(definition.key)
                .ok_or_else(|| invalid("array metadata is missing"))?;
            let location = if definition.storage == BytecodeStorage::FunctionLocal {
                let local = find_frame(fiber, current.frame, definition.owner)?;
                if !seen.insert((local.id, definition.key)) {
                    return Err(invalid("array reference binding is cyclic"));
                }
                let cell = local
                    .locals
                    .get(&definition.key)
                    .ok_or_else(|| invalid("local array storage is missing"))?;
                if program.is_reference_variable(definition.key) {
                    match cell.first() {
                        Some(VmValue::IntegerPlace(bound) | VmValue::StringPlace(bound))
                            if bound.backing.is_some() =>
                        {
                            current = *bound;
                            continue;
                        }
                        _ => return Err(unbound_reference()),
                    }
                }
                ArrayLocation::Local {
                    frame: local.id,
                    key: definition.key,
                }
            } else {
                if current
                    .frame
                    .is_some_and(|id| !fiber.frames.iter().any(|frame| frame.id == id))
                {
                    return Err(invalid("array token owner frame is not live"));
                }
                let character = if definition.storage == BytecodeStorage::Character {
                    let index = current
                        .character
                        .ok_or_else(|| invalid("character array selector was not captured"))?;
                    let index = usize::try_from(index)
                        .map_err(|_| argument("character selector is out of range"))?;
                    let rows = self
                        .memory
                        .legacy
                        .get(&generation)
                        .filter(|memory| {
                            memory
                                .characters
                                .iter()
                                .any(|row| row.contains_key(&definition.key))
                        })
                        .map_or(&self.memory.characters, |memory| &memory.characters);
                    if index >= rows.len() {
                        return Err(VmError::ScriptFailure(crate::ExecutionFailure::script(
                            crate::ScriptFaultKind::Bounds,
                            crate::VmFaultCode::Bounds,
                            "character selector is out of range",
                        )));
                    }
                    index
                } else {
                    if current.character.is_some() {
                        return Err(invalid("ordinary array has a character selector"));
                    }
                    0
                };
                self.memory.array_location(
                    generation,
                    definition.key,
                    definition.storage,
                    character,
                )?
            };
            let cell = self.memory.array_cell(fiber, location)?;
            if cell.value_type != definition.value_type
                || cell.dimensions.len() != definition.dimensions.len()
            {
                return Err(invalid("array storage differs from its declaration"));
            }
            break (
                location,
                cell.value_type,
                cell.dimensions.clone(),
                program
                    .effective_character_disposal(definition.key)
                    .ok_or_else(|| invalid("array disposal metadata is missing"))?,
            );
        };
        let length = self.memory.array_cell(fiber, location)?.len();
        let id = self.memory.array_leases.insert(ArrayLease {
            owner,
            input,
            location,
            length,
            value_type,
            dimensions,
            character_disposal,
        })?;
        Ok(PlaceDescriptor {
            variable: input,
            backing: Some(id),
            indices: Vec::new(),
            character: None,
            fiber: Some(fiber.id),
            frame: Some(owner.frame),
        })
    }

    /// Internal capture validation can be called while its pending record is moved out for
    /// invocation. It does not by itself grant Host/read/write access to an arbitrary ID.
    pub(crate) fn array_backing_record<'a>(
        &'a self,
        fiber: &'a Fiber,
        place: &PlaceDescriptor,
    ) -> Result<(&'a ArrayLease, &'a VariableCell), VmError> {
        let id = place
            .backing
            .ok_or_else(|| invalid("array backing identity is missing"))?;
        let lease = self
            .memory
            .array_leases
            .entries
            .get(&id)
            .ok_or_else(|| invalid("array backing identity is stale"))?;
        if lease.owner.fiber != fiber.id
            || place.fiber != Some(fiber.id)
            || place.frame != Some(lease.owner.frame)
            || place.variable != lease.input
            || place.character.is_some()
            || !fiber.frames.iter().any(|frame| {
                frame.id == lease.owner.frame
                    && frame.generation == lease.owner.generation
                    && frame.function == lease.owner.function
            })
        {
            return Err(invalid("array backing owner or source differs"));
        }
        let cell = self.memory.array_cell(fiber, lease.location)?;
        if cell.value_type != lease.value_type
            || cell.dimensions != lease.dimensions
            || cell.len() != lease.length
            || !cell.storage_is_valid()
        {
            return Err(invalid("array backing shape differs from capture"));
        }
        Ok((lease, cell))
    }

    pub(crate) fn checked_array_backing<'a>(
        &'a self,
        fiber: &'a Fiber,
        place: &PlaceDescriptor,
    ) -> Result<(&'a ArrayLease, &'a VariableCell), VmError> {
        let result = self.array_backing_record(fiber, place)?;
        self.invalidate_path_memo(fiber.id);
        let id = place.backing.expect("record requires identity");
        if !Self::reference_lease_is_reachable(fiber, id) {
            return Err(invalid(
                "array backing is not authorized by a live capture or REF alias",
            ));
        }
        Ok(result)
    }

    pub(crate) fn write_array_backing(
        &mut self,
        fiber: &mut Fiber,
        place: &PlaceDescriptor,
        value: VmValue,
    ) -> Result<(), VmError> {
        let (lease, cell) = self.checked_array_backing(fiber, place)?;
        let location = lease.location;
        // Preflight through the real scalar cell before any write/memo mutation.
        cell.read_execution(&place.indices)
            .map_err(VmError::ScriptFailure)?;
        if cell.value_type != value.value_type() {
            return Err(invalid("array backing write scalar type differs"));
        }
        self.invalidate_path_memo(fiber.id);
        self.memory
            .array_cell_mut(fiber, location)?
            .write_execution(&place.indices, value)
            .map_err(VmError::ScriptFailure)
    }
}

fn backing(value: &VmValue) -> Option<&PlaceDescriptor> {
    match value {
        VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => Some(place),
        _ => None,
    }
}

/// Actual roots only: scalar stacks cannot grant backing access. REF locals,
/// retained actual slots, and `RuntimeForm` call records are the sole user-call owners.
pub(crate) fn reference_leases(fiber: &Fiber) -> BTreeSet<ArrayLeaseId> {
    let mut roots = BTreeSet::new();
    for frame in &fiber.frames {
        for cell in frame.locals.values() {
            if let Some(id) = cell
                .first()
                .as_ref()
                .and_then(backing)
                .and_then(|place| place.backing)
            {
                roots.insert(id);
            }
        }
        for value in frame
            .user_calls
            .iter()
            .flat_map(|call| call.captured.iter().flatten())
        {
            if let Some(id) = backing(value).and_then(|place| place.backing) {
                roots.insert(id);
            }
        }
        if let Some(form) = &frame.runtime_form {
            roots.extend(form.reference_leases());
        }
    }
    roots
}

impl Vm {
    fn reference_lease_is_reachable(fiber: &Fiber, id: ArrayLeaseId) -> bool {
        reference_leases(fiber).contains(&id)
    }

    /// Build exact roots from validated call records. Duplicate aliases are legal
    /// only when they designate the very same owner/source capture.
    #[allow(clippy::too_many_lines)] // One pass proves every saved REF owner and call origin.
    pub(crate) fn collect_reference_owners(
        &self,
        expected: &mut std::collections::BTreeMap<ArrayLeaseId, ArrayLeaseOwner>,
    ) -> Result<(), VmError> {
        let mut add = |id, owner, input| -> Result<(), VmError> {
            let lease = self
                .memory
                .array_leases
                .entries
                .get(&id)
                .ok_or_else(|| invalid("REF backing is missing"))?;
            if lease.owner != owner || lease.input != input {
                return Err(invalid("REF backing capture origin differs"));
            }
            if expected
                .insert(id, owner)
                .is_some_and(|previous| previous != owner)
            {
                return Err(invalid("REF backing has incompatible owners"));
            }
            Ok(())
        };
        for fiber in self.fibers.values() {
            for (index, frame) in fiber.frames.iter().enumerate() {
                let program = self
                    .generations
                    .get(&frame.generation)
                    .ok_or_else(|| invalid("REF owner generation is missing"))?;
                let function = program
                    .function(frame.function)
                    .ok_or_else(|| invalid("REF owner function is missing"))?;
                for pending in &frame.user_calls {
                    let op = function
                        .code
                        .get(pending.resolve)
                        .filter(|op| op.opcode == erabasic_bytecode::Opcode::ResolveUserCall as u16)
                        .ok_or_else(|| invalid("REF pending resolve is missing"))?;
                    let spec = erabasic_bytecode::UserCallSpec::decode(&op.payload)
                        .map_err(|_| invalid("REF pending resolve is invalid"))?;
                    for (slot, value) in pending.captured.iter().enumerate() {
                        let Some(place) = value.as_ref().and_then(backing) else {
                            continue;
                        };
                        let Some(erabasic_bytecode::UserArgumentSpec::Variable(input)) =
                            spec.arguments.get(slot)
                        else {
                            return Err(invalid("REF pending source slot differs"));
                        };
                        self.array_backing_record(fiber, place)?;
                        add(
                            place
                                .backing
                                .ok_or_else(|| invalid("REF pending backing is missing"))?,
                            ArrayLeaseOwner {
                                fiber: fiber.id,
                                frame: frame.id,
                                generation: frame.generation,
                                function: frame.function,
                                origin: ArrayLeaseOrigin::UserBytecode {
                                    resolve: pending.resolve,
                                    slot,
                                },
                            },
                            *input,
                        )?;
                    }
                }
                if let Some(form) = &frame.runtime_form {
                    for (id, owner, input) in form.reference_captures() {
                        add(id, owner, input)?;
                    }
                }
                for (slot, formal) in function
                    .parameters
                    .iter()
                    .enumerate()
                    .filter(|(_, formal)| formal.by_reference)
                {
                    let place = frame
                        .locals
                        .get(&formal.key)
                        .and_then(VariableCell::first)
                        .and_then(|value| backing(&value).cloned())
                        .ok_or_else(|| invalid("REF formal alias is missing"))?;
                    self.array_backing_record(fiber, &place)?;
                    let call = frame
                        .user_call
                        .as_ref()
                        .ok_or_else(|| invalid("REF formal lacks a user-call origin"))?;
                    let caller = fiber
                        .frames
                        .get(
                            index
                                .checked_sub(1)
                                .ok_or_else(|| invalid("REF formal has no live caller"))?,
                        )
                        .filter(|caller| caller.id == call.caller)
                        .ok_or_else(|| invalid("REF formal caller is not its live ancestor"))?;
                    let id = place
                        .backing
                        .ok_or_else(|| invalid("REF formal backing is missing"))?;
                    match call.origin {
                        super::user_calls::UserCallOrigin::Bytecode { resolve, invoke } => {
                            let caller_program = self
                                .generations
                                .get(&caller.generation)
                                .ok_or_else(|| invalid("REF caller generation is missing"))?;
                            let caller_function = caller_program
                                .function(caller.function)
                                .ok_or_else(|| invalid("REF caller function is missing"))?;
                            let op = caller_function
                                .code
                                .get(resolve)
                                .filter(|op| {
                                    op.opcode == erabasic_bytecode::Opcode::ResolveUserCall as u16
                                })
                                .ok_or_else(|| invalid("REF caller resolve is missing"))?;
                            let resolve_payload = u32::try_from(resolve)
                                .map_err(|_| invalid("REF resolve exceeds bytecode format"))?;
                            if !caller_function.code.get(invoke).is_some_and(|op| {
                                op.opcode == erabasic_bytecode::Opcode::InvokeUserCall as u16
                                    && op.payload.as_slice() == resolve_payload.to_le_bytes()
                            }) {
                                return Err(invalid("REF caller invoke origin differs"));
                            }
                            let spec = erabasic_bytecode::UserCallSpec::decode(&op.payload)
                                .map_err(|_| invalid("REF caller spec is invalid"))?;
                            let Some(erabasic_bytecode::UserArgumentSpec::Variable(input)) =
                                spec.arguments.get(slot)
                            else {
                                return Err(invalid("REF caller source slot differs"));
                            };
                            add(
                                id,
                                ArrayLeaseOwner {
                                    fiber: fiber.id,
                                    frame: caller.id,
                                    generation: caller.generation,
                                    function: caller.function,
                                    origin: ArrayLeaseOrigin::UserBytecode { resolve, slot },
                                },
                                *input,
                            )?;
                        }
                        super::user_calls::UserCallOrigin::RuntimeForm => {
                            let form = caller
                                .runtime_form
                                .as_ref()
                                .filter(|form| form.valid_child_call(frame))
                                .ok_or_else(|| invalid("REF form caller wait differs"))?;
                            let (_, owner, input) = form.reference_captures().into_iter().find(|(capture, owner, _)| *capture == id
                                && matches!(owner.origin, ArrayLeaseOrigin::UserForm { slot: actual, .. } if actual == slot))
                                .ok_or_else(|| invalid("REF form formal differs from captured slot"))?;
                            add(id, owner, input)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn validate_reference_lease_sources(&self) -> Result<(), VmError> {
        for (id, lease) in &self.memory.array_leases.entries {
            if self.memory.array_leases.protected.contains(id) {
                continue;
            }
            let fiber = self
                .fibers
                .get(&lease.owner.fiber)
                .ok_or_else(|| invalid("array capture fiber is missing"))?;
            let position = fiber
                .frames
                .iter()
                .position(|frame| frame.id == lease.owner.frame)
                .ok_or_else(|| invalid("array capture owner is missing"))?;
            let frame = &fiber.frames[position];
            let program = self
                .generations
                .get(&frame.generation)
                .ok_or_else(|| invalid("array capture generation is missing"))?;
            let input = program
                .global(lease.input)
                .filter(|input| {
                    input.mutable
                        && input.value_type == lease.value_type
                        && input.dimensions.len() == lease.dimensions.len()
                        && input.owner.is_none_or(|owner| {
                            owner == frame.function
                                || input.storage == BytecodeStorage::FunctionPersistent
                        })
                })
                .ok_or_else(|| invalid("array capture source schema differs"))?;
            if program.is_reference_variable(input.key) {
                let bound = frame
                    .locals
                    .get(&input.key)
                    .and_then(VariableCell::first)
                    .and_then(|value| backing(&value).cloned())
                    .ok_or_else(|| invalid("captured REF source is unbound"))?;
                let (source, _) = self.array_backing_record(fiber, &bound)?;
                if !fiber.frames[..position]
                    .iter()
                    .any(|frame| frame.id == source.owner.frame)
                    || source.location != lease.location
                    || source.value_type != lease.value_type
                    || source.dimensions != lease.dimensions
                    || source.character_disposal != lease.character_disposal
                {
                    return Err(invalid(
                        "captured REF source does not match its live ancestor binding",
                    ));
                }
            } else {
                let _metadata = program
                    .runtime_variable(input.key)
                    .ok_or_else(|| invalid("array source metadata is missing"))?;
                if Some(lease.character_disposal) != program.effective_character_disposal(input.key)
                {
                    return Err(invalid("array disposal source differs"));
                }
                let matches = match lease.location {
                    ArrayLocation::Shared { key, .. } => {
                        key == input.key && matches!(input.storage, BytecodeStorage::Project)
                    }
                    ArrayLocation::Static { key, .. } => {
                        key == input.key
                            && matches!(
                                input.storage,
                                BytecodeStorage::FunctionStatic
                                    | BytecodeStorage::FunctionPersistent
                            )
                    }
                    ArrayLocation::Character { key, .. } => {
                        key == input.key && input.storage == BytecodeStorage::Character
                    }
                    ArrayLocation::Local { frame: owner, key } => {
                        key == input.key
                            && input.storage == BytecodeStorage::FunctionLocal
                            && owner == frame.id
                    }
                    // A direct character REF may outlive deletion; BIT direct character is
                    // independently rejected. Its retained scalar shape is checked by Memory.
                    ArrayLocation::Detached(_) => {
                        input.storage == BytecodeStorage::Character
                            && matches!(
                                lease.owner.origin,
                                ArrayLeaseOrigin::UserBytecode { .. }
                                    | ArrayLeaseOrigin::UserForm { .. }
                            )
                    }
                };
                if !matches {
                    return Err(invalid("array capture physical source differs"));
                }
            }
        }
        Ok(())
    }
}

impl Vm {
    pub(crate) fn require_bound_local_reference(
        &self,
        fiber: &Fiber,
        generation: crate::GenerationId,
        definition: &erabasic_bytecode::BytecodeGlobal,
        frame: Option<crate::FrameId>,
    ) -> Result<(), VmError> {
        let program = self
            .generations
            .get(&generation)
            .ok_or_else(|| invalid("REF metadata generation is missing"))?;
        if !program.is_reference_variable(definition.key) {
            return Ok(());
        }
        let owner = find_frame(fiber, frame, definition.owner)?;
        let cell = owner
            .locals
            .get(&definition.key)
            .ok_or_else(|| invalid("REF local storage is missing"))?;
        match cell.first() {
            Some(VmValue::IntegerPlace(bound) | VmValue::StringPlace(bound))
                if bound.backing.is_some() =>
            {
                self.array_backing_record(fiber, &bound)?;
                Ok(())
            }
            _ => Err(unbound_reference()),
        }
    }
}
