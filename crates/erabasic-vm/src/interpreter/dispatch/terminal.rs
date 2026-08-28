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
                let frame_id = self.allocate_frame_id();
                let generation = self
                    .generations
                    .get(&position.generation)
                    .expect("validated frame generation exists");
                let artifact = &generation.artifact;
                let Some(group) = artifact
                    .event_groups
                    .iter()
                    .find(|group| group.name.eq_ignore_ascii_case(&name))
                else {
                    if artifact.functions.iter().any(|function| {
                        function.name.eq_ignore_ascii_case(&name)
                            && function.kind != BytecodeFunctionKind::Event
                    }) {
                        return Err(StepError::new(
                            VmFaultCode::TypeMismatch,
                            format!("CALLEVENT target {name} is not an event"),
                        ));
                    }
                    return Ok(Some(StepOutcome::Continue));
                };
                let mut pending = std::collections::VecDeque::new();
                let groups: &[(&[erabasic_bytecode::BytecodeEventEntry], u8)] =
                    if group.only.is_empty() {
                        &[(&group.priority, 1), (&group.normal, 2), (&group.later, 3)]
                    } else {
                        &[(&group.only, 0)]
                    };
                for (entries, group_id) in groups {
                    pending.extend(entries.iter().map(|entry| EventDispatchEntry {
                        function: entry.function,
                        single: entry.single,
                        group: *group_id,
                    }));
                }
                let Some(active) = pending.pop_front() else {
                    return Ok(Some(StepOutcome::Continue));
                };
                if fiber.frames.len() >= self.config.maximum_call_depth {
                    return Err(StepError::new(
                        VmFaultCode::ResourceLimit,
                        "maximum call depth exceeded",
                    ));
                }
                let target = generation.function(active.function).ok_or_else(|| {
                    StepError::new(VmFaultCode::MissingSymbol, "event function is missing")
                })?;
                self.memory.ensure_function_statics(
                    position.generation,
                    target.key,
                    generation.function_statics(target.key),
                );
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .event_dispatch = Some(EventDispatch { active, pending });
                fiber.frames.push(make_frame(
                    frame_id,
                    position.generation,
                    target,
                    generation.function_locals(target.key),
                    Vec::new(),
                    false,
                    true,
                ));
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
