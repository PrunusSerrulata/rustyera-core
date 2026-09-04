#[allow(clippy::wildcard_imports)]
use super::*;
impl VmDebugInspect for Vm {
    fn stop_token(&self) -> Option<VmStopToken> {
        self.debug.last_stop.as_ref().map(|stop| stop.token)
    }

    fn fibers(
        &self,
        stop: VmStopToken,
        cursor: Option<usize>,
        limit: usize,
    ) -> Result<VmDebugPage<VmDebugFiber>, VmError> {
        self.validate_stop(stop)?;
        let (start, limit) = page_bounds(cursor, limit)?;
        let values = self
            .fibers
            .values()
            .skip(start)
            .take(limit)
            .map(|fiber| VmDebugFiber {
                id: fiber.id,
                status: fiber.public_status(),
                primary: Some(fiber.id) == self.primary_fiber,
                frame_count: fiber.frames.len(),
            })
            .collect::<Vec<_>>();
        let consumed = start.saturating_add(values.len());
        Ok(VmDebugPage {
            values,
            next_cursor: (consumed < self.fibers.len()).then_some(consumed),
        })
    }

    fn call_stack(&self, stop: VmStopToken, fiber: FiberId) -> Result<Vec<VmDebugFrame>, VmError> {
        self.validate_stop(stop)?;
        let fiber = self
            .fibers
            .get(&fiber)
            .ok_or(VmError::UnknownFiber(fiber))?;
        Ok(fiber
            .frames
            .iter()
            .rev()
            .map(|frame| {
                let name = self
                    .generations
                    .get(&frame.generation)
                    .and_then(|generation| {
                        generation
                            .artifact
                            .functions
                            .iter()
                            .find(|function| function.key == frame.function)
                    })
                    .map_or_else(String::new, |function| function.name.clone());
                VmDebugFrame {
                    id: frame.id,
                    generation: frame.generation,
                    function: frame.function,
                    function_name: name,
                    instruction: u32::try_from(frame.instruction).unwrap_or(u32::MAX),
                    source: self.debug_frame_source(frame),
                }
            })
            .collect())
    }

    fn operand_stack(
        &self,
        stop: VmStopToken,
        fiber: FiberId,
        frame: crate::FrameId,
        cursor: Option<usize>,
        limit: usize,
    ) -> Result<VmDebugPage<VmDebugOperand>, VmError> {
        self.validate_stop(stop)?;
        let (start, limit) = page_bounds(cursor, limit)?;
        let frame = self
            .fibers
            .get(&fiber)
            .ok_or(VmError::UnknownFiber(fiber))?
            .frames
            .iter()
            .find(|value| value.id == frame)
            .ok_or_else(|| VmError::InvalidArguments("unknown debugger frame".into()))?;
        let values = frame
            .stack
            .iter()
            .enumerate()
            .skip(start)
            .take(limit)
            .map(|(offset, value)| VmDebugOperand {
                offset,
                value: value.clone(),
            })
            .collect::<Vec<_>>();
        let consumed = start.saturating_add(values.len());
        Ok(VmDebugPage {
            values,
            next_cursor: (consumed < frame.stack.len()).then_some(consumed),
        })
    }

    fn variables(
        &self,
        stop: VmStopToken,
        cursor: Option<usize>,
        limit: usize,
    ) -> Result<VmDebugPage<VmDebugVariable>, VmError> {
        self.validate_stop(stop)?;
        let (start, limit) = page_bounds(cursor, limit)?;
        Ok(debug_variable_page(self, start, limit))
    }

    fn read_variable(
        &self,
        stop: VmStopToken,
        target: &VmDebugVariableRef,
    ) -> Result<VmDebugVariable, VmError> {
        self.validate_stop(stop)?;
        self.read_debug_variable(target)
    }
}

fn debug_variable_page(vm: &Vm, start: usize, limit: usize) -> VmDebugPage<VmDebugVariable> {
    let end = start.saturating_add(limit);
    let mut position = 0usize;
    let mut values = Vec::with_capacity(limit);
    let mut has_more = false;
    'generations: for (generation_id, generation) in &vm.generations {
        for definition in &generation.artifact.globals {
            let indices = vec![0; definition.dimensions.len()];
            match definition.storage {
                BytecodeStorage::Character => {
                    let character_count = vm.memory.characters.len();
                    let skip = start.saturating_sub(position).min(character_count);
                    position = position.saturating_add(skip);
                    for character in skip..character_count {
                        if !collect_debug_variable(
                            vm,
                            &VmDebugVariableRef {
                                target: PlaceDescriptor {
                                    backing: None,
                                    variable: definition.key,
                                    indices: indices.clone(),
                                    character: Some(character as u64),
                                    fiber: None,
                                    frame: None,
                                },
                                generation: *generation_id,
                            },
                            &mut position,
                            start,
                            end,
                            &mut values,
                            &mut has_more,
                        ) {
                            break 'generations;
                        }
                    }
                }
                BytecodeStorage::FunctionLocal => {
                    for fiber in vm.fibers.values() {
                        for frame in &fiber.frames {
                            if frame.generation == *generation_id
                                && frame.locals.contains_key(&definition.key)
                            {
                                if position < start {
                                    position = position.saturating_add(1);
                                } else if !collect_debug_variable(
                                    vm,
                                    &VmDebugVariableRef {
                                        target: PlaceDescriptor {
                                            backing: None,
                                            variable: definition.key,
                                            indices: indices.clone(),
                                            character: None,
                                            fiber: Some(fiber.id),
                                            frame: Some(frame.id),
                                        },
                                        generation: *generation_id,
                                    },
                                    &mut position,
                                    start,
                                    end,
                                    &mut values,
                                    &mut has_more,
                                ) {
                                    break 'generations;
                                }
                            }
                        }
                    }
                }
                _ => {
                    if position < start {
                        position = position.saturating_add(1);
                    } else if !collect_debug_variable(
                        vm,
                        &VmDebugVariableRef {
                            target: PlaceDescriptor {
                                backing: None,
                                variable: definition.key,
                                indices,
                                character: None,
                                fiber: None,
                                frame: None,
                            },
                            generation: *generation_id,
                        },
                        &mut position,
                        start,
                        end,
                        &mut values,
                        &mut has_more,
                    ) {
                        break 'generations;
                    }
                }
            }
        }
    }
    finish_debug_variable_page(values, has_more, end)
}

fn finish_debug_variable_page(
    values: Vec<VmDebugVariable>,
    has_more: bool,
    end: usize,
) -> VmDebugPage<VmDebugVariable> {
    VmDebugPage {
        values,
        next_cursor: has_more.then_some(end),
    }
}

fn collect_debug_variable(
    vm: &Vm,
    reference: &VmDebugVariableRef,
    position: &mut usize,
    start: usize,
    end: usize,
    values: &mut Vec<VmDebugVariable>,
    has_more: &mut bool,
) -> bool {
    if *position >= end {
        *has_more = true;
        return false;
    }
    if *position >= start
        && let Ok(value) = vm.read_debug_variable(reference)
    {
        values.push(value);
    }
    *position = position.saturating_add(1);
    true
}
