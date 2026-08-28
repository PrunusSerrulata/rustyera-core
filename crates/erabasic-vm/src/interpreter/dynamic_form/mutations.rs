//! Runtime expressions use the same place mutation primitive as compiled ++/--.
use super::{
    BytecodeType, Expr, ExprKind, Fiber, MAX_RUNTIME_FORM_NESTING, RuntimeFormContinuation,
    RuntimeFormTask, StepError, SymbolKey, Vm, VmFaultCode, VmValue, map_vm_error, methods,
    resource_limit,
};
use erabasic_bytecode::BytecodeStorage;

pub(super) fn mutation_variable<'a>(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    operand: &'a Expr,
    depth: usize,
) -> Result<(SymbolKey, &'a [Expr]), StepError> {
    if depth > MAX_RUNTIME_FORM_NESTING {
        return Err(resource_limit("mutation operand nesting limit"));
    }
    let (name, indices) = match &operand.kind {
        ExprKind::Group(inner) => return mutation_variable(program, function, inner, depth + 1),
        ExprKind::Identifier(name) => (name, &[][..]),
        ExprKind::Variable { name, indices } => (name, indices.as_slice()),
        _ => {
            return Err(StepError::script(
                crate::ScriptFaultKind::Argument,
                VmFaultCode::TypeMismatch,
                "increment/decrement operand must be a mutable integer variable",
            ));
        }
    };
    let definition = program.scoped_variable(function, name).ok_or_else(|| {
        StepError::script(
            crate::ScriptFaultKind::Resolve,
            VmFaultCode::MissingSymbol,
            format!("mutation variable {name} is missing"),
        )
    })?;
    if !definition.mutable || definition.value_type != BytecodeType::Integer {
        return Err(StepError::script(
            crate::ScriptFaultKind::Argument,
            VmFaultCode::TypeMismatch,
            "increment/decrement operand is not a writable integer place",
        ));
    }
    if indices.len()
        > definition.dimensions.len()
            + usize::from(definition.storage == BytecodeStorage::Character)
    {
        return Err(StepError::script(
            crate::ScriptFaultKind::Argument,
            VmFaultCode::Bounds,
            "mutation index count exceeds variable rank",
        ));
    }
    for index in indices {
        if methods::expression_type(program, function, index, depth + 1)? != BytecodeType::Integer {
            return Err(StepError::script(
                crate::ScriptFaultKind::Argument,
                VmFaultCode::TypeMismatch,
                "mutation index must be an integer",
            ));
        }
    }
    Ok((definition.key, indices))
}

impl RuntimeFormContinuation {
    pub(super) fn schedule_integer_mutation(
        &mut self,
        vm: &Vm,
        operand: &Expr,
        mode: u8,
    ) -> Result<(), StepError> {
        let program = vm
            .generations
            .get(&self.generation)
            .ok_or_else(|| invalid("mutation generation is missing"))?;
        let (variable, indices) = mutation_variable(program, self.function, operand, 0)?;
        self.work.push(RuntimeFormTask::MutateVariable {
            variable,
            indices: indices.len(),
            mode,
        });
        self.work
            .extend(indices.iter().rev().cloned().map(RuntimeFormTask::Evaluate));
        Ok(())
    }

    pub(super) fn valid_mutation_task(
        &self,
        vm: &Vm,
        variable: SymbolKey,
        indices: usize,
        mode: u8,
    ) -> bool {
        let Some(program) = vm.generations.get(&self.generation) else {
            return false;
        };
        let Some(definition) = program.global(variable) else {
            return false;
        };
        mode <= 3
            && definition.mutable
            && definition.value_type == BytecodeType::Integer
            && program
                .scoped_variable(self.function, &definition.name)
                .map(|definition| definition.key)
                == Some(variable)
            && indices
                <= definition.dimensions.len()
                    + usize::from(definition.storage == BytecodeStorage::Character)
    }

    pub(super) fn mutate_variable(
        &mut self,
        vm: &mut Vm,
        fiber: &mut Fiber,
        variable: SymbolKey,
        count: usize,
        mode: u8,
    ) -> Result<(), StepError> {
        if !self.valid_mutation_task(vm, variable, count, mode)
            || fiber.frames.last().map(|frame| frame.id) != Some(self.frame)
        {
            return Err(invalid(
                "stored mutation task has an invalid symbol/mode/owner",
            ));
        }
        let indices = self.take_indices(count)?;
        let definition = vm.generations[&self.generation]
            .global(variable)
            .ok_or_else(|| invalid("mutation variable disappeared"))?;
        let character = if definition.storage == BytecodeStorage::Character {
            Some(if indices.len() > definition.dimensions.len() {
                indices[0]
            } else {
                vm.target_character_for_generation(self.generation) as u64
            })
        } else {
            None
        };
        let value_indices = if character.is_some() && indices.len() > definition.dimensions.len() {
            &indices[1..]
        } else {
            &indices
        };
        let place = crate::PlaceDescriptor {
            variable,
            indices: value_indices.to_vec(),
            character,
            fiber: Some(fiber.id),
            frame: (definition.storage == BytecodeStorage::FunctionLocal).then_some(self.frame),
        };
        // This resolves REF aliases, enforces bounds/readonly storage, runs the common
        // profile arithmetic/warning policy, and commits through VM write_place.
        let value = super::super::execute_integer_mutation(
            vm,
            fiber,
            &[
                VmValue::IntegerPlace(Box::new(place)),
                VmValue::Integer(i64::from(mode)),
            ],
        )
        .map_err(map_vm_error)?;
        self.values.push(value);
        Ok(())
    }
}

fn invalid(message: &str) -> StepError {
    StepError::new(VmFaultCode::InvalidInstruction, message)
}
