//! MATCH holds variable tokens, not snapshots or fixed REF backing objects.
use super::{ExecutionPolicy, InstructionPosition, StepError, StepOutcome, map_vm_error, read_u32};
use crate::{
    Fiber, FrameId, GenerationId, PlaceDescriptor, ScriptFaultKind, Vm, VmFaultCode, VmValue,
};
use erabasic_bytecode::{
    BytecodeStorage, BytecodeType, MatchCallSpec, MatchInput, Opcode, RuntimeExpressionShape,
    RuntimeStagedKind, SymbolKey,
};
use serde::{Deserialize, Serialize};

fn invalid(message: impl Into<String>) -> StepError {
    StepError::new(VmFaultCode::InvalidInstruction, message)
}
fn script(kind: ScriptFaultKind, message: impl Into<String>) -> StepError {
    StepError::script(kind, VmFaultCode::TypeMismatch, message)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct MatchToken {
    pub generation: GenerationId,
    pub owner: FrameId,
    pub variable: SymbolKey,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct MatchState {
    pub input: MatchToken,
    pub output: Option<MatchToken>,
    pub needle_type: BytecodeType,
    pub begin: Option<i64>,
    pub length: Option<i64>,
    pub end: Option<i64>,
    pub cursor: i64,
    pub count: i64,
    pub needle: Option<VmValue>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct PendingMatchCall {
    pub begin: usize,
    pub stack_index: usize,
    pub state: MatchState,
}

impl MatchState {
    #[allow(clippy::too_many_lines)] // Capture validates one staged token contract end to end.
    pub(crate) fn capture(
        vm: &Vm,
        fiber: &Fiber,
        generation: GenerationId,
        owner: FrameId,
        function: SymbolKey,
        spec: &MatchCallSpec,
    ) -> Result<Self, StepError> {
        let program = vm
            .generations
            .get(&generation)
            .ok_or_else(|| invalid("MATCH generation missing"))?;
        if !program
            .artifact
            .manifest
            .compatibility
            .supports_snake_data_apis()
        {
            return Err(invalid("MATCH unavailable in this identity"));
        }
        let (name, kind, input_shape) = match &spec.input {
            MatchInput::Variable(key) => {
                let definition = program
                    .global(*key)
                    .ok_or_else(|| invalid("MATCH input schema is missing"))?;
                (
                    "MATCHALL",
                    RuntimeStagedKind::MatchAll,
                    RuntimeExpressionShape {
                        value_type: definition.value_type,
                        variable: true,
                        mutable: definition.mutable,
                    },
                )
            }
            MatchInput::Name(_) => (
                "MATCHALLEX",
                RuntimeStagedKind::MatchAllEx,
                RuntimeExpressionShape {
                    value_type: BytecodeType::String,
                    variable: false,
                    mutable: false,
                },
            ),
        };
        let scalar = |value_type| {
            Some(RuntimeExpressionShape {
                value_type,
                variable: false,
                mutable: false,
            })
        };
        let output = match spec.output {
            Some(key) => {
                let definition = program
                    .global(key)
                    .ok_or_else(|| invalid("MATCH output schema is missing"))?;
                Some(RuntimeExpressionShape {
                    value_type: definition.value_type,
                    variable: true,
                    mutable: definition.mutable,
                })
            }
            None => None,
        };
        let shapes = [
            Some(input_shape),
            scalar(spec.needle),
            scalar(spec.begin_type),
            spec.end_type.and_then(scalar),
            output,
        ];
        if !program
            .artifact
            .runtime_staged_authorizations
            .iter()
            .any(|family| {
                family.name.eq_ignore_ascii_case(name)
                    && family.kind == kind
                    && family.accepts(&shapes)
            })
        {
            return Err(StepError::classified(
                crate::FaultCategory::Permission,
                VmFaultCode::InvalidInstruction,
                format!("MATCH operation {name} lacks matching staged authorization"),
            ));
        }
        if spec.input_restructured_to_scalar {
            return Err(script(
                ScriptFaultKind::Argument,
                "MATCHALL indexed CONST input is no longer a variable token after Restructure",
            ));
        }
        let frame = fiber
            .frames
            .iter()
            .find(|frame| {
                frame.id == owner && frame.generation == generation && frame.function == function
            })
            .ok_or_else(|| invalid("MATCH token owner missing"))?;
        let token = |key| -> Result<MatchToken, StepError> {
            let definition = program
                .global(key)
                .ok_or_else(|| invalid("MATCH variable metadata missing"))?;
            if !token_in_scope(program, frame.function, definition) {
                return Err(invalid("MATCH token belongs to another function"));
            }
            Ok(MatchToken {
                generation,
                owner,
                variable: key,
            })
        };
        let input = match &spec.input {
            MatchInput::Variable(key) => token(*key)?,
            MatchInput::Name(name) => {
                let ignore_case = program.artifact.call_compatibility.ignore_case;
                let definition = program
                    .function_locals(function)
                    .chain(
                        program
                            .function_statics(function)
                            .filter(|value| value.storage != BytecodeStorage::FunctionPersistent),
                    )
                    .chain(
                        program
                            .function_statics(function)
                            .filter(|value| value.storage == BytecodeStorage::FunctionPersistent),
                    )
                    .chain(
                        program
                            .artifact
                            .globals
                            .iter()
                            .filter(|value| value.owner.is_none()),
                    )
                    .find(|value| {
                        crate::compat_text::match_name_equals(&value.name, name, ignore_case)
                    })
                    .ok_or_else(|| {
                        script(
                            ScriptFaultKind::Resolve,
                            format!("MATCHALLEX variable {name:?} does not exist"),
                        )
                    })?;
                if let Some(rejection) = program
                    .runtime_variable(definition.key)
                    .and_then(|entry| entry.match_name_rejection)
                {
                    return Err(
                        if rejection == erabasic_bytecode::MatchNameRejectionKind::Script {
                            script(
                                ScriptFaultKind::Resolve,
                                format!("MATCHALLEX variable {name:?} is disabled"),
                            )
                        } else {
                            invalid(format!(
                                "MATCHALLEX non-forbiddable variable {name:?} is disabled"
                            ))
                        },
                    );
                }
                token(definition.key)?
            }
        };
        let output = spec.output.map(token).transpose()?;
        Ok(Self {
            input,
            output,
            needle_type: spec.needle,
            begin: None,
            length: None,
            end: None,
            cursor: 0,
            count: 0,
            needle: None,
        })
    }
    pub(crate) fn phase(&self) -> u8 {
        if self.begin.is_none() {
            0
        } else if self.end.is_none() {
            1
        } else {
            2
        }
    }
    pub(crate) fn set_begin(
        &mut self,
        vm: &Vm,
        fiber: &Fiber,
        value: &VmValue,
    ) -> Result<(), StepError> {
        if self.phase() != 0 {
            return Err(invalid("MATCH begin phase repeated"));
        }
        let VmValue::Integer(begin) = value else {
            return Err(script(
                ScriptFaultKind::Argument,
                "MATCH begin is not Integer",
            ));
        };
        // Negative begin is checked only after end was evaluated by the reference.
        let definition = token_definition(vm, fiber, &self.input)?;
        let length = if definition.storage == BytecodeStorage::Character {
            i64::try_from(vm.memory.characters.len())
                .map_err(|_| invalid("MATCH character count exceeds Integer"))?
        } else if definition.dimensions.len() == 1 {
            token_length(vm, fiber, &self.input)?
        } else {
            1
        };
        self.begin = Some(*begin);
        self.length = Some(length);
        Ok(())
    }
    pub(crate) fn set_end(&mut self, value: Option<&VmValue>) -> Result<(), StepError> {
        if self.phase() != 1 {
            return Err(invalid("MATCH end phase is out of order"));
        }
        let length = self
            .length
            .ok_or_else(|| invalid("MATCH captured length missing"))?;
        let end = match value {
            None => length,
            Some(VmValue::Integer(value)) => *value,
            Some(_) => {
                return Err(script(
                    ScriptFaultKind::Argument,
                    "MATCH end is not Integer",
                ));
            }
        };
        let begin = self.begin.ok_or_else(|| invalid("MATCH begin missing"))?;
        if begin < 0 || end < 0 {
            return Err(script(
                ScriptFaultKind::Bounds,
                "MATCH search range is negative",
            ));
        }
        if begin > end {
            return Err(script(
                ScriptFaultKind::Bounds,
                "MATCH search range is reversed",
            ));
        }
        self.end = Some(end.min(length));
        self.cursor = begin;
        Ok(())
    }
    pub(crate) fn set_needle(&mut self, value: VmValue) -> Result<(), StepError> {
        if self.phase() != 2 || self.needle.is_some() || value.value_type() != self.needle_type {
            return Err(invalid("MATCH needle phase/type differs"));
        }
        self.needle = Some(value);
        Ok(())
    }
    /// Each committed output is visible to the next read and is not rolled back on later script error.
    pub(crate) fn scan(
        &mut self,
        vm: &mut Vm,
        fiber: &mut Fiber,
        limit: usize,
    ) -> Result<(usize, bool), StepError> {
        let end = self
            .end
            .ok_or_else(|| invalid("MATCH scan range missing"))?;
        let needle = self
            .needle
            .as_ref()
            .ok_or_else(|| invalid("MATCH scan needle missing"))?;
        let mut work = 0;
        while self.cursor < end && work < limit {
            let value = token_read(vm, fiber, &self.input, self.cursor)?;
            if value.value_type() != self.needle_type {
                return Err(script(
                    ScriptFaultKind::Argument,
                    "MATCH input getter differs from needle type",
                ));
            }
            if value == *needle {
                if let Some(output) = &self.output {
                    // Capacity and REF binding are queried afresh for every match, never eagerly.
                    let capacity = token_length(vm, fiber, output)?;
                    if self.count < capacity {
                        token_write(vm, fiber, output, self.count, self.cursor)?;
                    }
                }
                self.count = self
                    .count
                    .checked_add(1)
                    .ok_or_else(|| invalid("MATCH count overflow"))?;
            }
            self.cursor += 1;
            work += 1;
        }
        Ok((work, self.cursor >= end))
    }
    pub(crate) fn valid(&self, vm: &Vm, fiber: &Fiber) -> bool {
        if token_definition(vm, fiber, &self.input).is_err()
            || self
                .output
                .as_ref()
                .is_some_and(|value| token_definition(vm, fiber, value).is_err())
        {
            return false;
        }
        if !matches!(
            self.needle_type,
            BytecodeType::Integer | BytecodeType::String
        ) {
            return false;
        }
        match (self.begin, self.length, self.end, self.needle.as_ref()) {
            (None, None, None, None) => self.cursor == 0 && self.count == 0,
            (Some(_), Some(length), None, None) => {
                length >= 0 && self.cursor == 0 && self.count == 0
            }
            (Some(begin), Some(length), Some(end), needle) => {
                begin >= 0
                    && length >= 0
                    && end >= 0
                    && end <= length
                    && self.cursor >= begin
                    && self.cursor <= begin.max(end)
                    && self.count >= 0
                    && self.count <= self.cursor - begin
                    && needle.is_none_or(|value| value.value_type() == self.needle_type)
                    && (needle.is_some() || (self.cursor == begin && self.count == 0))
            }
            _ => false,
        }
    }
}
fn token_in_scope(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    definition: &erabasic_bytecode::BytecodeGlobal,
) -> bool {
    definition.owner.is_none()
        || definition.owner == Some(function)
        || (definition.storage == BytecodeStorage::FunctionPersistent
            && program
                .function_statics(function)
                .any(|candidate| candidate.key == definition.key))
}
fn token_definition<'a>(
    vm: &'a Vm,
    fiber: &Fiber,
    token: &MatchToken,
) -> Result<&'a erabasic_bytecode::BytecodeGlobal, StepError> {
    let owner = fiber
        .frames
        .iter()
        .find(|frame| frame.id == token.owner && frame.generation == token.generation)
        .ok_or_else(|| invalid("MATCH variable token owner expired"))?;
    let definition = vm
        .generations
        .get(&token.generation)
        .and_then(|program| program.global(token.variable))
        .ok_or_else(|| invalid("MATCH variable token schema missing"))?;
    let program = vm
        .generations
        .get(&token.generation)
        .ok_or_else(|| invalid("MATCH token generation missing"))?;
    if !token_in_scope(program, owner.function, definition) {
        return Err(invalid("MATCH variable token scope differs"));
    }
    Ok(definition)
}
fn token_place(vm: &Vm, fiber: &Fiber, token: &MatchToken) -> Result<PlaceDescriptor, StepError> {
    token_definition(vm, fiber, token)?;
    Ok(PlaceDescriptor {
        variable: token.variable,
        fiber: Some(fiber.id),
        frame: Some(token.owner),
        ..PlaceDescriptor::default()
    })
}
fn token_length(vm: &Vm, fiber: &Fiber, token: &MatchToken) -> Result<i64, StepError> {
    let definition = token_definition(vm, fiber, token)?;
    if definition.dimensions.is_empty() {
        return Err(script(
            ScriptFaultKind::Argument,
            "MATCH output token has no array length",
        ));
    }
    let dimensions = vm
        .place_dimensions(fiber, &token_place(vm, fiber, token)?)
        .map_err(map_vm_error)?;
    let length = *dimensions
        .first()
        .ok_or_else(|| invalid("MATCH array dimensions missing"))?;
    i64::try_from(length)
        .map_err(|_| StepError::new(VmFaultCode::ResourceLimit, "MATCH length exceeds Integer"))
}
fn token_read(
    vm: &Vm,
    fiber: &Fiber,
    token: &MatchToken,
    index: i64,
) -> Result<VmValue, StepError> {
    let definition = token_definition(vm, fiber, token)?;
    let mut place = token_place(vm, fiber, token)?;
    let rank = definition.dimensions.len();
    if definition.storage == BytecodeStorage::Character {
        if rank > 1 {
            return Err(script(
                ScriptFaultKind::Bounds,
                "MATCH character getter lacks an index",
            ));
        }
        place.character = Some(index.cast_unsigned());
        if rank == 1 {
            place.indices.push(0);
        }
    } else {
        match rank {
            0 => {}
            1 => place.indices.push(index.cast_unsigned()),
            2 => place.indices.extend([index.cast_unsigned(), 0]),
            _ => {
                return Err(script(
                    ScriptFaultKind::Bounds,
                    "MATCH getter lacks an index",
                ));
            }
        }
    }
    vm.read_place(fiber, &place).map_err(map_vm_error)
}
fn token_write(
    vm: &mut Vm,
    fiber: &mut Fiber,
    token: &MatchToken,
    index: i64,
    value: i64,
) -> Result<(), StepError> {
    let definition = token_definition(vm, fiber, token)?;
    if definition.value_type != BytecodeType::Integer {
        return Err(script(
            ScriptFaultKind::Argument,
            "MATCH output does not accept Integer",
        ));
    }
    if !definition.mutable {
        return Err(script(
            ScriptFaultKind::Argument,
            "MATCH output is read-only",
        ));
    }
    let mut place = token_place(vm, fiber, token)?;
    if definition.storage == BytecodeStorage::Character || definition.dimensions.len() != 1 {
        return Err(script(
            ScriptFaultKind::Bounds,
            "MATCH output write lacks an index",
        ));
    }
    place.indices.push(index.cast_unsigned());
    vm.write_place(fiber, &place, VmValue::Integer(value))
        .map_err(map_vm_error)
}

impl Vm {
    #[allow(clippy::too_many_lines)] // Three resumable phases share one retained state transition.
    pub(in crate::interpreter) fn dispatch_match(
        &mut self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        opcode: Opcode,
        policy: ExecutionPolicy,
    ) -> Result<Option<StepOutcome>, StepError> {
        if !matches!(
            opcode,
            Opcode::BeginMatchCall | Opcode::MatchCallRange | Opcode::FinishMatchCall
        ) {
            return Ok(None);
        }
        self.invalidate_path_memo(fiber.id);
        if opcode == Opcode::BeginMatchCall {
            let spec = MatchCallSpec::decode(position.encoded.payload).map_err(invalid)?;
            let owner = fiber
                .frames
                .last()
                .ok_or_else(|| invalid("MATCH owner missing"))?;
            if owner
                .operand_slots()
                .and_then(|slots| slots.checked_add(12))
                .is_none_or(|slots| slots > self.config.maximum_operand_stack)
            {
                return Err(StepError::new(
                    VmFaultCode::ResourceLimit,
                    "MATCH pending state exceeds operand limit",
                ));
            }
            let state = MatchState::capture(
                self,
                fiber,
                position.generation,
                owner.id,
                position.function,
                &spec,
            )?;
            let owner = fiber.frames.last_mut().unwrap();
            let stack_index = owner.stack.len();
            let token = i64::try_from(position.instruction)
                .map_err(|_| invalid("MATCH capture offset exceeds Integer"))?;
            owner.stack.push(VmValue::Integer(token));
            owner.match_calls.push(PendingMatchCall {
                begin: position.instruction,
                stack_index,
                state,
            });
            return Ok(Some(StepOutcome::Continue));
        }
        let begin = read_u32(position.encoded.payload, 0)? as usize;
        let program = self
            .generations
            .get(&position.generation)
            .ok_or_else(|| invalid("MATCH generation missing"))?;
        let function = program
            .function(position.function)
            .ok_or_else(|| invalid("MATCH function missing"))?;
        let opening = function
            .code
            .get(begin)
            .ok_or_else(|| invalid("MATCH opener missing"))?;
        if Opcode::try_from(opening.opcode) != Ok(Opcode::BeginMatchCall) {
            return Err(invalid("MATCH opener opcode differs"));
        }
        let spec = MatchCallSpec::decode(&opening.payload).map_err(invalid)?;
        let owner = fiber
            .frames
            .last_mut()
            .ok_or_else(|| invalid("MATCH owner missing"))?;
        let mut pending = owner
            .match_calls
            .pop()
            .filter(|call| call.begin == begin)
            .ok_or_else(|| invalid("MATCH pending origin differs"))?;
        let begin_token = i64::from(
            u32::try_from(begin)
                .map_err(|_| invalid("MATCH capture offset exceeds the bytecode format"))?,
        );
        if owner.stack.get(pending.stack_index) != Some(&VmValue::Integer(begin_token)) {
            return Err(invalid("MATCH stack token differs"));
        }
        let phase = pending.state.phase();
        let mut failed_scan_work = 0;
        let result = if opcode == Opcode::MatchCallRange {
            let wanted = *position
                .encoded
                .payload
                .get(4)
                .ok_or_else(|| invalid("MATCH range phase missing"))?;
            if wanted != phase || wanted > 1 {
                return Err(invalid("MATCH range phase differs"));
            }
            let value = if wanted == 0 || spec.end_type.is_some() {
                Some(
                    owner
                        .stack
                        .pop()
                        .ok_or_else(|| invalid("MATCH range operand missing"))?,
                )
            } else {
                None
            };
            if owner.stack.len() != pending.stack_index + 1 {
                return Err(invalid("MATCH range stack watermark differs"));
            }
            let range_result = if wanted == 0 {
                pending.state.set_begin(
                    self,
                    fiber,
                    value.as_ref().expect("MATCH begin presence was checked"),
                )
            } else {
                pending.state.set_end(value.as_ref())
            };
            range_result.map(|()| (0, false))
        } else {
            if phase != 2 || owner.stack.len() != pending.stack_index + 2 {
                return Err(invalid("MATCH finish stack/phase differs"));
            }
            let needle = owner
                .stack
                .last()
                .cloned()
                .ok_or_else(|| invalid("MATCH needle missing"))?;
            if let Some(captured) = &pending.state.needle {
                if captured != &needle {
                    return Err(invalid(
                        "MATCH captured needle differs from retained operand",
                    ));
                }
            } else {
                pending.state.set_needle(needle)?;
            }
            let limit = policy
                .remaining_instructions
                .min(u64::from(policy.remaining_quantum))
                .clamp(1, 256) as usize;
            let before = pending.state.cursor;
            let result = pending.state.scan(self, fiber, limit);
            if result.is_err() {
                // Each successful row advances cursor once. The failing row is
                // the base instruction already charged by the scheduler.
                failed_scan_work = (pending.state.cursor - before).cast_unsigned();
            }
            result
        };
        // Restore state even on error; common recovery/fault cleanup releases only ephemeral state.
        let owner = fiber.frames.last_mut().unwrap();
        match result {
            Ok((work, true)) => {
                owner.stack.truncate(pending.stack_index);
                owner.stack.push(VmValue::Integer(pending.state.count));
                Ok(Some(StepOutcome::BulkProgress(
                    u64::try_from(work)
                        .map_err(|_| invalid("MATCH work exceeds scheduler counter"))?
                        .saturating_sub(1),
                )))
            }
            Ok((work, false)) => {
                if opcode == Opcode::FinishMatchCall {
                    owner.instruction = position.instruction;
                }
                owner.match_calls.push(pending);
                Ok(Some(if opcode == Opcode::FinishMatchCall {
                    StepOutcome::BulkProgress(
                        u64::try_from(work)
                            .map_err(|_| invalid("MATCH work exceeds scheduler counter"))?
                            .saturating_sub(1),
                    )
                } else {
                    StepOutcome::Continue
                }))
            }
            Err(error) => {
                owner.match_calls.push(pending);
                Ok(Some(StepOutcome::BulkFailure {
                    additional_instructions: failed_scan_work,
                    error,
                }))
            }
        }
    }
}

impl Vm {
    /// Structural/CFG proof runs before this. This second pass binds tokens and
    /// retained scalar state to the exact owner artifact, without reading cells.
    pub(crate) fn valid_frame_match_calls(
        &self,
        fiber: &Fiber,
        frame: &crate::state::Frame,
    ) -> bool {
        let Some(program) = self.generations.get(&frame.generation) else {
            return false;
        };
        let Some(function) = program.function(frame.function) else {
            return false;
        };
        frame.match_calls.iter().all(|pending| {
            let Some(opening) = function.code.get(pending.begin) else {
                return false;
            };
            if Opcode::try_from(opening.opcode) != Ok(Opcode::BeginMatchCall) {
                return false;
            }
            let Ok(spec) = MatchCallSpec::decode(&opening.payload) else {
                return false;
            };
            let Ok(initial) = MatchState::capture(
                self,
                fiber,
                frame.generation,
                frame.id,
                frame.function,
                &spec,
            ) else {
                return false;
            };
            if pending.state.input != initial.input
                || pending.state.output != initial.output
                || pending.state.needle_type != initial.needle_type
                || !pending.state.valid(self, fiber)
            {
                return false;
            }
            if let Some(needle) = &pending.state.needle {
                let Some(instruction) = function.code.get(frame.instruction) else {
                    return false;
                };
                // Only a partly executed Finish can retain a needle. Earlier
                // phases may suspend nested functions/Host calls but cannot scan.
                let Some(begin_payload) = u32::try_from(pending.begin).ok() else {
                    return false;
                };
                if Opcode::try_from(instruction.opcode) != Ok(Opcode::FinishMatchCall)
                    || instruction.payload.as_ref() != begin_payload.to_le_bytes()
                    || frame.stack.len() != pending.stack_index + 2
                    || frame.stack.last() != Some(needle)
                    || frame.runtime_form.is_some()
                {
                    return false;
                }
                let (Some(begin), Some(end)) = (pending.state.begin, pending.state.end) else {
                    return false;
                };
                let advanced = pending.state.cursor - begin;
                // A legitimate chunk may be shorter than256 when either the
                // slice budget or fiber quantum expires. Provenance and the
                // original range/needle checks above still bind this cursor.
                advanced > 0 && pending.state.cursor < end
            } else {
                true
            }
        })
    }
}
