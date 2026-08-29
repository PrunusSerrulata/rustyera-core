use std::collections::BTreeMap;

use erabasic_bytecode::{BytecodeStorage, EncodedInstruction, SymbolKey};

use crate::state::Frame;
use crate::{
    Fiber, FiberId, FiberState, GenerationId, PlaceDescriptor, Vm, VmBreakpoint,
    VmBreakpointBinding, VmBreakpointLocation, VmDebugControl, VmDebugFiber, VmDebugFrame,
    VmDebugInspect, VmDebugOperand, VmDebugPage, VmDebugStop, VmDebugStopReason, VmDebugVariable,
    VmDebugVariableRef, VmDebugVariableWrite, VmError, VmResolvedBreakpoint, VmStepKind,
    VmStopToken,
};

mod pagination;

use pagination::page_bounds;

#[derive(Clone, Debug)]
struct StepPlan {
    fiber: FiberId,
    kind: VmStepKind,
    depth: usize,
    function: SymbolKey,
    instruction: usize,
    line: Option<(String, u64)>,
}

#[derive(Clone, Debug)]
struct BreakpointRecord {
    specification: VmBreakpoint,
    bindings: BTreeMap<GenerationId, Vec<(SymbolKey, usize)>>,
    fingerprint: Option<erabasic_bytecode::Digest>,
    anchor_function: Option<SymbolKey>,
    hit_count: u64,
}

struct BreakpointResolution {
    public: VmResolvedBreakpoint,
    positions: Vec<(SymbolKey, usize)>,
    fingerprint: Option<erabasic_bytecode::Digest>,
    anchor_function: Option<SymbolKey>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DebugState {
    pause_epoch: u64,
    revision: u64,
    paused: bool,
    selected: Option<FiberId>,
    last_stop: Option<VmDebugStop>,
    step: Option<StepPlan>,
    breakpoints: BTreeMap<u64, BreakpointRecord>,
    resume_skip: Option<(FiberId, GenerationId, SymbolKey, usize)>,
}

impl Vm {
    pub(crate) fn debug_is_paused(&self) -> bool {
        self.debug.paused
    }

    pub(crate) fn debug_checks_active(&self) -> bool {
        self.debug.resume_skip.is_some()
            || self.debug.step.is_some()
            || !self.debug.breakpoints.is_empty()
    }

    pub(crate) fn debug_step_fiber(&self) -> Option<FiberId> {
        self.debug.step.as_ref().map(|step| step.fiber)
    }

    pub(crate) fn debug_retained_terminal_fiber(&self) -> Option<FiberId> {
        self.debug.paused.then_some(self.debug.selected).flatten()
    }

    pub(crate) fn debug_stop_before(&mut self, fiber: &Fiber) -> Option<VmDebugStop> {
        let frame = fiber.frames.last()?;
        let position = (
            fiber.id,
            frame.generation,
            frame.function,
            frame.instruction,
        );
        if self.debug.resume_skip == Some(position) {
            self.debug.resume_skip = None;
            return None;
        }
        let breakpoint = self.debug.breakpoints.iter_mut().find_map(|(id, record)| {
            if !record.specification.enabled {
                return None;
            }
            let matched = record
                .bindings
                .get(&frame.generation)
                .is_some_and(|values| values.contains(&(frame.function, frame.instruction)))
                .then_some(*id);
            if matched.is_some() {
                record.hit_count = record.hit_count.saturating_add(1);
            }
            matched
        })?;
        Some(self.make_debug_stop_for_fiber(VmDebugStopReason::Breakpoint(breakpoint), fiber))
    }

    pub(crate) fn debug_stop_after(
        &mut self,
        fiber: &Fiber,
        blocked: bool,
        completed: bool,
    ) -> Option<VmDebugStop> {
        let step = self.debug.step.as_ref()?;
        if step.fiber != fiber.id {
            return None;
        }
        let reason = if blocked {
            Some(VmDebugStopReason::HostWait)
        } else if completed {
            Some(VmDebugStopReason::FiberCompleted)
        } else {
            let frame = fiber.frames.last()?;
            let depth = fiber.frames.len();
            let current_line = self
                .debug_frame_source(frame)
                .map(|source| (source.relative_path, source.line));
            match step.kind {
                VmStepKind::Instruction => Some(VmDebugStopReason::StepCompleted),
                VmStepKind::SourceLine => {
                    (current_line != step.line).then_some(VmDebugStopReason::StepCompleted)
                }
                VmStepKind::Into => (depth != step.depth
                    || frame.function != step.function
                    || frame.instruction != step.instruction.saturating_add(1))
                .then_some(VmDebugStopReason::StepCompleted),
                VmStepKind::Over => (depth < step.depth
                    || (depth == step.depth && current_line != step.line))
                    .then_some(VmDebugStopReason::StepCompleted),
                VmStepKind::Out => (depth < step.depth).then_some(VmDebugStopReason::StepCompleted),
            }
        }?;
        self.debug.step = None;
        Some(self.make_debug_stop_for_fiber(reason, fiber))
    }

    fn make_debug_stop_for_fiber(
        &mut self,
        reason: VmDebugStopReason,
        fiber: &Fiber,
    ) -> VmDebugStop {
        let mut stop = self.make_debug_stop(reason, Some(fiber.id));
        stop.source = fiber
            .frames
            .last()
            .and_then(|frame| self.debug_frame_source(frame));
        self.debug.last_stop = Some(stop.clone());
        stop
    }

    fn make_debug_stop(
        &mut self,
        reason: VmDebugStopReason,
        selected: Option<FiberId>,
    ) -> VmDebugStop {
        self.debug.pause_epoch = self.debug.pause_epoch.saturating_add(1);
        self.debug.paused = true;
        self.debug.selected = selected;
        let source = selected
            .and_then(|id| self.fibers.get(&id))
            .and_then(|fiber| fiber.frames.last())
            .and_then(|frame| self.debug_frame_source(frame));
        let stop = VmDebugStop {
            token: VmStopToken {
                pause_epoch: self.debug.pause_epoch,
                generation: self.current_generation,
            },
            reason,
            selected_fiber: selected,
            source,
        };
        self.debug.last_stop = Some(stop.clone());
        stop
    }

    fn validate_stop(&self, stop: VmStopToken) -> Result<(), VmError> {
        if !self.debug.paused
            || stop.pause_epoch != self.debug.pause_epoch
            || stop.generation != self.current_generation
        {
            return Err(VmError::InvalidState("stale debugger stop token".into()));
        }
        Ok(())
    }

    fn debug_frame_source(
        &self,
        frame: &Frame,
    ) -> Option<erabasic_bytecode::ResolvedSourceLocation> {
        let generation = self.generations.get(&frame.generation)?;
        let function = generation
            .artifact
            .functions
            .iter()
            .find(|value| value.key == frame.function)?;
        let offset = function
            .code
            .iter()
            .take(frame.instruction)
            .map(EncodedInstruction::encoded_len)
            .sum();
        generation
            .artifact
            .source_map
            .resolve(frame.function, offset)
    }

    fn read_debug_variable(&self, target: &VmDebugVariableRef) -> Result<VmDebugVariable, VmError> {
        let generation = self
            .generations
            .get(&target.generation)
            .ok_or_else(|| VmError::InvalidState("debug variable generation is missing".into()))?;
        let definition = generation
            .artifact
            .globals
            .iter()
            .find(|value| value.key == target.target.variable)
            .ok_or_else(|| VmError::InvalidArguments("unknown debug variable".into()))?;
        if target.target.indices.len() != definition.dimensions.len() {
            return Err(VmError::InvalidArguments(
                "debug variable requires one index per dimension".into(),
            ));
        }
        let value = if definition.storage == BytecodeStorage::FunctionLocal {
            let fiber = target
                .target
                .fiber
                .and_then(|id| self.fibers.get(&id))
                .ok_or_else(|| {
                    VmError::InvalidArguments("local variable fiber is missing".into())
                })?;
            let frame = target
                .target
                .frame
                .and_then(|id| fiber.frames.iter().find(|frame| frame.id == id))
                .filter(|frame| frame.generation == target.generation)
                .ok_or_else(|| {
                    VmError::InvalidArguments("local variable frame is missing".into())
                })?;
            if generation.is_reference_variable(definition.key) {
                self.read_place(fiber, &target.target)?
            } else {
                frame
                    .locals
                    .get(&definition.key)
                    .ok_or_else(|| {
                        VmError::InvalidArguments("local variable is unavailable".into())
                    })?
                    .read(&target.target.indices)
                    .map_err(VmError::InvalidArguments)?
            }
        } else {
            let character = usize::try_from(target.target.character.unwrap_or(0))
                .map_err(|_| VmError::InvalidArguments("character index is too large".into()))?;
            self.memory
                .cell(target.generation, definition, character)
                .ok_or_else(|| VmError::InvalidArguments("variable storage is unavailable".into()))?
                .read(&target.target.indices)
                .map_err(VmError::InvalidArguments)?
        };
        Ok(VmDebugVariable {
            target: target.clone(),
            name: definition.name.clone(),
            mutable: definition.mutable,
            value,
            revision: self.debug.revision,
        })
    }

    fn write_debug_variable(&mut self, write: &VmDebugVariableWrite) -> Result<(), VmError> {
        let generation = self
            .generations
            .get(&write.target.generation)
            .ok_or_else(|| VmError::InvalidState("debug variable generation is missing".into()))?;
        let definition = generation
            .artifact
            .globals
            .iter()
            .find(|value| value.key == write.target.target.variable)
            .cloned()
            .ok_or_else(|| VmError::InvalidArguments("unknown debug variable".into()))?;
        if !definition.mutable || write.value.value_type() != definition.value_type {
            return Err(VmError::InvalidArguments(
                "debug variable is read-only or has a different type".into(),
            ));
        }
        if generation.is_reference_variable(definition.key) {
            if write.target.target.backing.is_some() {
                return Err(VmError::InvalidArguments(
                    "debugger cannot inject an array backing identity".into(),
                ));
            }
            let id =
                write.target.target.fiber.ok_or_else(|| {
                    VmError::InvalidArguments("REF debug fiber is missing".into())
                })?;
            let mut fiber = self
                .fibers
                .remove(&id)
                .ok_or_else(|| VmError::InvalidArguments("REF debug fiber is stale".into()))?;
            let result = self.write_place(&mut fiber, &write.target.target, write.value.clone());
            self.fibers.insert(id, fiber);
            return result;
        }
        if definition.storage == BytecodeStorage::FunctionLocal {
            let fiber = write
                .target
                .target
                .fiber
                .and_then(|id| self.fibers.get_mut(&id))
                .ok_or_else(|| {
                    VmError::InvalidArguments("local variable fiber is missing".into())
                })?;
            let frame = write
                .target
                .target
                .frame
                .and_then(|id| fiber.frames.iter_mut().find(|frame| frame.id == id))
                .filter(|frame| frame.generation == write.target.generation)
                .ok_or_else(|| {
                    VmError::InvalidArguments("local variable frame is missing".into())
                })?;
            frame
                .locals
                .get_mut(&definition.key)
                .ok_or_else(|| VmError::InvalidArguments("local variable is unavailable".into()))?
                .write(&write.target.target.indices, write.value.clone())
                .map_err(VmError::InvalidArguments)
        } else {
            let character = usize::try_from(write.target.target.character.unwrap_or(0))
                .map_err(|_| VmError::InvalidArguments("character index is too large".into()))?;
            self.memory
                .cell_mut(
                    write.target.generation,
                    definition.key,
                    definition.storage,
                    character,
                )
                .ok_or_else(|| VmError::InvalidArguments("variable storage is unavailable".into()))?
                .write(&write.target.target.indices, write.value.clone())
                .map_err(VmError::InvalidArguments)
        }
    }

    fn resolve_breakpoint(
        &self,
        breakpoint: &VmBreakpoint,
        previous_fingerprint: Option<erabasic_bytecode::Digest>,
        previous_function: Option<SymbolKey>,
    ) -> BreakpointResolution {
        let generation = self.current_generation;
        let artifact = self.artifact();
        let mut binding = VmBreakpointBinding::Unbound;
        let mut positions = Vec::new();
        let mut source = None;
        let mut fingerprint = previous_fingerprint;
        let mut anchor_function = previous_function;
        match &breakpoint.location {
            VmBreakpointLocation::Function(key) => {
                if artifact
                    .functions
                    .iter()
                    .any(|function| function.key == *key)
                {
                    positions.push((*key, 0));
                    binding = VmBreakpointBinding::Verified;
                }
            }
            VmBreakpointLocation::Source {
                relative_path,
                content_hash,
                byte_offset,
            } => {
                let source_index = artifact.source_map.sources.iter().position(|candidate| {
                    candidate.relative_path.eq_ignore_ascii_case(relative_path)
                });
                if let Some(source_index) = source_index {
                    let exact_hash =
                        artifact.source_map.sources[source_index].content_hash == *content_hash;
                    let entry = if exact_hash {
                        artifact.source_map.entries.iter().find(|entry| {
                            entry.source_index as usize == source_index
                                && entry.byte_start <= *byte_offset
                                && *byte_offset < entry.byte_end.max(entry.byte_start + 1)
                        })
                    } else if let Some(anchor) = previous_fingerprint {
                        let matches = artifact
                            .source_map
                            .entries
                            .iter()
                            .filter(|entry| {
                                artifact.source_map.statement_fingerprint(entry) == Some(anchor)
                                    && previous_function
                                        .is_none_or(|function| entry.function == function)
                            })
                            .collect::<Vec<_>>();
                        (matches.len() == 1).then_some(matches[0])
                    } else {
                        None
                    };
                    if let Some(entry) = entry {
                        fingerprint = artifact.source_map.statement_fingerprint(entry);
                        anchor_function = Some(entry.function);
                        let instruction = artifact
                            .functions
                            .iter()
                            .find(|function| function.key == entry.function)
                            .and_then(|function| instruction_index(function, entry.code_start));
                        if let Some(instruction) = instruction {
                            positions.push((entry.function, instruction));
                            source = artifact
                                .source_map
                                .resolve(entry.function, entry.code_start);
                            binding = if exact_hash {
                                VmBreakpointBinding::Verified
                            } else {
                                VmBreakpointBinding::Moved
                            };
                        }
                    }
                }
            }
        }
        BreakpointResolution {
            public: VmResolvedBreakpoint {
                id: breakpoint.id,
                generation,
                binding,
                source,
                message: (binding == VmBreakpointBinding::Unbound)
                    .then(|| "breakpoint has no unique target in this generation".into()),
                hit_count: breakpoint.hit_count,
            },
            positions,
            fingerprint,
            anchor_function,
        }
    }
}

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
        let mut references = Vec::new();
        for (generation_id, generation) in &self.generations {
            for definition in &generation.artifact.globals {
                let indices = vec![0; definition.dimensions.len()];
                match definition.storage {
                    BytecodeStorage::Character => {
                        for character in 0..self.memory.characters.len() {
                            references.push(VmDebugVariableRef {
                                target: PlaceDescriptor {
                                    backing: None,
                                    variable: definition.key,
                                    indices: indices.clone(),
                                    character: Some(character as u64),
                                    fiber: None,
                                    frame: None,
                                },
                                generation: *generation_id,
                            });
                        }
                    }
                    BytecodeStorage::FunctionLocal => {
                        for fiber in self.fibers.values() {
                            for frame in &fiber.frames {
                                if frame.generation == *generation_id
                                    && frame.locals.contains_key(&definition.key)
                                {
                                    references.push(VmDebugVariableRef {
                                        target: PlaceDescriptor {
                                            backing: None,
                                            variable: definition.key,
                                            indices: indices.clone(),
                                            character: None,
                                            fiber: Some(fiber.id),
                                            frame: Some(frame.id),
                                        },
                                        generation: *generation_id,
                                    });
                                }
                            }
                        }
                    }
                    _ => references.push(VmDebugVariableRef {
                        target: PlaceDescriptor {
                            backing: None,
                            variable: definition.key,
                            indices,
                            character: None,
                            fiber: None,
                            frame: None,
                        },
                        generation: *generation_id,
                    }),
                }
            }
        }
        let consumed = start.saturating_add(limit).min(references.len());
        let values = references
            .get(start..consumed)
            .unwrap_or_default()
            .iter()
            .filter_map(|reference| self.read_debug_variable(reference).ok())
            .collect::<Vec<_>>();
        Ok(VmDebugPage {
            values,
            next_cursor: (consumed < references.len()).then_some(consumed),
        })
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

fn instruction_index(
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
