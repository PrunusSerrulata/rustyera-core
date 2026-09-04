#[allow(clippy::wildcard_imports)]
use super::super::*;

impl Vm {
    #[allow(clippy::too_many_lines)]
    pub(in crate::interpreter) fn dispatch_terminal(
        &mut self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        opcode: Opcode,
        policy: ExecutionPolicy,
    ) -> Result<Option<StepOutcome>, StepError> {
        fiber
            .frames
            .last()
            .ok_or_else(|| StepError::new(VmFaultCode::InvalidInstruction, "missing frame"))?;
        match opcode {
            Opcode::JumpDynamicLabel => {
                let missing_target = read_u32(position.encoded.payload, 0)? as usize;
                let VmValue::String(name) =
                    pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?
                else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "dynamic label target must be a string",
                    ));
                };
                let generation = self
                    .generations
                    .get(&position.generation)
                    .expect("validated frame generation exists");
                let function = generation
                    .function(position.function)
                    .expect("validated function exists");
                let target = function
                    .labels
                    .iter()
                    .find(|label| label.name.eq_ignore_ascii_case(&name))
                    .map_or(missing_target, |label| label.instruction as usize);
                let entered_structured_block =
                    self.reconcile_structured_jump(fiber, position, target);
                fiber.frames.last_mut().expect("frame exists").instruction = target;
                if entered_structured_block {
                    return Ok(Some(StepOutcome::Diagnostic {
                        code: STRUCTURED_GOTO_DIAGNOSTIC_CODE,
                        message: STRUCTURED_GOTO_DIAGNOSTIC_MESSAGE,
                        notification: crate::VmDiagnosticNotification::LogOnly,
                    }));
                }
            }
            Opcode::InvokeEvent => {
                let VmValue::String(name) =
                    pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?
                else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "event target must be a string",
                    ));
                };
                if fiber.frames.last().expect("frame exists").event_context {
                    return Err(StepError::new(
                        VmFaultCode::Trap,
                        "CALLEVENT is not allowed inside an event dispatch",
                    ));
                }
                let non_event_target = self
                    .generations
                    .get(&position.generation)
                    .expect("validated frame generation exists")
                    .artifact
                    .functions
                    .iter()
                    .any(|function| {
                        function.name.eq_ignore_ascii_case(&name)
                            && function.kind != BytecodeFunctionKind::Event
                    });
                if !self.start_event_dispatch(fiber, position.generation, &name)? {
                    if non_event_target {
                        return Err(StepError::new(
                            VmFaultCode::TypeMismatch,
                            format!("CALLEVENT target {name} is not an event"),
                        ));
                    }
                    return Ok(Some(StepOutcome::Continue));
                }
            }
            Opcode::Return => {
                let has_value = position.encoded.payload.first().copied().unwrap_or(0) != 0;
                let value = has_value
                    .then(|| pop(&mut fiber.frames.last_mut().expect("frame exists").stack))
                    .transpose()?;
                match self
                    .return_frame(
                        fiber,
                        value,
                        Some(position.instruction),
                        policy.allow_function_memo,
                    )
                    .map_err(map_vm_error)?
                {
                    crate::state::FrameReturn::Continue => {}
                    crate::state::FrameReturn::Completed(value) => {
                        return Ok(Some(StepOutcome::Completed(value)));
                    }
                }
            }
            Opcode::Yield => return Ok(Some(StepOutcome::Yielded)),
            Opcode::AwaitResume => {
                let tag = *position.encoded.payload.first().ok_or_else(|| {
                    StepError::new(VmFaultCode::InvalidInstruction, "missing resume type")
                })?;
                let expected = opcode::decode_type(tag).ok_or_else(|| {
                    StepError::new(VmFaultCode::InvalidInstruction, "invalid resume type")
                })?;
                fiber.state = FiberState::WaitingResume(expected);
                return Ok(Some(StepOutcome::Blocked));
            }
            Opcode::Trap => {
                let message = String::from_utf8_lossy(position.encoded.payload);
                return Err(StepError::new(VmFaultCode::Trap, message));
            }
            _ => return Ok(None),
        }
        Ok(Some(StepOutcome::Continue))
    }
}
