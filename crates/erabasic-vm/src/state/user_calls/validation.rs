use super::{
    Fiber, UserArgumentBinding, UserCallOrigin, UserCallSpec, Vm, VmValue, resolve_user_call,
    validate_user_call_target_kind,
};

impl Vm {
    pub(crate) fn valid_frame_user_calls(
        &self,
        fiber: &Fiber,
        frame: &super::super::Frame,
    ) -> bool {
        let Some(program) = self.generations.get(&frame.generation) else {
            return false;
        };
        let Some(function) = program.function(frame.function) else {
            return false;
        };
        let mut previous_token = None;
        for pending in &frame.user_calls {
            let Some(instruction) = function.code.get(pending.resolve) else {
                return false;
            };
            if instruction.opcode != erabasic_bytecode::Opcode::ResolveUserCall as u16
                || pending.resolve >= frame.instruction
                || pending.call.generation != frame.generation
                || previous_token.is_some_and(|index| pending.stack_index <= index)
            {
                return false;
            }
            let Ok(spec) = UserCallSpec::decode(&instruction.payload) else {
                return false;
            };
            let Some(target) = program.function(pending.call.function) else {
                return false;
            };
            if frame.stack.get(pending.stack_index) != Some(&VmValue::String(target.name.clone()))
                || resolve_user_call(program, frame.generation, &target.name, &spec)
                    .ok()
                    .flatten()
                    .as_ref()
                    != Some(&pending.call)
                || pending.next_slot > spec.arguments.len()
                || pending.captured.len() != pending.next_slot.min(pending.call.bindings.len())
            {
                return false;
            }
            for (slot, value) in pending.captured.iter().enumerate() {
                match (&pending.call.bindings[slot], value) {
                    (UserArgumentBinding::Default(_), None) => {}
                    (UserArgumentBinding::Value { .. }, Some(value))
                        if value.value_type() == target.parameters[slot].value_type => {}
                    (UserArgumentBinding::ArrayReference, Some(value)) => {
                        if self
                            .validate_captured_user_reference(fiber, &pending.call, slot, value)
                            .is_err()
                        {
                            return false;
                        }
                    }
                    _ => return false,
                }
            }
            previous_token = Some(pending.stack_index);
        }
        true
    }
}

impl Vm {
    /// The return mode is control flow, not an unchecked snapshot boolean. Bind it
    /// to the actual suspended instruction or the owner's validated form work.
    pub(crate) fn valid_frame_user_call_origin(
        &self,
        fiber: &Fiber,
        frame: &super::super::Frame,
    ) -> bool {
        let Some(index) = fiber
            .frames
            .iter()
            .position(|candidate| candidate.id == frame.id)
        else {
            return false;
        };
        let Some(caller) = index
            .checked_sub(1)
            .and_then(|index| fiber.frames.get(index))
        else {
            return frame.user_call.is_none();
        };
        let Some(program) = self.generations.get(&caller.generation) else {
            return false;
        };
        let Some(caller_function) = program.function(caller.function) else {
            return false;
        };
        let previous = caller
            .instruction
            .checked_sub(1)
            .and_then(|index| caller_function.code.get(index));
        let Some(call) = &frame.user_call else {
            return previous.is_none_or(|instruction| {
                instruction.opcode != erabasic_bytecode::Opcode::InvokeUserCall as u16
            }) && caller
                .runtime_form
                .as_ref()
                .is_none_or(|form| form.expected_child() != Some(frame.id));
        };
        if call.caller != caller.id
            || frame.generation != caller.generation
            || frame.return_value_to_caller != call.mode.expected_result().is_some()
        {
            return false;
        }
        let Some(target) = program.function(frame.function) else {
            return false;
        };
        if validate_user_call_target_kind(program, target, call.mode).is_err()
            || call
                .mode
                .expected_result()
                .is_some_and(|expected| target.result != Some(expected))
        {
            return false;
        }
        match call.origin {
            UserCallOrigin::Bytecode { resolve, invoke } => {
                if caller.instruction != invoke.saturating_add(1) || resolve >= invoke {
                    return false;
                }
                let Some(consumer) = caller_function.code.get(invoke) else {
                    return false;
                };
                let Some(origin) = caller_function.code.get(resolve) else {
                    return false;
                };
                let Ok(resolve_index) = u32::try_from(resolve) else {
                    return false;
                };
                if consumer.opcode != erabasic_bytecode::Opcode::InvokeUserCall as u16
                    || consumer.payload.as_slice() != resolve_index.to_le_bytes()
                    || origin.opcode != erabasic_bytecode::Opcode::ResolveUserCall as u16
                {
                    return false;
                }
                let Ok(spec) = UserCallSpec::decode(&origin.payload) else {
                    return false;
                };
                spec.mode == call.mode
                    && resolve_user_call(program, frame.generation, &target.name, &spec)
                        .is_ok_and(|resolved| resolved.is_some())
            }
            UserCallOrigin::RuntimeForm => caller
                .runtime_form
                .as_ref()
                .is_some_and(|form| form.valid_child_call(frame)),
        }
    }
}
