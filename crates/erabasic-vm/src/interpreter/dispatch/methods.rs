#[allow(clippy::wildcard_imports)]
use super::super::*;
use crate::state::methods::{
    MethodBinding, PendingMethodCall, ResolvedMethod, resolve_method_call,
};
use erabasic_bytecode::{MethodArgumentSpec, MethodCallSpec};

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
            Opcode::ResolveMethod
                | Opcode::SelectMethodArgument
                | Opcode::CaptureMethodArgument
                | Opcode::InvokeMethod
        ) {
            return Ok(None);
        }
        self.invalidate_path_memo(fiber.id);
        let program = Arc::clone(
            self.generations
                .get(&position.generation)
                .ok_or_else(|| invalid("method caller generation is missing"))?,
        );
        let caller = fiber
            .frames
            .last()
            .filter(|frame| {
                frame.generation == position.generation && frame.function == position.function
            })
            .ok_or_else(|| invalid("method caller identity differs"))?;
        let owner = caller.id;
        if opcode == Opcode::ResolveMethod {
            self.resolve_expression_method(fiber, owner, position, &program)?;
        } else {
            let operands = decode_method_consumer(caller, position, opcode, &program)?;
            self.execute_method_consumer(fiber, owner, position, opcode, &program, &operands)?;
        }
        Ok(Some(StepOutcome::Continue))
    }

    fn resolve_expression_method(
        &self,
        fiber: &mut Fiber,
        owner: crate::FrameId,
        position: &InstructionPosition<'_>,
        program: &ProgramGeneration,
    ) -> Result<(), StepError> {
        let spec = MethodCallSpec::decode(position.encoded.payload).map_err(invalid)?;
        let VmValue::String(name) =
            pop(&mut fiber.frames.last_mut().expect("caller exists").stack)?
        else {
            return Err(StepError::new(
                VmFaultCode::TypeMismatch,
                "dynamic method name must be a string",
            ));
        };
        let method = resolve_method_call(
            program,
            position.generation,
            &name,
            &spec.arguments,
            Some(spec.result),
        )
        .map_err(map_vm_error)?;
        if let Some(method) = &method {
            self.validate_method_references(fiber, owner, method, &spec.arguments)
                .map_err(map_vm_error)?;
        }
        let frame = fiber.frames.last_mut().expect("caller exists");
        if let Some(method) = method {
            let target = program
                .function(method.function)
                .ok_or_else(|| invalid("resolved method disappeared"))?;
            frame.method_calls.push(PendingMethodCall {
                resolve: position.instruction,
                stack_index: frame.stack.len(),
                captured: 0,
                method,
            });
            frame.stack.push(VmValue::String(target.name.clone()));
        } else if spec.allow_missing {
            if usize::try_from(spec.missing_target)
                .ok()
                .is_none_or(|target| {
                    target
                        >= program
                            .function(position.function)
                            .expect("caller exists")
                            .code
                            .len()
                })
            {
                return Err(invalid("method missing branch leaves its function"));
            }
            frame.stack.push(VmValue::String(String::new()));
            frame.instruction = spec.missing_target as usize;
        } else {
            return Err(StepError::new(
                VmFaultCode::MissingSymbol,
                format!("dynamic method {name} is missing"),
            ));
        }
        Ok(())
    }

    fn execute_method_consumer(
        &mut self,
        fiber: &mut Fiber,
        owner: crate::FrameId,
        position: &InstructionPosition<'_>,
        opcode: Opcode,
        program: &ProgramGeneration,
        operands: &MethodConsumer,
    ) -> Result<(), StepError> {
        let spec = &operands.spec;
        let slot = operands.slot;
        let token_index = operands.token_index;
        let method = &operands.method;
        match opcode {
            Opcode::SelectMethodArgument => {
                if !matches!(spec.arguments[slot], MethodArgumentSpec::Variable(_)) {
                    return Err(invalid(
                        "only variable arguments have a reference selection",
                    ));
                }
                if matches!(method.bindings[slot], MethodBinding::ArrayReference) {
                    let target = read_u32(position.encoded.payload, 6)? as usize;
                    if target
                        >= program
                            .function(position.function)
                            .expect("caller exists")
                            .code
                            .len()
                    {
                        return Err(invalid("method reference branch leaves its function"));
                    }
                    fiber.frames.last_mut().expect("caller exists").instruction = target;
                }
            }
            Opcode::CaptureMethodArgument => {
                let reference = match position.encoded.payload[6] {
                    0 => false,
                    1 => true,
                    _ => return Err(invalid("invalid method reference flag")),
                };
                if reference != matches!(method.bindings[slot], MethodBinding::ArrayReference) {
                    return Err(invalid(
                        "method argument capture mode differs from its formal",
                    ));
                }
                let actual = pop(&mut fiber.frames.last_mut().expect("caller exists").stack)?;
                let captured = self
                    .capture_method_argument(fiber, owner, method, &spec.arguments, slot, actual)
                    .map_err(map_vm_error)?;
                let frame = fiber.frames.last_mut().expect("caller exists");
                frame.stack.push(captured);
                frame
                    .method_calls
                    .last_mut()
                    .expect("resolution checked")
                    .captured += 1;
            }
            Opcode::InvokeMethod => {
                let frame = fiber.frames.last_mut().expect("caller exists");
                let mut values = frame.stack.split_off(token_index + 1).into_iter();
                frame.stack.pop();
                frame.method_calls.pop();
                let captured = spec
                    .arguments
                    .iter()
                    .map(|argument| {
                        if matches!(argument, MethodArgumentSpec::Omitted) {
                            None
                        } else {
                            values.next()
                        }
                    })
                    .collect::<Vec<_>>();
                self.invoke_method(fiber, owner, method, &spec.arguments, &captured)
                    .map_err(map_vm_error)?;
            }
            _ => unreachable!("method opcode was filtered"),
        }
        Ok(())
    }
}

struct MethodConsumer {
    spec: MethodCallSpec,
    slot: usize,
    token_index: usize,
    method: ResolvedMethod,
}

fn decode_method_consumer(
    caller: &crate::state::Frame,
    position: &InstructionPosition<'_>,
    opcode: Opcode,
    program: &ProgramGeneration,
) -> Result<MethodConsumer, StepError> {
    let expected_len = match opcode {
        Opcode::SelectMethodArgument => 10,
        Opcode::CaptureMethodArgument => 7,
        _ => 4,
    };
    if position.encoded.payload.len() != expected_len {
        return Err(invalid("invalid method instruction operands"));
    }
    let resolve = read_u32(position.encoded.payload, 0)? as usize;
    let instruction = program
        .function(position.function)
        .and_then(|function| function.code.get(resolve))
        .filter(|instruction| {
            instruction.opcode == Opcode::ResolveMethod as u16 && resolve < position.instruction
        })
        .ok_or_else(|| invalid("method instruction does not identify its resolve origin"))?;
    let spec = MethodCallSpec::decode(&instruction.payload).map_err(invalid)?;
    let slot = if opcode == Opcode::InvokeMethod {
        spec.arguments.len()
    } else {
        usize::from(read_u16(position.encoded.payload, 4)?)
    };
    if slot > spec.arguments.len()
        || (opcode != Opcode::InvokeMethod && slot == spec.arguments.len())
    {
        return Err(invalid("method argument slot is out of bounds"));
    }
    let previous = spec.arguments[..slot]
        .iter()
        .filter(|argument| !matches!(argument, MethodArgumentSpec::Omitted))
        .count();
    let extra = usize::from(opcode == Opcode::CaptureMethodArgument);
    let stack = &caller.stack;
    let token_index = stack
        .len()
        .checked_sub(previous + extra + 1)
        .ok_or_else(|| invalid("method captures underflow their resolve token"))?;
    let Some(VmValue::String(name)) = stack.get(token_index) else {
        return Err(invalid("method resolve token is not a string"));
    };
    let method = resolve_method_call(
        program,
        position.generation,
        name,
        &spec.arguments,
        Some(spec.result),
    )
    .map_err(map_vm_error)?
    .ok_or_else(|| invalid("method token no longer resolves"))?;
    let pending = caller
        .method_calls
        .last()
        .ok_or_else(|| invalid("method token has no resolution identity"))?;
    if pending.resolve != resolve
        || pending.stack_index != token_index
        || pending.captured != previous
        || pending.method != method
    {
        return Err(invalid(
            "method token, generation, origin, or captured slot differs",
        ));
    }
    Ok(MethodConsumer {
        spec,
        slot,
        token_index,
        method,
    })
}
