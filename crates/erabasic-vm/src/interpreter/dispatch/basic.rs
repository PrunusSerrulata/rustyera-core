#[allow(clippy::wildcard_imports)]
use super::super::*;

impl Vm {
    #[allow(clippy::too_many_lines)]
    pub(in crate::interpreter) fn dispatch_basic(
        &mut self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        opcode: Opcode,
        policy: ExecutionPolicy,
    ) -> Result<Option<StepOutcome>, StepError> {
        let frame = fiber
            .frames
            .last_mut()
            .ok_or_else(|| StepError::new(VmFaultCode::InvalidInstruction, "missing frame"))?;
        match opcode {
            Opcode::Nop => {}
            Opcode::PushInteger => {
                let bytes = exact::<8>(position.encoded.payload)?;
                frame
                    .stack
                    .push(VmValue::Integer(i64::from_le_bytes(bytes)));
            }
            Opcode::PushString => {
                let length = read_u32(position.encoded.payload, 0)? as usize;
                let bytes = position
                    .encoded
                    .payload
                    .get(4..4 + length)
                    .filter(|_| position.encoded.payload.len() == 4 + length)
                    .ok_or_else(|| {
                        StepError::new(VmFaultCode::InvalidInstruction, "invalid string operand")
                    })?;
                let value = std::str::from_utf8(bytes).map_err(|_| {
                    StepError::new(VmFaultCode::InvalidInstruction, "string is not UTF-8")
                })?;
                frame.stack.push(VmValue::String(value.into()));
            }
            Opcode::LoadVariable | Opcode::StoreVariable | Opcode::MakePlace => {
                let count = read_u16(position.encoded.payload, 16)? as usize;
                let operation = *position.encoded.payload.get(18).ok_or_else(|| {
                    StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "missing variable operation",
                    )
                })?;
                let value = (opcode == Opcode::StoreVariable)
                    .then(|| pop(&mut fiber.frames.last_mut().expect("frame exists").stack))
                    .transpose()?;
                let indices = pop_indices(
                    &mut fiber.frames.last_mut().expect("frame exists").stack,
                    count,
                )?;
                let frame = fiber.frames.last().expect("frame exists");
                let definition = position.variable.ok_or_else(|| {
                    StepError::new(
                        VmFaultCode::MissingSymbol,
                        "variable operand does not identify a defined global",
                    )
                })?;
                let key = definition.key;
                let character = if definition.storage == BytecodeStorage::Character {
                    if indices.as_slice().len() > definition.dimensions.len() {
                        Some(indices.as_slice()[0])
                    } else {
                        Some(self.target_character_for_generation(position.generation) as u64)
                    }
                } else {
                    None
                };
                let value_indices = if character.is_some()
                    && indices.as_slice().len() > definition.dimensions.len()
                {
                    &indices.as_slice()[1..]
                } else {
                    indices.as_slice()
                };
                if opcode == Opcode::MakePlace {
                    let place = PlaceDescriptor {
                        variable: key,
                        indices: value_indices.to_vec(),
                        character,
                        fiber: Some(fiber.id),
                        frame: (definition.storage == BytecodeStorage::FunctionLocal)
                            .then_some(frame.id),
                    };
                    let value = match definition.value_type {
                        BytecodeType::Integer => VmValue::IntegerPlace(Box::new(place)),
                        BytecodeType::String => VmValue::StringPlace(Box::new(place)),
                        BytecodeType::IntegerPlace | BytecodeType::StringPlace => {
                            return Err(StepError::new(
                                VmFaultCode::InvalidInstruction,
                                "a variable schema cannot contain place values",
                            ));
                        }
                    };
                    fiber
                        .frames
                        .last_mut()
                        .expect("frame exists")
                        .stack
                        .push(value);
                } else if opcode == Opcode::LoadVariable {
                    let value = self
                        .read_variable_resolved(
                            fiber,
                            position.generation,
                            definition,
                            value_indices,
                            character,
                            (definition.storage == BytecodeStorage::FunctionLocal)
                                .then_some(frame.id),
                        )
                        .map_err(map_vm_error)?;
                    fiber
                        .frames
                        .last_mut()
                        .expect("frame exists")
                        .stack
                        .push(value);
                } else {
                    let mut value = value.expect("store value was popped");
                    if operation != 0 {
                        let previous = self
                            .read_variable_resolved(
                                fiber,
                                position.generation,
                                definition,
                                value_indices,
                                character,
                                (definition.storage == BytecodeStorage::FunctionLocal)
                                    .then_some(frame.id),
                            )
                            .map_err(map_vm_error)?;
                        value = binary_value(assign_binary_tag(operation)?, previous, value)?;
                    }
                    self.write_variable_resolved(
                        fiber,
                        position.generation,
                        definition,
                        value_indices,
                        character,
                        (definition.storage == BytecodeStorage::FunctionLocal).then_some(frame.id),
                        value,
                    )
                    .map_err(map_vm_error)?;
                }
            }
            Opcode::Unary => {
                let operation = *position.encoded.payload.first().ok_or_else(|| {
                    StepError::new(VmFaultCode::InvalidInstruction, "missing unary operation")
                })?;
                let value = pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?;
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .stack
                    .push(unary_value(operation, value)?);
            }
            Opcode::Binary => {
                let operation = *position.encoded.payload.first().ok_or_else(|| {
                    StepError::new(VmFaultCode::InvalidInstruction, "missing binary operation")
                })?;
                let stack = &mut fiber.frames.last_mut().expect("frame exists").stack;
                let right = pop(stack)?;
                let left = pop(stack)?;
                stack.push(binary_value(operation, left, right)?);
            }
            Opcode::ToString => {
                let value = pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?;
                let value = match value {
                    VmValue::Integer(value) => value.to_string(),
                    VmValue::String(value) => value,
                    VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => {
                        match self.read_place(fiber, &place).map_err(map_vm_error)? {
                            VmValue::Integer(value) => value.to_string(),
                            VmValue::String(value) => value,
                            VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => {
                                return Err(StepError::new(
                                    VmFaultCode::TypeMismatch,
                                    "a place cannot contain another place",
                                ));
                            }
                        }
                    }
                };
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .stack
                    .push(VmValue::String(value));
            }
            Opcode::Pop => {
                pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?;
            }
            Opcode::Dup => {
                let stack = &mut fiber.frames.last_mut().expect("frame exists").stack;
                let value = stack.last().cloned().ok_or_else(|| {
                    StepError::new(VmFaultCode::InvalidInstruction, "dup underflows the stack")
                })?;
                stack.push(value);
            }
            Opcode::StorePlace => {
                let place = pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?;
                let value = pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?;
                let place = match place {
                    VmValue::IntegerPlace(place) if matches!(value, VmValue::Integer(_)) => place,
                    VmValue::StringPlace(place) if matches!(value, VmValue::String(_)) => place,
                    _ => {
                        return Err(StepError::new(
                            VmFaultCode::TypeMismatch,
                            "indirect store place and value types differ",
                        ));
                    }
                };
                self.write_place(fiber, &place, value)
                    .map_err(map_vm_error)?;
            }
            Opcode::Concat => {
                let count = read_u16(position.encoded.payload, 0)? as usize;
                let stack = &mut fiber.frames.last_mut().expect("frame exists").stack;
                let mut parts = Vec::with_capacity(count);
                for _ in 0..count {
                    let VmValue::String(part) = pop(stack)? else {
                        return Err(StepError::new(
                            VmFaultCode::TypeMismatch,
                            "concat expects strings",
                        ));
                    };
                    parts.push(part);
                }
                parts.reverse();
                stack.push(VmValue::String(parts.concat()));
            }
            Opcode::ForStart => {
                let stack = &mut fiber.frames.last_mut().expect("frame exists").stack;
                let VmValue::Integer(step) = pop(stack)? else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "FOR step expects an integer",
                    ));
                };
                let VmValue::Integer(end) = pop(stack)? else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "FOR end expects an integer",
                    ));
                };
                let VmValue::Integer(start) = pop(stack)? else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "FOR start expects an integer",
                    ));
                };
                let VmValue::IntegerPlace(counter) = pop(stack)? else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "FOR counter expects an integer place",
                    ));
                };
                self.write_place(fiber, &counter, VmValue::Integer(start))
                    .map_err(map_vm_error)?;
                let active = (step > 0 && start < end) || (step < 0 && start > end);
                if active
                    && let Some(additional_instructions) = self
                        .try_bulk_fill_loop(fiber, position, &counter, start, end, step, policy)?
                {
                    return Ok(Some(StepOutcome::BulkProgress(additional_instructions)));
                }
                if active {
                    fiber
                        .frames
                        .last_mut()
                        .expect("frame exists")
                        .for_loops
                        .push(ForLoopState {
                            counter: *counter,
                            end,
                            step,
                        });
                }
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .stack
                    .push(VmValue::Integer(i64::from(active)));
            }
            Opcode::ForNext => {
                let state = frame.for_loops.last().cloned().ok_or_else(|| {
                    StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "NEXT has no active FOR loop",
                    )
                })?;
                if state.is_bypassed() {
                    frame.for_loops.pop();
                    frame.stack.push(VmValue::Integer(0));
                    return Ok(Some(StepOutcome::Continue));
                }
                let VmValue::Integer(current) = self
                    .read_place(fiber, &state.counter)
                    .map_err(map_vm_error)?
                else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "FOR counter storage is not integer",
                    ));
                };
                let next = current.wrapping_add(state.step);
                self.write_place(fiber, &state.counter, VmValue::Integer(next))
                    .map_err(map_vm_error)?;
                let active =
                    (state.step > 0 && next < state.end) || (state.step < 0 && next > state.end);
                let frame = fiber.frames.last_mut().expect("frame exists");
                if !active {
                    frame.for_loops.pop();
                }
                frame.stack.push(VmValue::Integer(i64::from(active)));
            }
            Opcode::ForBreak => {
                let state = frame.for_loops.last().cloned().ok_or_else(|| {
                    StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "BREAK has no active FOR loop",
                    )
                })?;
                if state.is_bypassed() {
                    frame.for_loops.pop();
                    return Ok(Some(StepOutcome::Continue));
                }
                let VmValue::Integer(current) = self
                    .read_place(fiber, &state.counter)
                    .map_err(map_vm_error)?
                else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "FOR counter storage is not integer",
                    ));
                };
                self.write_place(
                    fiber,
                    &state.counter,
                    VmValue::Integer(current.wrapping_add(state.step)),
                )
                .map_err(map_vm_error)?;
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .for_loops
                    .pop()
                    .expect("active FOR loop was checked");
            }
            Opcode::SelectStart => {
                let value = pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?;
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .select_values
                    .push(value);
            }
            Opcode::SelectCompare => {
                let operation = *position.encoded.payload.first().ok_or_else(|| {
                    StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "CASE comparison operation is missing",
                    )
                })?;
                let selector = fiber
                    .frames
                    .last()
                    .and_then(|frame| frame.select_values.last())
                    .cloned();
                if selector.as_ref().is_some_and(is_bypassed_select_value) {
                    let stack = &mut fiber.frames.last_mut().expect("frame exists").stack;
                    let operands = match operation {
                        6 => 2,
                        8 => 0,
                        _ => 1,
                    };
                    for _ in 0..operands {
                        pop(stack)?;
                    }
                    stack.push(VmValue::Integer(0));
                    return Ok(Some(StepOutcome::Continue));
                }
                let selector = selector.ok_or_else(|| {
                    StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "CASE is outside SELECTCASE",
                    )
                })?;
                let stack = &mut fiber.frames.last_mut().expect("frame exists").stack;
                let matched = if operation == 8 {
                    true
                } else if operation == 6 {
                    let upper = pop(stack)?;
                    let lower = pop(stack)?;
                    let VmValue::Integer(lower_match) = binary_value(10, selector.clone(), lower)?
                    else {
                        unreachable!("comparison produces integer")
                    };
                    let VmValue::Integer(upper_match) = binary_value(8, selector, upper)? else {
                        unreachable!("comparison produces integer")
                    };
                    lower_match != 0 && upper_match != 0
                } else {
                    let operand = pop(stack)?;
                    let binary_operation = match operation {
                        0 => 11,
                        1 => 12,
                        2 => 7,
                        3 => 8,
                        4 => 9,
                        5 => 10,
                        7 => 13,
                        _ => {
                            return Err(StepError::new(
                                VmFaultCode::InvalidInstruction,
                                "unknown CASE comparison operation",
                            ));
                        }
                    };
                    let VmValue::Integer(value) =
                        binary_value(binary_operation, selector, operand)?
                    else {
                        unreachable!("comparison produces integer")
                    };
                    value != 0
                };
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .stack
                    .push(VmValue::Integer(i64::from(matched)));
            }
            Opcode::SelectEnd => {
                frame.select_values.pop().ok_or_else(|| {
                    StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "ENDSELECT is outside SELECTCASE",
                    )
                })?;
            }
            Opcode::Jump | Opcode::JumpIfFalse => {
                let target = read_u32(position.encoded.payload, 0)? as usize;
                let take = if opcode == Opcode::JumpIfFalse {
                    let VmValue::Integer(condition) =
                        pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?
                    else {
                        return Err(StepError::new(
                            VmFaultCode::TypeMismatch,
                            "conditional jump expects an integer",
                        ));
                    };
                    condition == 0
                } else {
                    true
                };
                if take {
                    let entered_structured_block =
                        self.reconcile_structured_jump(fiber, position, target);
                    if target <= position.instruction {
                        fiber.backward_branches_without_progress =
                            fiber.backward_branches_without_progress.saturating_add(1);
                    }
                    fiber.frames.last_mut().expect("frame exists").instruction = target;
                    if entered_structured_block {
                        return Ok(Some(StepOutcome::Diagnostic {
                            code: STRUCTURED_GOTO_DIAGNOSTIC_CODE,
                            message: STRUCTURED_GOTO_DIAGNOSTIC_MESSAGE,
                        }));
                    }
                }
            }
            _ => return Ok(None),
        }
        Ok(Some(StepOutcome::Continue))
    }
}
