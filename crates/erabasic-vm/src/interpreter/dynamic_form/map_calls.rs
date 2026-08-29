//! MAP extends the existing runtime-form continuation; no separate expression evaluator.
use super::call_plan::{RuntimeBoundCall, RuntimeCallSite};
use super::{
    BytecodeType, Deserialize, Expr, ExprKind, Fiber, NativeServiceRegistry,
    RuntimeFormContinuation, RuntimeFormTask, Serialize, StepError, SymbolKey, Vm, VmFaultCode,
    VmValue, owner_frame, owner_frame_mut, resource_limit,
};
use crate::interpreter::map_calls::{CapturedMapCall, live_map_leases, map_missing};
use crate::structured::{MapLease, MapLeaseOrigin, MapLeaseOwner};
use erabasic_bytecode::{BoundRuntimeNative, BytecodeStorage, MapCallKind};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct MapWorkCall {
    slot: u64,
    lease: MapLease,
    pub(super) name: String,
    kind: MapCallKind,
    pub(super) bound: BoundRuntimeNative,
    pub(super) site: RuntimeCallSite,
    pub(super) source: Vec<Option<Expr>>,
    arity: usize,
    value_depth: usize,
}
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
/// Consumes an already resolved declaration; never recursively types arguments.
/// Root's post-2B `TypeAnalysis::call` can invoke this with its existing definition.
pub(super) fn validate_map_output_definition(
    kind: MapCallKind,
    arity: usize,
    definition: Option<&erabasic_bytecode::BytecodeGlobal>,
) -> Result<(), StepError> {
    if kind == MapCallKind::Values
        && arity == 3
        && !definition.is_some_and(|variable| {
            variable.value_type == BytecodeType::String
                && variable.mutable
                && variable.dimensions.len() == 1
        })
    {
        return Err(argument(
            "MAP_VALUES output must be a mutable String array of rank one",
        ));
    }
    Ok(())
}
pub(super) fn output_variable<'a>(
    program: &'a crate::ProgramGeneration,
    function: SymbolKey,
    expression: &Expr,
) -> Result<&'a erabasic_bytecode::BytecodeGlobal, StepError> {
    match &expression.kind {
        ExprKind::Group(inner) => output_variable(program, function, inner),
        ExprKind::Identifier(name) | ExprKind::Variable { name, .. } => program
            .scoped_variable(function, name)
            .filter(|variable| {
                variable.value_type == BytecodeType::String
                    && variable.mutable
                    && variable.dimensions.len() == 1
            })
            .ok_or_else(|| {
                argument("MAP_VALUES output must be a mutable String array of rank one")
            }),
        _ => Err(argument("MAP_VALUES output must be a variable token")),
    }
}
fn argument_error() -> StepError {
    argument("MAP arguments cannot be omitted")
}
impl RuntimeFormContinuation {
    pub(super) fn schedule_map(
        &mut self,
        vm: &Vm,
        bound: &BoundRuntimeNative,
        arguments: Vec<Option<Expr>>,
        site: RuntimeCallSite,
    ) -> Result<(), StepError> {
        let program = vm
            .generations
            .get(&self.generation)
            .ok_or_else(|| invalid("MAP form generation missing"))?;
        if !self.valid_map_binding(program, bound, site, &arguments) {
            return Err(invalid("MAP plan binding differs"));
        }
        let first = arguments[0].clone().ok_or_else(argument_error)?;
        self.work.push(RuntimeFormTask::MapCapture {
            bound: bound.clone(),
            site,
            arguments,
        });
        self.work.push(RuntimeFormTask::Evaluate(first));
        Ok(())
    }
    pub(super) fn capture_map(
        &mut self,
        vm: &Vm,
        fiber: &Fiber,
        natives: &NativeServiceRegistry,
        bound: BoundRuntimeNative,
        site: RuntimeCallSite,
        arguments: Vec<Option<Expr>>,
    ) -> Result<(), StepError> {
        let VmValue::String(name) = self.pop_value("MAP name missing")? else {
            return Err(invalid("MAP name is not String"));
        };
        let program = vm
            .generations
            .get(&self.generation)
            .ok_or_else(|| invalid("MAP form generation missing"))?;
        if !self.valid_map_binding(program, &bound, site, &arguments) {
            return Err(invalid("MAP capture plan differs"));
        }
        let kind = MapCallKind::from_name(&bound.import.name)
            .ok_or_else(|| invalid("MAP kind missing"))?;
        if !natives.staged_map_provider(bound.service_key) {
            return Err(StepError::classified(
                crate::FaultCategory::HostContract,
                VmFaultCode::Native,
                "MAP provider is not registered",
            ));
        }
        let slot = self.next_map_call;
        self.next_map_call = slot
            .checked_add(1)
            .ok_or_else(|| resource_limit("MAP form capture identity exhausted"))?;
        let mut live = live_map_leases(vm.fibers.values().chain(std::iter::once(fiber)));
        live.extend(self.map_leases()); // This continuation is temporarily taken out of its frame while stepping.
        natives.retain_map_leases(&live)?;
        let lease = natives.capture_map(
            &name,
            MapLeaseOwner {
                fiber: fiber.id,
                frame: self.frame,
                generation: self.generation,
                function: self.function,
                origin: MapLeaseOrigin::RuntimeForm {
                    instruction: self.instruction,
                    slot,
                },
            },
        )?;
        let Some(lease) = lease else {
            self.values.push(map_missing(kind));
            return Ok(());
        };
        let call = MapWorkCall {
            slot,
            lease,
            name,
            kind,
            bound,
            site,
            source: arguments.clone(),
            arity: arguments.len(),
            value_depth: self.values.len(),
        };
        if kind == MapCallKind::Values && arguments.len() > 1 {
            let enabled = arguments[arguments.len() - 1]
                .clone()
                .ok_or_else(argument_error)?;
            let output = if arguments.len() == 3 {
                arguments[1].clone()
            } else {
                None
            };
            self.work
                .push(RuntimeFormTask::MapValuesEnabled { call, output });
            self.work.push(RuntimeFormTask::Evaluate(enabled));
        } else {
            self.work.push(RuntimeFormTask::MapFinish(call));
            self.work
                .extend(arguments.into_iter().skip(1).rev().map(|argument| {
                    RuntimeFormTask::Evaluate(argument.expect("signature checked omission"))
                }));
        }
        Ok(())
    }
    pub(super) fn map_values_enabled(
        &mut self,
        vm: &Vm,
        fiber: &Fiber,
        natives: &NativeServiceRegistry,
        call: MapWorkCall,
        output: Option<Expr>,
    ) -> Result<(), StepError> {
        let result = (|| {
            let enabled = self.pop_integer("MAP_VALUES enabled value is missing")?;
            if enabled == 0 {
                self.values.push(VmValue::String(String::new()));
                return Ok(false);
            }
            self.values.push(VmValue::Integer(enabled));
            if let Some(output) = output {
                let program = vm
                    .generations
                    .get(&self.generation)
                    .ok_or_else(|| invalid("MAP form generation missing"))?;
                let variable = output_variable(program, self.function, &output)?;
                let place = crate::PlaceDescriptor {
                    backing: None,
                    variable: variable.key,
                    indices: Vec::new(),
                    fiber: Some(fiber.id),
                    frame: (variable.storage == BytecodeStorage::FunctionLocal)
                        .then_some(self.frame),
                    character: (variable.storage == BytecodeStorage::Character)
                        .then(|| vm.target_character_for_generation(self.generation) as u64),
                };
                self.values.push(VmValue::StringPlace(Box::new(place)));
            }
            Ok(true)
        })();
        match result {
            Ok(true) => {
                self.work.push(RuntimeFormTask::MapFinish(call));
                Ok(())
            }
            Ok(false) => natives.release_map(call.lease),
            Err(error) => {
                natives.release_map(call.lease)?;
                Err(error)
            }
        }
    }
    pub(super) fn finish_map(
        &mut self,
        vm: &mut Vm,
        fiber: &mut Fiber,
        natives: &mut NativeServiceRegistry,
        call: MapWorkCall,
    ) -> Result<(), StepError> {
        let MapWorkCall {
            slot: _,
            lease,
            name,
            kind,
            bound,
            site,
            source,
            arity,
            value_depth,
        } = call;
        let preparation = (|| {
            if self.values.len() != value_depth + arity - 1 {
                return Err(invalid("MAP form temporary value depth differs"));
            }
            let program = vm
                .generations
                .get(&self.generation)
                .ok_or_else(|| invalid("MAP form generation missing"))?;
            if !self.valid_map_binding(program, &bound, site, &source) {
                return Err(invalid("MAP completion plan differs"));
            }
            let import = bound.import.clone();
            let mut arguments = self.take_values(arity - 1)?;
            if kind == MapCallKind::Values && arity == 3 {
                arguments.swap(0, 1);
            }
            arguments.insert(0, VmValue::String(name));
            Ok((import, arguments))
        })();
        let (import, arguments) = match preparation {
            Ok(value) => value,
            Err(error) => {
                natives.release_map(lease)?;
                return Err(error);
            }
        };
        let depth = owner_frame(fiber, self.frame)?.stack.len();
        vm.finish_captured_map(
            fiber,
            natives,
            CapturedMapCall {
                kind,
                lease,
                service_key: bound.service_key,
                import,
                arguments,
            },
        )?;
        let owner = owner_frame_mut(fiber, self.frame)?;
        if owner.stack.len() != depth + 1 {
            return Err(invalid("MAP form completion has unexpected stack effect"));
        }
        self.values
            .push(owner.stack.pop().expect("one result checked"));
        Ok(())
    }
    pub(crate) fn map_leases(&self) -> impl Iterator<Item = MapLease> + '_ {
        self.work.iter().filter_map(|task| match task {
            RuntimeFormTask::MapFinish(call) | RuntimeFormTask::MapValuesEnabled { call, .. } => {
                Some(call.lease)
            }
            _ => None,
        })
    }
    pub(super) fn valid_map_task(&self, vm: &Vm, fiber: &Fiber, call: &MapWorkCall) -> bool {
        let Some(program) = vm.generations.get(&self.generation) else {
            return false;
        };
        let expected = MapLeaseOwner {
            fiber: fiber.id,
            frame: self.frame,
            generation: self.generation,
            function: self.function,
            origin: MapLeaseOrigin::RuntimeForm {
                instruction: self.instruction,
                slot: call.slot,
            },
        };
        call.slot > 0
            && call.slot < self.next_map_call
            && call.lease.owner == expected
            && call.value_depth <= self.values.len()
            && program
                .artifact
                .manifest
                .compatibility
                .supports_map_extensions()
            && MapCallKind::from_name(&call.bound.import.name) == Some(call.kind)
            && call.arity == call.bound.import.parameters.len()
            && call.arity == call.source.len()
            && self.valid_map_binding(program, &call.bound, call.site, &call.source)
    }
    pub(super) fn valid_map_output_source(call: &MapWorkCall, output: Option<&Expr>) -> bool {
        call.kind == MapCallKind::Values
            && call.arity > 1
            && output == (call.arity == 3).then(|| call.source[1].as_ref()).flatten()
    }
    pub(super) fn valid_map_binding(
        &self,
        program: &crate::ProgramGeneration,
        bound: &BoundRuntimeNative,
        site: RuntimeCallSite,
        source: &[Option<Expr>],
    ) -> bool {
        program
            .artifact
            .manifest
            .compatibility
            .supports_map_extensions()
            && self.lookup_bound_call(site) == Some(&RuntimeBoundCall::Native(bound.clone()))
            && self.validate_call_arguments(program, site, source)
            && bound.omitted_arguments.is_empty()
            && source.iter().all(Option::is_some)
            && MapCallKind::from_name(&bound.import.name).is_some_and(|kind| {
                kind.valid_parameters(&bound.import.parameters)
                    && bound.import.result == Some(kind.result_type())
            })
    }
}
