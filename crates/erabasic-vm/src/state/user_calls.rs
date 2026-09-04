use erabasic_bytecode::{
    BytecodeConstant, BytecodeFunctionKind, BytecodeStorage, BytecodeType, SymbolKey,
    UserArgumentSpec, UserCallMode, UserCallSpec,
};
use serde::{Deserialize, Serialize};

mod validation;

use super::{Fiber, ProgramGeneration};
use crate::{
    GenerationId, PlaceDescriptor, Vm, VmError, VmValue, bind_persistent_arguments, make_frame,
};

/// A saved return policy must be justified by the suspended caller's actual operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum UserCallOrigin {
    Bytecode { resolve: usize, invoke: usize },
    RuntimeForm,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct UserCallFrame {
    pub caller: crate::FrameId,
    pub mode: UserCallMode,
    pub origin: UserCallOrigin,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ResolvedUserCall {
    pub generation: GenerationId,
    pub function: SymbolKey,
    pub mode: UserCallMode,
    pub bindings: Vec<UserArgumentBinding>,
}

impl ResolvedUserCall {
    pub(crate) fn allows_path_memo_observation(&self) -> bool {
        matches!(
            self.mode,
            UserCallMode::Procedure | UserCallMode::MethodDiscard
        ) && !self
            .bindings
            .iter()
            .any(|binding| matches!(binding, UserArgumentBinding::ArrayReference))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum UserArgumentBinding {
    Default(BytecodeConstant),
    Value { convert_integer_to_string: bool },
    ArrayReference,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct PendingUserCall {
    pub resolve: usize,
    pub stack_index: usize,
    pub next_slot: usize,
    pub captured: Vec<Option<VmValue>>,
    pub call: ResolvedUserCall,
}

fn invalid(message: impl Into<String>) -> VmError {
    VmError::InvalidArguments(message.into())
}

fn script_argument(message: impl Into<String>) -> VmError {
    VmError::ScriptFailure(crate::ExecutionFailure::script(
        crate::ScriptFaultKind::Argument,
        crate::VmFaultCode::TypeMismatch,
        message,
    ))
}

/// Resolve the complete signature without evaluating any actual or fallback expression.
pub(crate) fn resolve_user_call(
    program: &ProgramGeneration,
    generation: GenerationId,
    name: &str,
    spec: &UserCallSpec,
) -> Result<Option<ResolvedUserCall>, VmError> {
    let Some(target) = program.function_by_name(name) else {
        return Ok(None);
    };
    // Events hidden from ordinary calls are absent from dynamic method lookup too.
    if spec.mode.is_method()
        && target.kind == BytecodeFunctionKind::Event
        && !program.artifact.call_compatibility.allow_event_as_normal
    {
        return Ok(None);
    }
    if spec.allow_missing
        && spec.mode == UserCallMode::MethodDiscard
        && target.kind != BytecodeFunctionKind::Method
    {
        return Ok(None);
    }
    validate_user_call_target_kind(program, target, spec.mode)?;
    bind_user_call_signature(program, generation, target, spec).map(Some)
}

pub(crate) fn validate_user_call_target_kind(
    program: &ProgramGeneration,
    target: &erabasic_bytecode::BytecodeFunction,
    mode: UserCallMode,
) -> Result<(), VmError> {
    let valid = if mode.is_method() {
        target.kind == BytecodeFunctionKind::Method
    } else {
        target.kind != BytecodeFunctionKind::Method
            && (target.kind != BytecodeFunctionKind::Event
                || program.artifact.call_compatibility.allow_event_as_normal)
    };
    if !valid {
        return Err(script_argument(format!(
            "dynamic target {} has an incompatible kind",
            target.name
        )));
    }
    Ok(())
}

/// CALLSTR checks target kind outside TRY before restructuring and `ConvertArg`.
/// This shared binder keeps that argument-only capture boundary explicit.
pub(crate) fn bind_user_call_signature(
    program: &ProgramGeneration,
    generation: GenerationId,
    target: &erabasic_bytecode::BytecodeFunction,
    spec: &UserCallSpec,
) -> Result<ResolvedUserCall, VmError> {
    let name = &target.name;
    let policy = program.artifact.call_compatibility;
    let arguments = &spec.arguments;
    let arity = policy
        .user_argument_policy
        .decide(arguments.len(), target.parameters.len());
    if arity.is_rejected() {
        return Err(script_argument(format!(
            "function {name} expects at most {} arguments, found {}",
            target.parameters.len(),
            arguments.len()
        )));
    }
    let mut bindings = Vec::with_capacity(target.parameters.len());
    for (slot, parameter) in target.parameters.iter().enumerate() {
        let argument = arguments.get(slot).unwrap_or(&UserArgumentSpec::Omitted);
        if matches!(argument, UserArgumentSpec::Omitted) {
            if parameter.by_reference {
                return Err(script_argument(format!(
                    "method {name} omits reference argument {}",
                    slot + 1
                )));
            }
            let default = parameter
                .default
                .clone()
                .or_else(|| {
                    policy
                        .allow_omitted_arguments
                        .then(|| match parameter.value_type {
                            BytecodeType::String => BytecodeConstant::String(String::new()),
                            _ => BytecodeConstant::Integer(0),
                        })
                })
                .ok_or_else(|| {
                    script_argument(format!(
                        "method {name} omits required argument {}",
                        slot + 1
                    ))
                })?;
            bindings.push(UserArgumentBinding::Default(default));
            continue;
        }
        bindings.push(resolve_supplied_method_argument(
            program, name, slot, parameter, argument,
        )?);
    }
    if spec.mode.is_method()
        && !matches!(
            target.result,
            Some(BytecodeType::Integer | BytecodeType::String)
        )
    {
        return Err(script_argument(format!(
            "method {name} has no scalar return type"
        )));
    }
    if spec
        .mode
        .expected_result()
        .is_some_and(|expected| target.result != Some(expected))
    {
        return Err(script_argument(format!(
            "method {name} has an incompatible return type"
        )));
    }
    Ok(ResolvedUserCall {
        generation,
        function: target.key,
        mode: spec.mode,
        bindings,
    })
}

fn resolve_supplied_method_argument(
    program: &ProgramGeneration,
    name: &str,
    slot: usize,
    parameter: &erabasic_bytecode::BytecodeParameter,
    argument: &UserArgumentSpec,
) -> Result<UserArgumentBinding, VmError> {
    let actual_type = match argument {
        UserArgumentSpec::Value(value_type) => *value_type,
        UserArgumentSpec::Variable(key) => {
            program
                .global(*key)
                .ok_or_else(|| invalid("method argument variable is missing"))?
                .value_type
        }
        UserArgumentSpec::Omitted => unreachable!("omission was handled above"),
    };
    if parameter.by_reference {
        let UserArgumentSpec::Variable(key) = argument else {
            return Err(script_argument(format!(
                "method {name} argument {} requires an array",
                slot + 1
            )));
        };
        let source = program
            .global(*key)
            .ok_or_else(|| invalid("reference source is missing"))?;
        let destination = program
            .global(parameter.key)
            .ok_or_else(|| invalid("reference parameter is missing"))?;
        let expected = match actual_type {
            BytecodeType::Integer => BytecodeType::IntegerPlace,
            BytecodeType::String => BytecodeType::StringPlace,
            _ => return Err(invalid("method reference source is not a scalar array")),
        };
        if !source.mutable
            || matches!(source.storage, BytecodeStorage::Calculated)
            || source.dimensions.is_empty()
            || source.dimensions.len() != destination.dimensions.len()
            || parameter.value_type != expected
        {
            return Err(script_argument(format!(
                "method {name} argument {} has an incompatible array reference",
                slot + 1
            )));
        }
        Ok(UserArgumentBinding::ArrayReference)
    } else {
        let convert_integer_to_string = actual_type == BytecodeType::Integer
            && parameter.value_type == BytecodeType::String
            && program
                .artifact
                .call_compatibility
                .auto_convert_integer_to_string;
        if !matches!(actual_type, BytecodeType::Integer | BytecodeType::String)
            || (actual_type != parameter.value_type && !convert_integer_to_string)
        {
            return Err(script_argument(format!(
                "method {name} argument {} has an incompatible value type",
                slot + 1
            )));
        }
        Ok(UserArgumentBinding::Value {
            convert_integer_to_string,
        })
    }
}

pub(crate) fn exists_method(
    program: &ProgramGeneration,
    generation: GenerationId,
    name: &str,
) -> i64 {
    let spec = UserCallSpec {
        mode: UserCallMode::MethodDiscard,
        allow_missing: true,
        missing_target: 0,
        arguments: Vec::new(),
    };
    match resolve_user_call(program, generation, name, &spec) {
        Ok(Some(call)) => match program
            .function(call.function)
            .and_then(|target| target.result)
        {
            Some(BytecodeType::Integer) => 1,
            Some(BytecodeType::String) => 2,
            _ => 0,
        },
        Ok(None) | Err(_) => 0,
    }
}

impl Vm {
    /// Resolve a formal alias to its scoped backing without reselecting a character.
    pub(crate) fn user_call_array_place(
        &self,
        fiber: &Fiber,
        _generation: GenerationId,
        place: &PlaceDescriptor,
    ) -> Result<PlaceDescriptor, VmError> {
        let mut current = place.clone();
        let mut owners = std::collections::BTreeSet::new();
        loop {
            if current.fiber != Some(fiber.id) || !current.indices.is_empty() {
                return Err(invalid("REF does not identify a whole array in this fiber"));
            }
            if current.backing.is_some() {
                self.array_backing_record(fiber, &current)?;
                return Ok(current);
            }
            let (generation, definition) = self.place_definition(fiber, &current)?;
            let program = self
                .generations
                .get(&generation)
                .ok_or_else(|| invalid("REF generation is missing"))?;
            if !program.is_reference_variable(definition.key) {
                return Err(invalid("REF alias is not a captured backing"));
            }
            let frame = super::find_frame(fiber, current.frame, definition.owner)?;
            if !owners.insert((frame.id, definition.key)) {
                return Err(invalid("REF alias cycle"));
            }
            current = match frame
                .locals
                .get(&definition.key)
                .and_then(crate::VariableCell::first)
            {
                Some(VmValue::IntegerPlace(bound) | VmValue::StringPlace(bound))
                    if bound.backing.is_some() =>
                {
                    *bound
                }
                _ => return Err(super::references::unbound_reference()),
            };
        }
    }

    /// Snapshot aliases are checked against live owners before restoring any external state.
    pub(crate) fn valid_frame_references(&self, fiber: &Fiber, frame: &super::Frame) -> bool {
        let Some(program) = self.generations.get(&frame.generation) else {
            return false;
        };
        let Some(function) = program.function(frame.function) else {
            return false;
        };
        function
            .parameters
            .iter()
            .filter(|parameter| parameter.by_reference)
            .all(|parameter| {
                let Some(cell) = frame.locals.get(&parameter.key) else {
                    return false;
                };
                let Some(value) = cell.first() else {
                    return false;
                };
                if value.value_type() != parameter.value_type {
                    return false;
                }
                let Ok(VmValue::IntegerPlace(place) | VmValue::StringPlace(place)) =
                    self.user_call_variable_place(fiber, frame.generation, frame.id, parameter.key)
                else {
                    return false;
                };
                self.user_call_array_place(fiber, frame.generation, &place)
                    .is_ok()
            })
    }

    pub(crate) fn user_call_variable_place(
        &self,
        fiber: &Fiber,
        generation: GenerationId,
        owner: crate::FrameId,
        variable: SymbolKey,
    ) -> Result<VmValue, VmError> {
        let program = self
            .generations
            .get(&generation)
            .ok_or_else(|| invalid("method generation is missing"))?;
        let definition = program
            .global(variable)
            .ok_or_else(|| invalid("method variable is missing"))?;
        let place = PlaceDescriptor {
            variable,
            backing: None,
            indices: Vec::new(),
            character: (definition.storage == BytecodeStorage::Character)
                .then(|| self.target_character_for_generation(generation) as u64),
            fiber: Some(fiber.id),
            frame: (definition.storage == BytecodeStorage::FunctionLocal).then_some(owner),
        };
        Ok(match definition.value_type {
            BytecodeType::Integer => VmValue::IntegerPlace(Box::new(place)),
            BytecodeType::String => VmValue::StringPlace(Box::new(place)),
            _ => return Err(invalid("method variable has an invalid type")),
        })
    }

    #[allow(clippy::too_many_arguments)] // Call ownership and origin are independent validated inputs.
    pub(crate) fn capture_user_argument(
        &mut self,
        fiber: &Fiber,
        owner: crate::FrameId,
        method: &ResolvedUserCall,
        specs: &[UserArgumentSpec],
        slot: usize,
        actual: VmValue,
        origin: super::array_leases::ArrayLeaseOrigin,
    ) -> Result<VmValue, VmError> {
        let program = self
            .generations
            .get(&method.generation)
            .ok_or_else(|| invalid("method generation is missing"))?;
        let spec = specs
            .get(slot)
            .ok_or_else(|| invalid("method argument slot is missing"))?;
        match method
            .bindings
            .get(slot)
            .ok_or_else(|| invalid("method argument binding is missing"))?
        {
            UserArgumentBinding::Default(_) => {
                Err(invalid("omitted method argument was evaluated"))
            }
            UserArgumentBinding::Value {
                convert_integer_to_string,
            } => {
                let expected = match spec {
                    UserArgumentSpec::Value(value_type) => *value_type,
                    UserArgumentSpec::Variable(key) => {
                        program
                            .global(*key)
                            .ok_or_else(|| invalid("method argument variable is missing"))?
                            .value_type
                    }
                    UserArgumentSpec::Omitted => {
                        return Err(invalid("omitted method argument was captured"));
                    }
                };
                if actual.value_type() != expected {
                    return Err(invalid("captured method value has an incompatible type"));
                }
                if *convert_integer_to_string {
                    let VmValue::Integer(value) = actual else {
                        return Err(invalid("invalid method conversion"));
                    };
                    Ok(VmValue::String(value.to_string()))
                } else {
                    Ok(actual)
                }
            }
            UserArgumentBinding::ArrayReference => {
                let UserArgumentSpec::Variable(variable) = spec else {
                    return Err(invalid("reference argument has no variable identity"));
                };
                let expected_type = match program
                    .global(*variable)
                    .map(|definition| definition.value_type)
                {
                    Some(BytecodeType::Integer) => BytecodeType::IntegerPlace,
                    Some(BytecodeType::String) => BytecodeType::StringPlace,
                    _ => return Err(invalid("REF source scalar type is missing")),
                };
                let formal = program
                    .function(method.function)
                    .and_then(|function| function.parameters.get(slot))
                    .and_then(|parameter| program.global(parameter.key))
                    .ok_or_else(|| invalid("REF formal is missing"))?;
                let formal_type = formal.value_type;
                let formal_rank = formal.dimensions.len();
                if actual.value_type() != expected_type {
                    return Err(invalid("captured REF type differs"));
                }
                let (VmValue::IntegerPlace(mut actual) | VmValue::StringPlace(mut actual)) = actual
                else {
                    return Err(invalid("captured REF is not a place"));
                };
                if actual.variable != *variable
                    || actual.backing.is_some()
                    || actual.fiber != Some(fiber.id)
                    || actual.frame.is_some_and(|frame| frame != owner)
                {
                    return Err(invalid("captured REF differs from its source slot/owner"));
                }
                // MakePlace only evaluates the selected character. Its synthetic zero element
                // indices are discarded; ordinary source indices were never executed.
                actual.indices.clear();
                let actual = self.capture_array_reference(fiber, &actual, origin)?;
                let (_, cell) = self.array_backing_record(fiber, &actual)?;
                if cell.value_type != formal_type || cell.dimensions.len() != formal_rank {
                    self.memory
                        .array_leases
                        .release(actual.backing.expect("capture has identity"));
                    return Err(invalid(
                        "captured REF backing rank/type differs from formal",
                    ));
                }
                Ok(match expected_type {
                    BytecodeType::IntegerPlace => VmValue::IntegerPlace(Box::new(actual)),
                    _ => VmValue::StringPlace(Box::new(actual)),
                })
            }
        }
    }

    /// Later actuals may rebind the original REF. Captures keep the backing selected
    /// at their own evaluation point, not whichever alias is live at invocation.
    pub(crate) fn validate_captured_user_reference(
        &self,
        fiber: &Fiber,
        call: &ResolvedUserCall,
        slot: usize,
        value: &VmValue,
    ) -> Result<(), VmError> {
        let program = self
            .generations
            .get(&call.generation)
            .ok_or_else(|| invalid("captured user call generation is missing"))?;
        let target = program
            .function(call.function)
            .ok_or(VmError::MissingFunction(call.function))?;
        let formal = target
            .parameters
            .get(slot)
            .filter(|parameter| parameter.by_reference)
            .and_then(|parameter| program.global(parameter.key))
            .ok_or_else(|| invalid("captured reference formal is missing"))?;
        let (VmValue::IntegerPlace(place) | VmValue::StringPlace(place)) = value else {
            return Err(invalid("captured reference is not a place"));
        };
        if !place.indices.is_empty() {
            return Err(invalid("captured REF contains element indices"));
        }
        let (_, backing) = self.array_backing_record(fiber, place)?;
        let expected = match backing.value_type {
            BytecodeType::Integer => BytecodeType::IntegerPlace,
            BytecodeType::String => BytecodeType::StringPlace,
            _ => return Err(invalid("captured REF backing is not scalar")),
        };
        if value.value_type() != expected
            || backing.value_type != formal.value_type
            || backing.dimensions.len() != formal.dimensions.len()
        {
            return Err(invalid(
                "captured REF backing has an incompatible rank or type",
            ));
        }
        Ok(())
    }

    pub(crate) fn invoke_user_call(
        &mut self,
        fiber: &mut Fiber,
        owner: crate::FrameId,
        method: &ResolvedUserCall,
        specs: &[UserArgumentSpec],
        captured: &[Option<VmValue>],
        origin: UserCallOrigin,
    ) -> Result<(), VmError> {
        if fiber.frames.len() >= self.config.maximum_call_depth {
            return Err(VmError::ResourceLimit("maximum method call depth"));
        }
        let program = std::sync::Arc::clone(
            self.generations
                .get(&method.generation)
                .ok_or_else(|| invalid("method generation is missing"))?,
        );
        let target = program
            .function(method.function)
            .ok_or(VmError::MissingFunction(method.function))?;
        if resolve_user_call(
            &program,
            method.generation,
            &target.name,
            &UserCallSpec {
                mode: method.mode,
                allow_missing: false,
                missing_target: 0,
                arguments: specs.to_vec(),
            },
        )?
        .as_ref()
            != Some(method)
            || captured.len() != specs.len().min(method.bindings.len())
        {
            return Err(invalid(
                "resolved method state no longer matches its signature",
            ));
        }
        let caller = fiber
            .frames
            .last()
            .filter(|frame| frame.id == owner && frame.generation == method.generation)
            .ok_or_else(|| invalid("method caller generation or frame differs"))?;
        let event_context = caller.event_context || target.kind == BytecodeFunctionKind::Event;
        let mut arguments = Vec::with_capacity(method.bindings.len());
        for (slot, binding) in method.bindings.iter().enumerate() {
            let value = captured.get(slot).and_then(Option::as_ref);
            let value = match (binding, value) {
                (UserArgumentBinding::Default(value), None) => match value {
                    BytecodeConstant::Integer(value) => VmValue::Integer(*value),
                    BytecodeConstant::String(value) => VmValue::String(value.clone()),
                },
                (UserArgumentBinding::ArrayReference, Some(value)) => {
                    self.validate_captured_user_reference(fiber, method, slot, value)?;
                    value.clone()
                }
                (UserArgumentBinding::Value { .. }, Some(value))
                    if value.value_type() == target.parameters[slot].value_type =>
                {
                    value.clone()
                }
                _ => {
                    return Err(invalid(
                        "method captures do not match its omitted slots or parameter types",
                    ));
                }
            };
            arguments.push(value);
        }
        super::validate_arguments(target, &arguments)?;
        if matches!(origin, UserCallOrigin::RuntimeForm) || !method.allows_path_memo_observation() {
            self.invalidate_path_memo(fiber.id);
        }
        self.memory.ensure_function_statics(
            method.generation,
            target.key,
            program.function_statics(target.key),
        );
        bind_persistent_arguments(
            &mut self.memory,
            method.generation,
            target,
            &program,
            &arguments,
        )?;
        self.observe_path_memo_arguments(fiber.id, method.generation, target, &program, &arguments);
        let mut frame = make_frame(
            self.allocate_frame_id(),
            method.generation,
            target,
            program.function_locals(target.key),
            arguments,
            method.mode.expected_result().is_some(),
            event_context,
        );
        frame.user_call = Some(UserCallFrame {
            caller: owner,
            mode: method.mode,
            origin,
        });
        fiber.frames.push(frame);
        Ok(())
    }
}
