use erabasic_bytecode::{
    BytecodeConstant, BytecodeFunctionKind, BytecodeStorage, BytecodeType, MethodArgumentSpec,
    MethodResult, SymbolKey,
};
use serde::{Deserialize, Serialize};

use super::{Fiber, ProgramGeneration};
use crate::{
    GenerationId, PlaceDescriptor, Vm, VmError, VmValue, bind_persistent_arguments, make_frame,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ResolvedMethod {
    pub generation: GenerationId,
    pub function: SymbolKey,
    pub result: MethodResult,
    pub bindings: Vec<MethodBinding>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum MethodBinding {
    Default(BytecodeConstant),
    Value { convert_integer_to_string: bool },
    ArrayReference,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct PendingMethodCall {
    pub resolve: usize,
    pub stack_index: usize,
    pub captured: usize,
    pub method: ResolvedMethod,
}

fn invalid(message: impl Into<String>) -> VmError {
    VmError::InvalidArguments(message.into())
}

/// Resolve the complete signature without evaluating any actual or fallback expression.
pub(crate) fn resolve_method_call(
    program: &ProgramGeneration,
    generation: GenerationId,
    name: &str,
    arguments: &[MethodArgumentSpec],
    result: Option<MethodResult>,
) -> Result<Option<ResolvedMethod>, VmError> {
    let Some(target) = program.function_by_name(name) else {
        return Ok(None);
    };
    let policy = program.artifact.call_compatibility;
    if target.kind == BytecodeFunctionKind::Event && !policy.allow_event_as_normal {
        return Ok(None);
    }
    if target.kind != BytecodeFunctionKind::Method {
        return Err(invalid(format!("dynamic target {name} is not a method")));
    }
    if arguments.len() > target.parameters.len() {
        return Err(invalid(format!(
            "method {name} expects at most {} arguments, found {}",
            target.parameters.len(),
            arguments.len()
        )));
    }
    let mut bindings = Vec::with_capacity(target.parameters.len());
    for (slot, parameter) in target.parameters.iter().enumerate() {
        let argument = arguments.get(slot).unwrap_or(&MethodArgumentSpec::Omitted);
        if matches!(argument, MethodArgumentSpec::Omitted) {
            if parameter.by_reference {
                return Err(invalid(format!(
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
                    invalid(format!(
                        "method {name} omits required argument {}",
                        slot + 1
                    ))
                })?;
            bindings.push(MethodBinding::Default(default));
            continue;
        }
        bindings.push(resolve_supplied_method_argument(
            program, name, slot, parameter, argument,
        )?);
    }
    let actual_result = match target.result {
        Some(BytecodeType::Integer) => MethodResult::Integer,
        Some(BytecodeType::String) => MethodResult::String,
        _ => {
            return Err(invalid(format!(
                "method {name} does not return an integer or string"
            )));
        }
    };
    if result.is_some_and(|expected| expected != actual_result) {
        return Err(invalid(format!(
            "method {name} has an incompatible return type"
        )));
    }
    Ok(Some(ResolvedMethod {
        generation,
        function: target.key,
        result: actual_result,
        bindings,
    }))
}

fn resolve_supplied_method_argument(
    program: &ProgramGeneration,
    name: &str,
    slot: usize,
    parameter: &erabasic_bytecode::BytecodeParameter,
    argument: &MethodArgumentSpec,
) -> Result<MethodBinding, VmError> {
    let actual_type = match argument {
        MethodArgumentSpec::Value(value_type) => *value_type,
        MethodArgumentSpec::Variable(key) => {
            program
                .global(*key)
                .ok_or_else(|| invalid("method argument variable is missing"))?
                .value_type
        }
        MethodArgumentSpec::Omitted => unreachable!("omission was handled above"),
    };
    if parameter.by_reference {
        let MethodArgumentSpec::Variable(key) = argument else {
            return Err(invalid(format!(
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
            || matches!(
                source.storage,
                BytecodeStorage::Character | BytecodeStorage::Calculated
            )
            || source.dimensions.is_empty()
            || source.dimensions.len() != destination.dimensions.len()
            || parameter.value_type != expected
        {
            return Err(invalid(format!(
                "method {name} argument {} has an incompatible array reference",
                slot + 1
            )));
        }
        Ok(MethodBinding::ArrayReference)
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
            return Err(invalid(format!(
                "method {name} argument {} has an incompatible value type",
                slot + 1
            )));
        }
        Ok(MethodBinding::Value {
            convert_integer_to_string,
        })
    }
}

pub(crate) fn exists_method(
    program: &ProgramGeneration,
    generation: GenerationId,
    name: &str,
) -> i64 {
    match resolve_method_call(program, generation, name, &[], None) {
        Ok(Some(method)) => match method.result {
            MethodResult::Integer => 1,
            MethodResult::String => 2,
        },
        Ok(None) | Err(_) => 0,
    }
}

impl Vm {
    /// Follow only existing whole-array REF bindings; reject stale owners, cycles and slices.
    pub(crate) fn method_array_place(
        &self,
        fiber: &Fiber,
        generation: GenerationId,
        place: &PlaceDescriptor,
    ) -> Result<PlaceDescriptor, VmError> {
        let program = self
            .generations
            .get(&generation)
            .ok_or_else(|| invalid("method reference generation is missing"))?;
        let mut current = place.clone();
        let source = program
            .global(place.variable)
            .ok_or_else(|| invalid("method reference source is missing"))?;
        let source_type = source.value_type;
        let source_rank = source.dimensions.len();
        let mut alias_owner = None;
        let mut seen = std::collections::BTreeSet::new();
        loop {
            if current.fiber != Some(fiber.id)
                || current.character.is_some()
                || !current.indices.is_empty()
                || !seen.insert((current.variable, current.frame))
            {
                return Err(invalid(
                    "method reference is stale, cyclic, or not a whole array",
                ));
            }
            let definition = program
                .global(current.variable)
                .ok_or_else(|| invalid("method reference variable is missing"))?;
            if definition.value_type != source_type
                || !definition.mutable
                || definition.dimensions.is_empty()
                || definition.dimensions.len() != source_rank
                || matches!(
                    definition.storage,
                    BytecodeStorage::Character | BytecodeStorage::Calculated
                )
            {
                return Err(invalid(
                    "method reference requires a mutable non-character array",
                ));
            }
            if definition.storage == BytecodeStorage::FunctionLocal {
                let (owner_index, owner) = fiber
                    .frames
                    .iter()
                    .enumerate()
                    .find(|(_, frame)| {
                        Some(frame.id) == current.frame
                            && frame.generation == generation
                            && Some(frame.function) == definition.owner
                    })
                    .ok_or_else(|| invalid("method reference owner frame is missing"))?;
                if alias_owner.is_some_and(|previous| owner_index >= previous) {
                    return Err(invalid(
                        "method reference does not point to an ancestor frame",
                    ));
                }
                let cell = owner
                    .locals
                    .get(&current.variable)
                    .ok_or_else(|| invalid("method reference local storage is missing"))?;
                if let Some(value @ (VmValue::IntegerPlace(_) | VmValue::StringPlace(_))) =
                    cell.first()
                {
                    let expected = match source_type {
                        BytecodeType::Integer => BytecodeType::IntegerPlace,
                        BytecodeType::String => BytecodeType::StringPlace,
                        _ => return Err(invalid("method reference source is not scalar")),
                    };
                    if value.value_type() != expected {
                        return Err(invalid("method alias storage has an incompatible type"));
                    }
                    if let VmValue::IntegerPlace(bound) | VmValue::StringPlace(bound) = value {
                        alias_owner = Some(owner_index);
                        current = *bound;
                        continue;
                    }
                }
            } else if current.frame.is_some()
                || self.memory.cell(generation, definition, 0).is_none()
            {
                return Err(invalid(
                    "method reference storage or frame identity is invalid",
                ));
            }
            return Ok(current);
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
                    self.method_variable_place(fiber, frame.generation, frame.id, parameter.key)
                else {
                    return false;
                };
                self.method_array_place(fiber, frame.generation, &place)
                    .is_ok()
            })
    }

    pub(crate) fn method_variable_place(
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
            indices: Vec::new(),
            character: None,
            fiber: Some(fiber.id),
            frame: (definition.storage == BytecodeStorage::FunctionLocal).then_some(owner),
        };
        Ok(match definition.value_type {
            BytecodeType::Integer => VmValue::IntegerPlace(Box::new(place)),
            BytecodeType::String => VmValue::StringPlace(Box::new(place)),
            _ => return Err(invalid("method variable has an invalid type")),
        })
    }

    pub(crate) fn capture_method_argument(
        &self,
        fiber: &Fiber,
        owner: crate::FrameId,
        method: &ResolvedMethod,
        specs: &[MethodArgumentSpec],
        slot: usize,
        actual: VmValue,
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
            MethodBinding::Default(_) => Err(invalid("omitted method argument was evaluated")),
            MethodBinding::Value {
                convert_integer_to_string,
            } => {
                let expected = match spec {
                    MethodArgumentSpec::Value(value_type) => *value_type,
                    MethodArgumentSpec::Variable(key) => {
                        program
                            .global(*key)
                            .ok_or_else(|| invalid("method argument variable is missing"))?
                            .value_type
                    }
                    MethodArgumentSpec::Omitted => {
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
            MethodBinding::ArrayReference => {
                let MethodArgumentSpec::Variable(variable) = spec else {
                    return Err(invalid("reference argument has no variable identity"));
                };
                let expected =
                    self.method_variable_place(fiber, method.generation, owner, *variable)?;
                let expected_type = expected.value_type();
                if actual.value_type() != expected_type {
                    return Err(invalid(
                        "captured method reference has an incompatible type",
                    ));
                }
                let (VmValue::IntegerPlace(expected) | VmValue::StringPlace(expected)) = expected
                else {
                    unreachable!("helper returns a place");
                };
                let (VmValue::IntegerPlace(actual) | VmValue::StringPlace(actual)) = actual else {
                    return Err(invalid("captured method reference is not a place"));
                };
                let expected = self.method_array_place(fiber, method.generation, &expected)?;
                let actual = self.method_array_place(fiber, method.generation, &actual)?;
                if actual != expected {
                    return Err(invalid(
                        "captured reference does not match the argument variable",
                    ));
                }
                let target = program
                    .function(method.function)
                    .ok_or(VmError::MissingFunction(method.function))?;
                let formal = program
                    .global(target.parameters[slot].key)
                    .ok_or_else(|| invalid("method formal is missing"))?;
                let backing = program
                    .global(actual.variable)
                    .ok_or_else(|| invalid("method backing array is missing"))?;
                if backing.dimensions.len() != formal.dimensions.len()
                    || backing.value_type != formal.value_type
                {
                    return Err(invalid(
                        "method backing array has an incompatible rank or type",
                    ));
                }
                Ok(match expected_type {
                    BytecodeType::IntegerPlace => VmValue::IntegerPlace(Box::new(actual)),
                    _ => VmValue::StringPlace(Box::new(actual)),
                })
            }
        }
    }

    pub(crate) fn invoke_method(
        &mut self,
        fiber: &mut Fiber,
        owner: crate::FrameId,
        method: &ResolvedMethod,
        specs: &[MethodArgumentSpec],
        captured: &[Option<VmValue>],
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
        if resolve_method_call(
            &program,
            method.generation,
            &target.name,
            specs,
            Some(method.result),
        )?
        .as_ref()
            != Some(method)
            || captured.len() != specs.len()
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
        let event_context = caller.event_context;
        let mut arguments = Vec::with_capacity(method.bindings.len());
        for (slot, binding) in method.bindings.iter().enumerate() {
            let value = captured.get(slot).and_then(Option::as_ref);
            let value = match (binding, value) {
                (MethodBinding::Default(value), None) => match value {
                    BytecodeConstant::Integer(value) => VmValue::Integer(*value),
                    BytecodeConstant::String(value) => VmValue::String(value.clone()),
                },
                (MethodBinding::ArrayReference, Some(value)) => {
                    self.capture_method_argument(fiber, owner, method, specs, slot, value.clone())?
                }
                (MethodBinding::Value { .. }, Some(value))
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
        self.invalidate_path_memo(fiber.id);
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
        let frame = make_frame(
            self.allocate_frame_id(),
            method.generation,
            target,
            program.function_locals(target.key),
            arguments,
            true,
            event_context,
        );
        fiber.frames.push(frame);
        Ok(())
    }

    pub(crate) fn validate_method_references(
        &self,
        fiber: &Fiber,
        owner: crate::FrameId,
        method: &ResolvedMethod,
        specs: &[MethodArgumentSpec],
    ) -> Result<(), VmError> {
        for (slot, binding) in method.bindings.iter().enumerate() {
            if !matches!(binding, MethodBinding::ArrayReference) {
                continue;
            }
            let Some(MethodArgumentSpec::Variable(variable)) = specs.get(slot) else {
                return Err(invalid("method reference has no variable identity"));
            };
            let place = self.method_variable_place(fiber, method.generation, owner, *variable)?;
            self.capture_method_argument(fiber, owner, method, specs, slot, place)?;
        }
        Ok(())
    }

    pub(crate) fn valid_frame_methods(&self, fiber: &Fiber, frame: &super::Frame) -> bool {
        let Some(program) = self.generations.get(&frame.generation) else {
            return false;
        };
        let Some(function) = program.function(frame.function) else {
            return false;
        };
        let mut previous_end = None;
        for pending in &frame.method_calls {
            let Some(instruction) = function.code.get(pending.resolve) else {
                return false;
            };
            if instruction.opcode != erabasic_bytecode::Opcode::ResolveMethod as u16
                || pending.resolve >= frame.instruction
                || pending.method.generation != frame.generation
                || previous_end.is_some_and(|end| pending.stack_index <= end)
            {
                return false;
            }
            let Ok(spec) = erabasic_bytecode::MethodCallSpec::decode(&instruction.payload) else {
                return false;
            };
            let Some(target) = program.function(pending.method.function) else {
                return false;
            };
            if frame.stack.get(pending.stack_index) != Some(&VmValue::String(target.name.clone()))
                || resolve_method_call(
                    program,
                    frame.generation,
                    &target.name,
                    &spec.arguments,
                    Some(spec.result),
                )
                .ok()
                .flatten()
                .as_ref()
                    != Some(&pending.method)
            {
                return false;
            }
            let slots = spec
                .arguments
                .iter()
                .enumerate()
                .filter(|(_, spec)| !matches!(spec, MethodArgumentSpec::Omitted))
                .map(|(slot, _)| slot)
                .collect::<Vec<_>>();
            if pending.captured > slots.len() {
                return false;
            }
            for (offset, slot) in slots.into_iter().take(pending.captured).enumerate() {
                let Some(value) = frame.stack.get(pending.stack_index + offset + 1) else {
                    return false;
                };
                if value.value_type() != target.parameters[slot].value_type {
                    return false;
                }
                if matches!(pending.method.bindings[slot], MethodBinding::ArrayReference)
                    && self
                        .capture_method_argument(
                            fiber,
                            frame.id,
                            &pending.method,
                            &spec.arguments,
                            slot,
                            value.clone(),
                        )
                        .is_err()
                {
                    return false;
                }
            }
            previous_end = pending.stack_index.checked_add(pending.captured);
            if previous_end.is_none() {
                return false;
            }
        }
        true
    }
}
