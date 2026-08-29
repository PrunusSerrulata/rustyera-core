//! The opaque operand owns a captured MAP object, including after name release/recreation.
use super::{
    BytecodeStorage, Fiber, HostReady, ImportKind, InstructionPosition, NativeCallRequest,
    NativeServiceRegistry, Opcode, StepError, StepOutcome, SymbolKey, Vm, VmFaultCode, VmValue,
    map_vm_error, native_implicit_place_views, native_place_views, pop, pop_arguments, read_u32,
    validate_native_ready,
};
use crate::structured::{MapLease, MapLeaseOrigin, MapLeaseOwner};
use erabasic_bytecode::{MapCallKind, RuntimeImport};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct PendingMapCall {
    pub begin: usize,
    pub stack_index: usize,
    pub name: String,
    pub lease: Option<MapLease>,
}
pub(in crate::interpreter) struct CapturedMapCall {
    pub kind: MapCallKind,
    pub lease: MapLease,
    pub service_key: SymbolKey,
    pub import: RuntimeImport,
    pub arguments: Vec<VmValue>,
}
fn invalid(message: &str) -> StepError {
    StepError::new(VmFaultCode::InvalidInstruction, message)
}
pub(super) fn map_missing(kind: MapCallKind) -> VmValue {
    VmValue::default_for(kind.result_type())
}
pub(crate) fn live_map_leases<'a>(fibers: impl Iterator<Item = &'a Fiber>) -> BTreeSet<MapLease> {
    fibers
        .flat_map(|fiber| fiber.frames.iter())
        .flat_map(|frame| {
            frame.map_calls.iter().filter_map(|call| call.lease).chain(
                frame
                    .runtime_form
                    .iter()
                    .flat_map(super::dynamic_form::RuntimeFormContinuation::map_leases),
            )
        })
        .collect()
}
impl Vm {
    pub(in crate::interpreter) fn map_import(
        &self,
        generation: crate::GenerationId,
        function: SymbolKey,
        begin: usize,
    ) -> Result<(MapCallKind, RuntimeImport), StepError> {
        let program = self
            .generations
            .get(&generation)
            .ok_or_else(|| invalid("MAP generation is missing"))?;
        if !program
            .artifact
            .manifest
            .compatibility
            .supports_map_extensions()
        {
            return Err(invalid("MAP extension is unavailable in this identity"));
        }
        let function = program
            .function(function)
            .ok_or_else(|| invalid("MAP function is missing"))?;
        let opening = function
            .code
            .get(begin)
            .filter(|op| Opcode::try_from(op.opcode) == Ok(Opcode::BeginMapCall))
            .ok_or_else(|| invalid("MAP opener is missing"))?;
        let import = function
            .imports
            .get(read_u32(&opening.payload, 0)? as usize)
            .filter(|import| import.kind == ImportKind::Native)
            .ok_or_else(|| invalid("MAP Native import is missing"))?;
        let native = program
            .artifact
            .native_imports
            .iter()
            .find(|native| native.import.key == import.key)
            .ok_or_else(|| invalid("MAP Native definition is missing"))?;
        let kind = MapCallKind::from_name(&native.import.name)
            .ok_or_else(|| invalid("MAP opener uses a non-staged Native"))?;
        if native.import.namespace != "rustyera.vm"
            || !kind.valid_parameters(&native.import.parameters)
            || native.import.result != Some(kind.result_type())
        {
            return Err(invalid("MAP Native signature differs"));
        }
        Ok((kind, native.import.clone()))
    }
    #[allow(clippy::too_many_lines)] // Capture, abandon, and finish are one atomic opcode family.
    pub(in crate::interpreter) fn dispatch_map_calls(
        &mut self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        opcode: Opcode,
        natives: &mut NativeServiceRegistry,
    ) -> Result<Option<StepOutcome>, StepError> {
        if !matches!(
            opcode,
            Opcode::BeginMapCall | Opcode::FinishMapCall | Opcode::AbandonMapCall
        ) {
            return Ok(None);
        }
        self.invalidate_path_memo(fiber.id);
        if opcode == Opcode::BeginMapCall {
            let (_, import) =
                self.map_import(position.generation, position.function, position.instruction)?;
            if !natives.staged_map_provider(import.key) {
                return Err(StepError::classified(
                    crate::FaultCategory::HostContract,
                    VmFaultCode::Native,
                    "MAP provider is not registered",
                ));
            }
            natives.retain_map_leases(&live_map_leases(
                self.fibers.values().chain(std::iter::once(&*fiber)),
            ))?;
            let owner = fiber
                .frames
                .last_mut()
                .ok_or_else(|| invalid("MAP owner is missing"))?;
            if owner
                .operand_slots()
                .and_then(|n| n.checked_add(2))
                .is_none_or(|n| n > self.config.maximum_operand_stack)
            {
                return Err(StepError::new(
                    VmFaultCode::ResourceLimit,
                    "MAP capture exceeds operand limit",
                ));
            }
            let VmValue::String(name) = pop(&mut owner.stack)? else {
                return Err(invalid("MAP name is not String"));
            };
            let lease = natives.capture_map(
                &name,
                MapLeaseOwner {
                    fiber: fiber.id,
                    frame: owner.id,
                    generation: owner.generation,
                    function: owner.function,
                    origin: MapLeaseOrigin::Bytecode {
                        begin: position.instruction,
                    },
                },
            )?;
            let pending = PendingMapCall {
                begin: position.instruction,
                stack_index: owner.stack.len(),
                name,
                lease,
            };
            let token = i64::try_from(position.instruction)
                .map_err(|_| invalid("MAP capture offset exceeds Integer"))?;
            owner.stack.push(VmValue::Integer(token));
            owner
                .stack
                .push(VmValue::Integer(i64::from(lease.is_some())));
            owner.map_calls.push(pending);
        } else {
            let begin = read_u32(position.encoded.payload, 0)? as usize;
            let (kind, import) = self.map_import(position.generation, position.function, begin)?;
            let owner = fiber
                .frames
                .last_mut()
                .ok_or_else(|| invalid("MAP owner is missing"))?;
            let pending = owner
                .map_calls
                .last()
                .ok_or_else(|| invalid("MAP capture is missing"))?;
            let count = if opcode == Opcode::FinishMapCall {
                import.parameters.len() - 1
            } else {
                0
            };
            let begin_token = i64::from(
                u32::try_from(begin)
                    .map_err(|_| invalid("MAP capture offset exceeds the bytecode format"))?,
            );
            if pending.begin != begin
                || owner.stack.len() != pending.stack_index + count + 1
                || owner.stack.get(pending.stack_index) != Some(&VmValue::Integer(begin_token))
                || pending.lease.is_some_and(|lease| {
                    lease.owner
                        != MapLeaseOwner {
                            fiber: fiber.id,
                            frame: owner.id,
                            generation: owner.generation,
                            function: owner.function,
                            origin: MapLeaseOrigin::Bytecode { begin },
                        }
                })
            {
                return Err(invalid("MAP capture owner, origin or stack differs"));
            }
            let mut arguments = pop_arguments(&mut owner.stack, count)?;
            let pending = owner.map_calls.pop().expect("checked capture");
            owner.stack.pop();
            if opcode == Opcode::AbandonMapCall {
                if let Some(lease) = pending.lease {
                    natives.release_map(lease)?;
                }
                owner.stack.push(map_missing(kind));
            } else {
                let lease = pending
                    .lease
                    .ok_or_else(|| invalid("missing MAP cannot enter completion"))?;
                if kind == MapCallKind::Values && import.parameters.len() == 3 {
                    arguments.swap(0, 1);
                }
                arguments.insert(0, VmValue::String(pending.name));
                self.finish_captured_map(
                    fiber,
                    natives,
                    CapturedMapCall {
                        kind,
                        lease,
                        service_key: import.key,
                        import,
                        arguments,
                    },
                )?;
            }
        }
        Ok(Some(StepOutcome::Continue))
    }
    pub(in crate::interpreter) fn finish_captured_map(
        &mut self,
        fiber: &mut Fiber,
        natives: &mut NativeServiceRegistry,
        call: CapturedMapCall,
    ) -> Result<(), StepError> {
        let CapturedMapCall {
            kind,
            lease,
            service_key,
            import,
            arguments,
        } = call;
        let result = (|| {
            if !natives.staged_map_provider(service_key) {
                return Err(StepError::classified(
                    crate::FaultCategory::HostContract,
                    VmFaultCode::Native,
                    "MAP provider is not registered",
                ));
            }
            if kind == MapCallKind::Values && arguments.len() == 3 {
                let Some(VmValue::StringPlace(place)) = arguments.get(1) else {
                    return Err(invalid("MAP_VALUES output place missing"));
                };
                let (_, definition) = self.place_definition(fiber, place).map_err(map_vm_error)?;
                if definition.storage == BytecodeStorage::Character {
                    return Err(StepError::script(
                        crate::ScriptFaultKind::Operation,
                        VmFaultCode::Native,
                        "MAP_VALUES GetArray cannot access a direct character array",
                    ));
                }
            }
            let places = native_place_views(self, fiber, &arguments).map_err(map_vm_error)?;
            let implicit_places =
                native_implicit_place_views(self, fiber, kind.implicit_places(arguments.len()))
                    .map_err(map_vm_error)?;
            // One cumulative budget across every key/entry, shared by bytecode and
            // RuntimeForm through this existing completion entry point.
            let mut text_budget = crate::compat_text::TextBudget::new(
                self.config.maximum_operand_stack,
                self.config.maximum_snapshot_bytes,
            );
            let ready = natives.apply_map(
                kind,
                lease,
                &NativeCallRequest {
                    service_key,
                    omitted_arguments: Vec::new(),
                    import,
                    arguments,
                    places,
                    implicit_places,
                },
                &mut text_budget,
            )?;
            // MAP mutation operations return no writes; operations producing writes only read MAP.
            // Thus a rejected write set cannot leave a partially modified MAP behind.
            validate_native_ready(self, fiber, Some(kind.result_type()), &ready)
                .and_then(|()| {
                    self.apply_host_ready(
                        fiber,
                        Some(kind.result_type()),
                        HostReady {
                            value: ready.value,
                            writes: ready.writes,
                        },
                    )
                })
                .map_err(|error| {
                    StepError::classified(
                        crate::FaultCategory::HostContract,
                        VmFaultCode::Native,
                        error.to_string(),
                    )
                })
        })();
        natives.release_map(lease)?;
        result
    }
}

pub(crate) fn valid_map_calls(vm: &Vm, fiber: &Fiber, frame: &crate::state::Frame) -> bool {
    frame.map_calls.iter().all(|call| {
        let token_matches = i64::try_from(call.begin).ok().is_some_and(|begin| {
            frame.stack.get(call.stack_index) == Some(&VmValue::Integer(begin))
        });
        vm.map_import(frame.generation, frame.function, call.begin)
            .is_ok()
            && token_matches
            && call.lease.is_none_or(|lease| {
                lease.owner
                    == MapLeaseOwner {
                        fiber: fiber.id,
                        frame: frame.id,
                        generation: frame.generation,
                        function: frame.function,
                        origin: MapLeaseOrigin::Bytecode { begin: call.begin },
                    }
            })
    })
}
