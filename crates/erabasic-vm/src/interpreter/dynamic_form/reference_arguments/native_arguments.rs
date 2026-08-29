//! Typed place adaptation for an active, validated reference argument template.
//! This reuses `MakePlace`'s descriptor semantics and the shared VM-native executor.
use super::{
    BytecodeType, Fiber, RuntimeFormContinuation, StepError, SymbolKey, Vm, VmValue, invalid,
};
use erabasic_bytecode::BytecodeStorage;

impl RuntimeFormContinuation {
    pub(in crate::interpreter::dynamic_form) fn capture_reference_place(
        &mut self,
        vm: &Vm,
        fiber: &Fiber,
        key: SymbolKey,
        count: usize,
    ) -> Result<(), StepError> {
        let indices = self.take_indices(count)?;
        let program = vm
            .generations
            .get(&self.generation)
            .ok_or_else(|| invalid("reference argument place generation missing"))?;
        let definition = program
            .global(key)
            .ok_or_else(|| invalid("reference argument place definition missing"))?;
        let explicit_character = definition.storage == BytecodeStorage::Character
            && indices.len() > definition.dimensions.len();
        let character = (definition.storage == BytecodeStorage::Character).then(|| {
            if explicit_character {
                indices[0]
            } else {
                vm.target_character_for_generation(self.generation) as u64
            }
        });
        let place = crate::PlaceDescriptor {
            variable: key,
            indices: if explicit_character {
                indices[1..].to_vec()
            } else {
                indices
            },
            character,
            fiber: Some(fiber.id),
            frame: (definition.storage == BytecodeStorage::FunctionLocal).then_some(self.frame),
            backing: None,
        };
        self.values.push(match definition.value_type {
            BytecodeType::Integer => VmValue::IntegerPlace(Box::new(place)),
            BytecodeType::String => VmValue::StringPlace(Box::new(place)),
            _ => {
                return Err(invalid(
                    "reference argument variable schema contains a place",
                ));
            }
        });
        Ok(())
    }
}
