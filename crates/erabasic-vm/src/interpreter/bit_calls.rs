//! Capture once before tail evaluation; bounded work retains the opaque opener.
use super::{ExecutionPolicy, InstructionPosition, StepError, StepOutcome, map_vm_error, read_u32};
use crate::state::array_leases::{ArrayLeaseId, ArrayLeaseOrigin, ArrayLeaseOwner};
use crate::state::bit_calls::{BitProgress, BitWork, PendingBitCall};
use crate::{Fiber, PlaceDescriptor, Vm, VmError, VmFaultCode, VmValue};
use erabasic_bytecode::{
    BitCallSpec, BytecodeType, Opcode, RuntimeExpressionShape, RuntimeStagedKind, SymbolKey,
};
use std::collections::{BTreeMap, BTreeSet};
fn invalid(message: &str) -> StepError {
    StepError::new(VmFaultCode::InvalidInstruction, message)
}

pub(crate) fn live_bit_leases<'a>(
    fibers: impl Iterator<Item = &'a Fiber>,
) -> BTreeSet<ArrayLeaseId> {
    let mut roots = BTreeSet::new();
    for fiber in fibers {
        roots.extend(crate::state::references::reference_leases(fiber));
        roots.extend(fiber.frames.iter().flat_map(|frame| {
            frame.bit_calls.iter().map(|call| call.lease).chain(
                frame
                    .runtime_form
                    .iter()
                    .flat_map(super::dynamic_form::RuntimeFormContinuation::bit_leases),
            )
        }));
    }
    roots
}
impl Vm {
    pub(crate) fn bit_spec(
        &self,
        generation: crate::GenerationId,
        function: SymbolKey,
        begin: usize,
    ) -> Result<BitCallSpec, StepError> {
        let program = self
            .generations
            .get(&generation)
            .ok_or_else(|| invalid("BIT generation is missing"))?;
        if !program
            .artifact
            .manifest
            .compatibility
            .supports_snake_data_apis()
        {
            return Err(StepError::classified(
                crate::FaultCategory::Permission,
                VmFaultCode::InvalidInstruction,
                "BIT operations are unavailable in this compatibility identity",
            ));
        }
        let opening = program
            .function(function)
            .and_then(|function| function.code.get(begin))
            .filter(|instruction| Opcode::try_from(instruction.opcode) == Ok(Opcode::BeginBitCall))
            .ok_or_else(|| invalid("BIT capture instruction is missing"))?;
        let spec = BitCallSpec::decode(&opening.payload).map_err(|message| invalid(&message))?;
        let input = program
            .global(spec.input)
            .ok_or_else(|| invalid("BIT input schema is missing"))?;
        let mut shapes = Vec::with_capacity(usize::from(spec.tail_count) + 1);
        shapes.push(Some(RuntimeExpressionShape {
            value_type: input.value_type,
            variable: true,
            mutable: input.mutable,
        }));
        for index in 0..spec.tail_count {
            shapes.push(
                (spec.present & (1 << index) != 0).then_some(RuntimeExpressionShape {
                    value_type: BytecodeType::Integer,
                    variable: false,
                    mutable: false,
                }),
            );
        }
        let name = match spec.operation {
            erabasic_bytecode::BitOperation::Set => "BITSET",
            erabasic_bytecode::BitOperation::Get => "BITGET",
            erabasic_bytecode::BitOperation::Toggle => "BITTOGGLE",
            erabasic_bytecode::BitOperation::IndexOfFirst => "BITINDEXOFFIRST",
        };
        if !program
            .artifact
            .runtime_staged_authorizations
            .iter()
            .any(|family| {
                family.name.eq_ignore_ascii_case(name)
                    && family.kind == RuntimeStagedKind::Bit(spec.operation)
                    && family.accepts(&shapes)
            })
        {
            return Err(StepError::classified(
                crate::FaultCategory::Permission,
                VmFaultCode::InvalidInstruction,
                format!("BIT operation {name} lacks matching staged authorization"),
            ));
        }
        Ok(spec)
    }
    #[allow(clippy::too_many_lines)] // One opcode family shares a single resumable state transition.
    pub(in crate::interpreter) fn dispatch_bit_calls(
        &mut self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        opcode: Opcode,
        policy: ExecutionPolicy,
    ) -> Result<Option<StepOutcome>, StepError> {
        if !matches!(opcode, Opcode::BeginBitCall | Opcode::FinishBitCall) {
            return Ok(None);
        }
        self.invalidate_path_memo(fiber.id);
        if opcode == Opcode::BeginBitCall {
            let spec =
                self.bit_spec(position.generation, position.function, position.instruction)?;
            let owner = fiber
                .frames
                .last()
                .ok_or_else(|| invalid("BIT owner is missing"))?;
            if owner
                .operand_slots()
                .and_then(|n| n.checked_add(2))
                .is_none_or(|n| n > self.config.maximum_operand_stack)
            {
                return Err(StepError::new(
                    VmFaultCode::ResourceLimit,
                    "BIT capture exceeds operand limit",
                ));
            }
            let token = PlaceDescriptor {
                backing: None,
                variable: spec.input,
                indices: Vec::new(),
                fiber: Some(fiber.id),
                frame: Some(owner.id),
                character: None,
            };
            self.memory.array_leases.retain(&live_bit_leases(
                self.fibers.values().chain(std::iter::once(&*fiber)),
            ));
            let lease = self
                .capture_bit_array(
                    fiber,
                    &token,
                    ArrayLeaseOrigin::Bytecode {
                        begin: position.instruction,
                    },
                )
                .map_err(map_vm_error)?;
            let owner = fiber
                .frames
                .last_mut()
                .ok_or_else(|| invalid("BIT owner disappeared"))?;
            owner.bit_calls.push(PendingBitCall {
                begin: position.instruction,
                stack_index: owner.stack.len(),
                spec,
                lease,
                work: None,
            });
            let token = i64::try_from(position.instruction)
                .map_err(|_| invalid("BIT capture offset exceeds Integer"))?;
            owner.stack.push(VmValue::Integer(token));
            return Ok(Some(StepOutcome::Continue));
        }
        let begin = read_u32(position.encoded.payload, 0)? as usize;
        let spec = self.bit_spec(position.generation, position.function, begin)?;
        let owner = fiber
            .frames
            .last()
            .ok_or_else(|| invalid("BIT owner is missing"))?;
        let pending = owner
            .bit_calls
            .last()
            .ok_or_else(|| invalid("BIT pending capture is missing"))?;
        let begin_token = i64::from(
            u32::try_from(begin)
                .map_err(|_| invalid("BIT capture offset exceeds the bytecode format"))?,
        );
        if pending.begin != begin
            || pending.spec != spec
            || owner.stack.len() != pending.stack_index + 1 + spec.evaluated_arguments()
            || owner.stack.get(pending.stack_index) != Some(&VmValue::Integer(begin_token))
        {
            return Err(invalid("BIT completion stack or capture differs"));
        }
        let values = owner.stack[pending.stack_index + 1..]
            .iter()
            .map(|value| match value {
                VmValue::Integer(value) => Ok(*value),
                _ => Err(invalid("BIT scalar operand is not Integer")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let lease = pending.lease;
        let length = self
            .memory
            .array_leases
            .entries
            .get(&lease)
            .ok_or_else(|| invalid("BIT backing lease is missing"))?
            .length;
        let mut work = pending.work.clone().map_or_else(
            || BitWork::new(spec, &values, length).map_err(map_vm_error),
            Ok,
        )?;
        if !work.valid_for(spec, &values, length) {
            return Err(invalid("BIT work progress differs from operands"));
        }
        let limit = policy
            .remaining_instructions
            .min(u64::from(policy.remaining_quantum))
            .clamp(1, 256) as usize;
        let (progress, visited) = work
            .advance(self, fiber, lease, limit)
            .map_err(map_vm_error)?;
        let owner = fiber
            .frames
            .last_mut()
            .ok_or_else(|| invalid("BIT owner disappeared"))?;
        match progress {
            BitProgress::Complete(value) => {
                let pending = owner.bit_calls.pop().expect("validated capture");
                owner.stack.truncate(pending.stack_index);
                owner.stack.push(VmValue::Integer(value));
                self.memory.array_leases.release(lease);
            }
            BitProgress::Continue => {
                owner.bit_calls.last_mut().expect("validated capture").work = Some(work);
                owner.instruction = position.instruction;
            }
        }
        let visited = u64::try_from(visited)
            .map_err(|_| invalid("BIT progress exceeds the scheduler counter"))?;
        Ok(Some(StepOutcome::BulkProgress(visited.saturating_sub(1))))
    }
    pub(crate) fn prune_bit_leases(&mut self) {
        self.memory
            .array_leases
            .retain(&live_bit_leases(self.fibers.values()));
    }
    pub(crate) fn validate_bit_leases(&self) -> Result<(), VmError> {
        let mut expected = BTreeMap::new();
        for fiber in self.fibers.values() {
            for frame in &fiber.frames {
                for call in &frame.bit_calls {
                    let spec = self
                        .bit_spec(frame.generation, frame.function, call.begin)
                        .map_err(|error| VmError::Snapshot(error.to_string()))?;
                    let Ok(begin_token) = i64::try_from(call.begin) else {
                        return Err(VmError::Snapshot(
                            "BIT capture offset exceeds Integer".into(),
                        ));
                    };
                    if spec != call.spec
                        || self
                            .memory
                            .array_leases
                            .entries
                            .get(&call.lease)
                            .is_none_or(|lease| lease.input != spec.input)
                        || frame.stack.get(call.stack_index) != Some(&VmValue::Integer(begin_token))
                    {
                        return Err(VmError::Snapshot("BIT capture spec/token differs".into()));
                    }
                    let owner = ArrayLeaseOwner {
                        fiber: fiber.id,
                        frame: frame.id,
                        generation: frame.generation,
                        function: frame.function,
                        origin: ArrayLeaseOrigin::Bytecode { begin: call.begin },
                    };
                    if expected.insert(call.lease, owner).is_some() {
                        return Err(VmError::Snapshot("BIT lease is owned twice".into()));
                    }
                    if let Some(work) = &call.work {
                        if !matches!(fiber.state, crate::FiberState::Runnable) {
                            return Err(VmError::Snapshot(
                                "BIT partial work cannot own a Host wait or terminal state".into(),
                            ));
                        }
                        let instruction = self
                            .generations
                            .get(&frame.generation)
                            .and_then(|program| program.function(frame.function))
                            .and_then(|function| function.code.get(frame.instruction));
                        let begin_payload = u32::try_from(call.begin).map_err(|_| {
                            VmError::Snapshot(
                                "BIT capture offset exceeds the bytecode format".into(),
                            )
                        })?;
                        if !instruction.is_some_and(|op| {
                            Opcode::try_from(op.opcode) == Ok(Opcode::FinishBitCall)
                                && op.payload.as_slice() == begin_payload.to_le_bytes()
                        }) {
                            return Err(VmError::Snapshot(
                                "BIT active work is not at its finish instruction".into(),
                            ));
                        }
                        let values = frame
                            .stack
                            .get(call.stack_index + 1..)
                            .ok_or_else(|| VmError::Snapshot("BIT operands missing".into()))?
                            .iter()
                            .map(|value| match value {
                                VmValue::Integer(value) => Some(*value),
                                _ => None,
                            })
                            .collect::<Option<Vec<_>>>();
                        let length = self
                            .memory
                            .array_leases
                            .entries
                            .get(&call.lease)
                            .map(|entry| entry.length);
                        if !values.zip(length).is_some_and(|(values, length)| {
                            work.has_progress(spec, &values, length)
                        }) {
                            return Err(VmError::Snapshot(
                                "BIT work range or progress is invalid".into(),
                            ));
                        }
                    }
                }
                if let Some(form) = &frame.runtime_form {
                    for (lease, owner) in form.bit_owners(fiber.id) {
                        if expected.insert(lease, owner).is_some() {
                            return Err(VmError::Snapshot("BIT form lease is owned twice".into()));
                        }
                    }
                }
            }
        }
        self.collect_reference_owners(&mut expected)?;
        self.validate_reference_lease_sources()?;
        self.validate_bit_lease_symbols()?;
        self.memory.validate_array_leases(
            &self.fibers,
            &expected,
            self.config.maximum_operand_stack,
        )
    }
}
