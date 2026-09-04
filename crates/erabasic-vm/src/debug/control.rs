#[allow(clippy::wildcard_imports)]
use super::*;
impl VmDebugControl for Vm {
    fn request_pause(&mut self) -> Result<VmDebugStop, VmError> {
        if self.debug.paused {
            return Err(VmError::InvalidState("VM is already debug-paused".into()));
        }
        let selected = self
            .primary_fiber
            .filter(|id| self.fibers.contains_key(id))
            .or_else(|| self.fibers.keys().next().copied());
        Ok(self.make_debug_stop(VmDebugStopReason::PauseRequested, selected))
    }

    fn continue_execution(&mut self, stop: VmStopToken) -> Result<(), VmError> {
        self.validate_stop(stop)?;
        if matches!(
            self.debug.last_stop.as_ref().map(|value| &value.reason),
            Some(VmDebugStopReason::Breakpoint(_))
        ) && let Some(fiber_id) = self.debug.selected
            && let Some(frame) = self
                .fibers
                .get(&fiber_id)
                .and_then(|fiber| fiber.frames.last())
        {
            self.debug.resume_skip = Some((
                fiber_id,
                frame.generation,
                frame.function,
                frame.instruction,
            ));
        }
        self.debug.paused = false;
        self.debug.last_stop = None;
        Ok(())
    }

    fn step(&mut self, stop: VmStopToken, fiber: FiberId, kind: VmStepKind) -> Result<(), VmError> {
        self.validate_stop(stop)?;
        let value = self
            .fibers
            .get(&fiber)
            .ok_or(VmError::UnknownFiber(fiber))?;
        if !matches!(value.state, FiberState::Runnable) {
            return Err(VmError::InvalidState(
                "only a runnable fiber can be stepped".into(),
            ));
        }
        let frame = value
            .frames
            .last()
            .ok_or_else(|| VmError::InvalidState("step fiber has no frame".into()))?;
        self.debug.step = Some(StepPlan {
            fiber,
            kind,
            depth: value.frames.len(),
            function: frame.function,
            instruction: frame.instruction,
            line: self
                .debug_frame_source(frame)
                .map(|source| (source.relative_path, source.line)),
        });
        self.debug.paused = false;
        self.debug.last_stop = None;
        Ok(())
    }

    fn write_variables(
        &mut self,
        stop: VmStopToken,
        writes: &[VmDebugVariableWrite],
    ) -> Result<Vec<VmDebugVariable>, VmError> {
        self.validate_stop(stop)?;
        if writes.is_empty() || writes.len() > 1024 {
            return Err(VmError::InvalidArguments(
                "debug variable write batch has an invalid size".into(),
            ));
        }
        if writes
            .iter()
            .any(|write| write.expected_revision != self.debug.revision)
        {
            return Err(VmError::InvalidState(
                "stale debug variable revision".into(),
            ));
        }
        let memory = self.memory.clone();
        let fibers = self.fibers.clone();
        for write in writes {
            if let Err(error) = self.write_debug_variable(write) {
                self.memory = memory;
                self.fibers = fibers;
                return Err(error);
            }
        }
        self.debug.revision = self.debug.revision.saturating_add(1);
        writes
            .iter()
            .map(|write| self.read_debug_variable(&write.target))
            .collect()
    }

    fn update_breakpoints(
        &mut self,
        breakpoints: &[VmBreakpoint],
        remove: &[u64],
    ) -> Result<Vec<VmResolvedBreakpoint>, VmError> {
        if breakpoints
            .len()
            .saturating_add(self.debug.breakpoints.len())
            > 4096
        {
            return Err(VmError::ResourceLimit("debug breakpoints"));
        }
        for id in remove {
            self.debug.breakpoints.remove(id);
        }
        let mut results = Vec::new();
        for breakpoint in breakpoints {
            let previous = self
                .debug
                .breakpoints
                .get(&breakpoint.id)
                .and_then(|record| record.fingerprint);
            let previous_function = self
                .debug
                .breakpoints
                .get(&breakpoint.id)
                .and_then(|record| record.anchor_function);
            let resolved = self.resolve_breakpoint(breakpoint, previous, previous_function);
            self.debug.breakpoints.insert(
                breakpoint.id,
                BreakpointRecord {
                    specification: breakpoint.clone(),
                    bindings: BTreeMap::from([(self.current_generation, resolved.positions)]),
                    fingerprint: resolved.fingerprint,
                    anchor_function: resolved.anchor_function,
                    hit_count: breakpoint.hit_count,
                },
            );
            results.push(resolved.public);
        }
        results.sort_by_key(|value| value.id);
        Ok(results)
    }
}

impl Vm {
    pub(crate) fn debug_rebind_breakpoints(&mut self) {
        let specifications = self
            .debug
            .breakpoints
            .values()
            .map(|record| {
                (
                    record.specification.clone(),
                    record.fingerprint,
                    record.anchor_function,
                    record.hit_count,
                )
            })
            .collect::<Vec<_>>();
        for (mut specification, fingerprint, function, hit_count) in specifications {
            specification.hit_count = hit_count;
            let resolved = self.resolve_breakpoint(&specification, fingerprint, function);
            if let Some(record) = self.debug.breakpoints.get_mut(&specification.id) {
                record
                    .bindings
                    .insert(self.current_generation, resolved.positions);
                record.fingerprint = resolved.fingerprint;
                record.anchor_function = resolved.anchor_function;
                record.hit_count = hit_count;
            }
        }
    }
}

pub(super) fn instruction_index(
    function: &erabasic_bytecode::BytecodeFunction,
    target_offset: u64,
) -> Option<usize> {
    let mut offset = 0;
    for (index, instruction) in function.code.iter().enumerate() {
        if offset == target_offset {
            return Some(index);
        }
        offset = offset.saturating_add(instruction.encoded_len());
    }
    None
}
