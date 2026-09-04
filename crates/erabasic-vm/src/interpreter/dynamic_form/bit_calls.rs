//! `BIT` reuses `RuntimeFormTask` evaluation; only its first token is captured early.
use super::call_plan::{RuntimeBoundCall, RuntimeCallSite};
use super::{
    BytecodeType, Deserialize, Expr, Fiber, RuntimeFormContinuation, RuntimeFormTask, Serialize,
    StepError, Vm, VmFaultCode, VmValue, map_vm_error, resource_limit,
};
use crate::state::array_leases::{ArrayLeaseId, ArrayLeaseOrigin, ArrayLeaseOwner};
use crate::state::bit_calls::{BitProgress, BitWork};
use erabasic_bytecode::{BitCallSpec, BitOperation, BytecodeGlobal, RuntimeExpressionShape};
fn invalid(message: &str) -> StepError {
    StepError::new(VmFaultCode::InvalidInstruction, message)
}
fn argument(message: &str) -> StepError {
    StepError::script(
        crate::ScriptFaultKind::Argument,
        VmFaultCode::TypeMismatch,
        message,
    )
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct FormBitCall {
    pub(super) slot: u64,
    pub(super) spec: BitCallSpec,
    pub(super) site: RuntimeCallSite,
    pub(super) source: Vec<Option<Expr>>,
    pub(super) lease: ArrayLeaseId,
    pub(super) value_depth: usize,
    pub(super) work: Option<BitWork>,
}

/// The `TypeAnalysis` visitor has already visited every argument exactly once.
/// Consume its shapes and resolved variable definition; never recursively infer.
pub(super) fn validate_shapes(
    operation: BitOperation,
    definition: Option<&BytecodeGlobal>,
    shapes: &[Option<RuntimeExpressionShape>],
) -> Result<BitCallSpec, StepError> {
    let definition = definition.ok_or_else(|| argument("BIT input must be a variable token"))?;
    if !definition.mutable
        || definition.value_type != BytecodeType::Integer
        || definition.dimensions.len() != 1
    {
        return Err(argument(
            "BIT input must be a mutable Integer array of rank one",
        ));
    }
    let mut spec = BitCallSpec {
        operation,
        input: definition.key,
        tail_count: u8::try_from(shapes.len().saturating_sub(1))
            .map_err(|_| argument("BIT arity exceeds limit"))?,
        present: 0,
    };
    if shapes.first().is_none_or(|shape| {
        !shape.is_some_and(|shape| {
            shape.variable && shape.mutable && shape.value_type == BytecodeType::Integer
        })
    }) {
        return Err(argument("BIT input cannot be omitted or passed by value"));
    }
    for (index, shape) in shapes.iter().skip(1).enumerate() {
        if let Some(shape) = shape {
            if shape.value_type != BytecodeType::Integer || index >= 3 {
                return Err(argument("BIT tail must contain Integer values"));
            }
            spec.present |= 1 << index;
        }
    }
    BitCallSpec::decode(&spec.encode())
        .map_err(|_| argument("BIT argument presence or arity differs"))
}

impl RuntimeFormContinuation {
    pub(super) fn schedule_bit(
        &mut self,
        vm: &Vm,
        spec: BitCallSpec,
        source: Vec<Option<Expr>>,
        site: RuntimeCallSite,
    ) -> Result<(), StepError> {
        if !self.valid_bit_capture(vm, spec, site, &source) {
            return Err(invalid("BIT plan binding differs"));
        }
        self.work
            .push(RuntimeFormTask::BitCapture { spec, site, source });
        Ok(())
    }
    pub(super) fn capture_bit(
        &mut self,
        vm: &mut Vm,
        fiber: &Fiber,
        spec: BitCallSpec,
        site: RuntimeCallSite,
        source: Vec<Option<Expr>>,
    ) -> Result<(), StepError> {
        if !self.valid_bit_capture(vm, spec, site, &source) {
            return Err(invalid("BIT capture plan differs"));
        }
        let slot = self.next_bit_call;
        self.next_bit_call = slot
            .checked_add(1)
            .ok_or_else(|| resource_limit("BIT form lease identities exhausted"))?;
        let mut live = crate::interpreter::bit_calls::live_bit_leases(
            vm.fibers.values().chain(std::iter::once(fiber)),
        );
        live.extend(self.bit_leases());
        live.extend(self.reference_leases());
        vm.memory.array_leases.retain(&live);
        let token = crate::PlaceDescriptor {
            variable: spec.input,
            fiber: Some(fiber.id),
            frame: Some(self.frame),
            ..Default::default()
        };
        let lease = vm
            .capture_bit_array(
                fiber,
                &token,
                ArrayLeaseOrigin::RuntimeForm {
                    instruction: self.instruction,
                    slot,
                },
            )
            .map_err(map_vm_error)?;
        self.work.push(RuntimeFormTask::BitFinish(FormBitCall {
            slot,
            spec,
            site,
            source: source.clone(),
            lease,
            value_depth: self.values.len(),
            work: None,
        }));
        self.work.extend(
            source
                .into_iter()
                .skip(1)
                .rev()
                .flatten()
                .map(RuntimeFormTask::Evaluate),
        );
        Ok(())
    }
    pub(super) fn finish_bit(
        &mut self,
        vm: &mut Vm,
        fiber: &mut Fiber,
        mut call: FormBitCall,
    ) -> Result<(), StepError> {
        if self.values.len() != call.value_depth + call.spec.evaluated_arguments() {
            return Err(invalid("BIT form operand depth differs"));
        }
        let values = self.values[call.value_depth..]
            .iter()
            .map(|value| match value {
                VmValue::Integer(value) => Ok(*value),
                _ => Err(invalid("BIT form tail is not Integer")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let length = vm
            .memory
            .array_leases
            .entries
            .get(&call.lease)
            .ok_or_else(|| invalid("BIT form lease missing"))?
            .length;
        let mut work = call.work.take().map_or_else(
            || BitWork::new(call.spec, &values, length).map_err(map_vm_error),
            Ok,
        )?;
        if !work.valid_for(call.spec, &values, length) {
            return Err(invalid("BIT form work range differs"));
        }
        // One word per existing form task preserves the scheduler's instruction
        // budget exactly; bytecode dispatch can coalesce up to256 such words.
        match work
            .advance(vm, fiber, call.lease, 1)
            .map_err(map_vm_error)?
            .0
        {
            BitProgress::Complete(value) => {
                self.values.truncate(call.value_depth);
                self.values.push(VmValue::Integer(value));
                vm.memory.array_leases.release(call.lease);
            }
            BitProgress::Continue => {
                call.work = Some(work);
                self.work.push(RuntimeFormTask::BitFinish(call));
            }
        }
        Ok(())
    }
    pub(crate) fn bit_leases(&self) -> impl Iterator<Item = ArrayLeaseId> + '_ {
        self.work.iter().filter_map(|task| match task {
            RuntimeFormTask::BitFinish(call) => Some(call.lease),
            _ => None,
        })
    }
    pub(crate) fn bit_owners(
        &self,
        fiber: crate::FiberId,
    ) -> impl Iterator<Item = (ArrayLeaseId, ArrayLeaseOwner)> + '_ {
        self.work.iter().filter_map(move |task| match task {
            RuntimeFormTask::BitFinish(call) => Some((
                call.lease,
                ArrayLeaseOwner {
                    fiber,
                    frame: self.frame,
                    generation: self.generation,
                    function: self.function,
                    origin: ArrayLeaseOrigin::RuntimeForm {
                        instruction: self.instruction,
                        slot: call.slot,
                    },
                },
            )),
            _ => None,
        })
    }
    pub(super) fn valid_bit_capture(
        &self,
        vm: &Vm,
        spec: BitCallSpec,
        site: RuntimeCallSite,
        source: &[Option<Expr>],
    ) -> bool {
        let Some(program) = vm.generations.get(&self.generation) else {
            return false;
        };
        self.lookup_bound_call(site) == Some(&RuntimeBoundCall::Bit(spec))
            && self.validate_call_arguments(program, site, source)
    }
    pub(super) fn valid_bit_task(&self, vm: &Vm, call: &FormBitCall) -> bool {
        if !self.valid_bit_capture(vm, call.spec, call.site, &call.source)
            || call.slot == 0
            || call.slot >= self.next_bit_call
            || call.value_depth > self.values.len()
            || BitCallSpec::decode(&call.spec.encode()) != Ok(call.spec)
        {
            return false;
        }
        let Some(lease) = vm.memory.array_leases.entries.get(&call.lease) else {
            return false;
        };
        let Some(program) = vm.generations.get(&self.generation) else {
            return false;
        };
        if lease.input != call.spec.input
            || !program
                .artifact
                .manifest
                .compatibility
                .supports_snake_data_apis()
            || !program.artifact.globals.iter().any(|input| {
                input.key == call.spec.input
                    && input.mutable
                    && input.value_type == BytecodeType::Integer
                    && input.dimensions.len() == 1
            })
        {
            return false;
        }
        if let Some(work) = &call.work {
            if self.awaiting_user_call.is_some()
                || vm
                    .fibers
                    .get(&lease.owner.fiber)
                    .is_none_or(|fiber| !matches!(fiber.state, crate::FiberState::Runnable))
            {
                return false;
            }
            let values = self.values.get(call.value_depth..).and_then(|values| {
                values
                    .iter()
                    .map(|value| match value {
                        VmValue::Integer(value) => Some(*value),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
            });
            let words = vm
                .memory
                .array_leases
                .entries
                .get(&call.lease)
                .map(|lease| lease.length);
            return values
                .zip(words)
                .is_some_and(|(values, words)| work.has_progress(call.spec, &values, words));
        }
        true
    }
}
