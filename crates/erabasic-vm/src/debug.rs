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
                            .and_then(|function| {
                                control::instruction_index(function, entry.code_start)
                            });
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

mod control;
mod inspect;
