#[allow(clippy::wildcard_imports)]
use super::super::*;
use crate::state::user_calls::{
    PendingUserCall, ResolvedUserCall, UserArgumentBinding, UserCallOrigin, resolve_user_call,
};
use erabasic_bytecode::{UserArgumentAdvance, UserArgumentSpec, UserCallSpec};

fn invalid(message: impl Into<String>) -> StepError {
    StepError::new(VmFaultCode::InvalidInstruction, message)
}

impl Vm {
    pub(in crate::interpreter) fn dispatch_methods(
        &mut self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        opcode: Opcode,
    ) -> Result<Option<StepOutcome>, StepError> {
        if !matches!(
            opcode,
            Opcode::ResolveUserCall
                | Opcode::SelectUserArgument
                | Opcode::CaptureUserArgument
                | Opcode::InvokeUserCall
                | Opcode::GuardUserArgument
                | Opcode::AdvanceUserArgument
                | Opcode::AbandonUserCall
        ) {
            return Ok(None);
        }
        let program = Arc::clone(
            self.generations
                .get(&position.generation)
                .ok_or_else(|| invalid("user-call generation is missing"))?,
        );
        let caller = fiber
            .frames
            .last()
            .filter(|frame| {
                frame.generation == position.generation && frame.function == position.function
            })
            .ok_or_else(|| invalid("user-call caller identity differs"))?;
        let owner = caller.id;
        if opcode == Opcode::ResolveUserCall {
            self.resolve_expression_user_call(fiber, position, &program)?;
        } else if opcode == Opcode::AbandonUserCall {
            abandon_user_call(fiber, position, &program)?;
        } else {
            let operands = decode_user_consumer(caller, position, opcode, &program)?;
            self.execute_user_consumer(fiber, owner, position, opcode, &program, &operands)?;
        }
        Ok(Some(StepOutcome::Continue))
    }

    fn resolve_expression_user_call(
        &mut self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        program: &ProgramGeneration,
    ) -> Result<(), StepError> {
        let spec = UserCallSpec::decode(position.encoded.payload).map_err(invalid)?;
        if spec.mode.expected_result().is_some() || spec.mode.unwinds_caller() {
            // GETMETH and deferred JUMP remain memo boundaries. Ordinary CALLFORM
            // and CALLFORMF keep their caller and can join its observed trace.
            self.invalidate_path_memo(fiber.id);
        }
        let VmValue::String(name) =
            pop(&mut fiber.frames.last_mut().expect("caller exists").stack)?
        else {
            return Err(invalid("user-call name is not a string"));
        };
        let call =
            resolve_user_call(program, position.generation, &name, &spec).map_err(map_vm_error)?;
        if let Some(call) = call {
            if !call.allows_path_memo_observation() {
                self.invalidate_path_memo(fiber.id);
            }
            self.queue_user_call_diagnostic(&call, spec.arguments.len());
            let target = program
                .function(call.function)
                .ok_or_else(|| invalid("resolved user target disappeared"))?;
            let frame = fiber.frames.last_mut().expect("caller exists");
            let slots = frame
                .operand_slots()
                .and_then(|slots| slots.checked_add(2))
                .and_then(|slots| slots.checked_add(call.bindings.len()));
            if slots.is_none_or(|slots| slots > self.config.maximum_operand_stack) {
                return Err(StepError::new(
                    VmFaultCode::ResourceLimit,
                    "maximum user-call state exceeded",
                ));
            }
            // No array cell is read here. Earlier value actuals may legitimately rebind
            // a later REF; its backing is selected only when that actual is captured.
            frame.user_calls.push(PendingUserCall {
                resolve: position.instruction,
                stack_index: frame.stack.len(),
                next_slot: 0,
                captured: Vec::new(),
                call,
            });
            frame.stack.push(VmValue::String(target.name.clone()));
        } else if spec.allow_missing {
            checked_user_target(program, position.function, spec.missing_target as usize)?;
            let frame = fiber.frames.last_mut().expect("caller exists");
            frame.stack.push(VmValue::String(String::new()));
            frame.instruction = spec.missing_target as usize;
        } else {
            return Err(StepError::script(
                crate::ScriptFaultKind::Resolve,
                VmFaultCode::MissingSymbol,
                format!("dynamic user target {name} is missing"),
            ));
        }
        Ok(())
    }

    fn execute_user_consumer(
        &mut self,
        fiber: &mut Fiber,
        owner: crate::FrameId,
        position: &InstructionPosition<'_>,
        opcode: Opcode,
        program: &ProgramGeneration,
        operands: &UserConsumer,
    ) -> Result<(), StepError> {
        let slot = operands.slot;
        let spec = &operands.spec;
        let call = &operands.call;
        let binding = call.bindings.get(slot);
        match opcode {
            Opcode::GuardUserArgument => {
                if matches!(spec.arguments[slot], UserArgumentSpec::Omitted) {
                    return Err(invalid("omitted user argument cannot be guarded"));
                }
                if binding.is_none() {
                    let target = read_u32(position.encoded.payload, 6)? as usize;
                    checked_user_target(program, position.function, target)?;
                    fiber.frames.last_mut().expect("caller exists").instruction = target;
                }
            }
            Opcode::SelectUserArgument => {
                if !matches!(spec.arguments[slot], UserArgumentSpec::Variable(_))
                    || binding.is_none()
                {
                    return Err(invalid("only retained variable arguments select REF"));
                }
                if matches!(binding, Some(UserArgumentBinding::ArrayReference)) {
                    let target = read_u32(position.encoded.payload, 6)? as usize;
                    checked_user_target(program, position.function, target)?;
                    fiber.frames.last_mut().expect("caller exists").instruction = target;
                }
            }
            Opcode::CaptureUserArgument => {
                let reference = match position.encoded.payload[6] {
                    0 => false,
                    1 => true,
                    _ => return Err(invalid("invalid user REF flag")),
                };
                if binding.is_none()
                    || matches!(binding, Some(UserArgumentBinding::Default(_)))
                    || reference != matches!(binding, Some(UserArgumentBinding::ArrayReference))
                {
                    return Err(invalid("user capture differs from the retained formal"));
                }
                let actual = pop(&mut fiber.frames.last_mut().expect("caller exists").stack)?;
                let value = self
                    .capture_user_argument(
                        fiber,
                        owner,
                        call,
                        &spec.arguments,
                        slot,
                        actual,
                        crate::state::array_leases::ArrayLeaseOrigin::UserBytecode {
                            resolve: read_u32(position.encoded.payload, 0)? as usize,
                            slot,
                        },
                    )
                    .map_err(map_vm_error)?;
                let pending = fiber
                    .frames
                    .last_mut()
                    .expect("caller exists")
                    .user_calls
                    .last_mut()
                    .expect("resolution checked");
                pending.captured.push(Some(value));
                pending.next_slot += 1;
            }
            Opcode::AdvanceUserArgument => {
                advance_user_actual(fiber, operands, position.encoded.payload)?;
            }
            Opcode::InvokeUserCall => {
                let frame = fiber.frames.last_mut().expect("caller exists");
                let pending = frame.user_calls.pop().expect("resolution checked");
                frame.stack.pop();
                self.invoke_user_call(
                    fiber,
                    owner,
                    call,
                    &spec.arguments,
                    &pending.captured,
                    UserCallOrigin::Bytecode {
                        resolve: pending.resolve,
                        invoke: position.instruction,
                    },
                )
                .map_err(map_vm_error)?;
            }
            _ => unreachable!("user-call opcode was filtered"),
        }
        Ok(())
    }
}

fn advance_user_actual(
    fiber: &mut Fiber,
    operands: &UserConsumer,
    payload: &[u8],
) -> Result<(), StepError> {
    let slot = operands.slot;
    let spec = &operands.spec;
    let binding = operands.call.bindings.get(slot);
    let reason = UserArgumentAdvance::decode(payload[6]).map_err(invalid)?;
    match reason {
        UserArgumentAdvance::Omitted => {
            if !matches!(spec.arguments[slot], UserArgumentSpec::Omitted)
                || binding
                    .is_some_and(|binding| !matches!(binding, UserArgumentBinding::Default(_)))
            {
                return Err(invalid("user omission does not match its formal"));
            }
        }
        UserArgumentAdvance::Discarded => {
            if matches!(spec.arguments[slot], UserArgumentSpec::Omitted) || binding.is_some() {
                return Err(invalid("retained user argument cannot be discarded"));
            }
        }
    }
    let pending = fiber
        .frames
        .last_mut()
        .expect("caller exists")
        .user_calls
        .last_mut()
        .expect("resolution checked");
    if binding.is_some() {
        pending.captured.push(None);
    }
    pending.next_slot += 1;
    Ok(())
}

struct UserConsumer {
    spec: UserCallSpec,
    slot: usize,
    call: ResolvedUserCall,
}

fn checked_user_target(
    program: &ProgramGeneration,
    function: SymbolKey,
    target: usize,
) -> Result<(), StepError> {
    if program
        .function(function)
        .is_none_or(|function| target >= function.code.len())
    {
        return Err(invalid("user-call branch leaves its function"));
    }
    Ok(())
}

fn user_origin(
    position: &InstructionPosition<'_>,
    program: &ProgramGeneration,
) -> Result<(usize, UserCallSpec), StepError> {
    let resolve = read_u32(position.encoded.payload, 0)? as usize;
    let instruction = program
        .function(position.function)
        .and_then(|function| function.code.get(resolve))
        .filter(|instruction| {
            instruction.opcode == Opcode::ResolveUserCall as u16 && resolve < position.instruction
        })
        .ok_or_else(|| invalid("user consumer has no earlier resolve origin"))?;
    Ok((
        resolve,
        UserCallSpec::decode(&instruction.payload).map_err(invalid)?,
    ))
}

fn decode_user_consumer(
    caller: &crate::state::Frame,
    position: &InstructionPosition<'_>,
    opcode: Opcode,
    program: &ProgramGeneration,
) -> Result<UserConsumer, StepError> {
    let expected_len = match opcode {
        Opcode::GuardUserArgument | Opcode::SelectUserArgument => 10,
        Opcode::CaptureUserArgument | Opcode::AdvanceUserArgument => 7,
        Opcode::InvokeUserCall => 4,
        _ => return Err(invalid("invalid user consumer opcode")),
    };
    if position.encoded.payload.len() != expected_len {
        return Err(invalid("invalid user consumer operands"));
    }
    let (resolve, spec) = user_origin(position, program)?;
    let slot = if opcode == Opcode::InvokeUserCall {
        spec.arguments.len()
    } else {
        usize::from(read_u16(position.encoded.payload, 4)?)
    };
    if slot > spec.arguments.len()
        || (opcode != Opcode::InvokeUserCall && slot == spec.arguments.len())
    {
        return Err(invalid("user argument slot is out of bounds"));
    }
    let pending = caller
        .user_calls
        .last()
        .ok_or_else(|| invalid("user consumer has no pending identity"))?;
    let extra = usize::from(opcode == Opcode::CaptureUserArgument);
    let token = caller
        .stack
        .len()
        .checked_sub(extra + 1)
        .ok_or_else(|| invalid("user consumer underflows its token"))?;
    let target = program
        .function(pending.call.function)
        .ok_or_else(|| invalid("user target disappeared"))?;
    if pending.resolve != resolve
        || pending.stack_index != token
        || pending.next_slot != slot
        || pending.call.generation != position.generation
        || pending.call.mode != spec.mode
        || pending.captured.len() != slot.min(pending.call.bindings.len())
        || caller.stack.get(token) != Some(&VmValue::String(target.name.clone()))
    {
        return Err(invalid(
            "user token, generation, origin or slot progress differs",
        ));
    }
    Ok(UserConsumer {
        spec,
        slot,
        call: pending.call.clone(),
    })
}

fn abandon_user_call(
    fiber: &mut Fiber,
    position: &InstructionPosition<'_>,
    program: &ProgramGeneration,
) -> Result<(), StepError> {
    if position.encoded.payload.len() != 4 {
        return Err(invalid("invalid abandon operands"));
    }
    let (resolve, spec) = user_origin(position, program)?;
    if !spec.allow_missing || spec.missing_target as usize != position.instruction {
        return Err(invalid("abandon is not the matching missing branch"));
    }
    let frame = fiber.frames.last_mut().expect("caller exists");
    if let Some(pending) = frame
        .user_calls
        .last()
        .filter(|pending| pending.resolve == resolve)
    {
        if pending.next_slot != 0 || frame.stack.len() != pending.stack_index + 1 {
            return Err(invalid("abandoned call has already consumed arguments"));
        }
        frame.user_calls.pop();
    }
    if pop(&mut frame.stack)? != VmValue::String(String::new()) {
        return Err(invalid("abandon did not consume a missing target token"));
    }
    Ok(())
}
