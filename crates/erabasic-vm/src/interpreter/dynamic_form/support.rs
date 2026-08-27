use erabasic_ast::{BinaryOp, UnaryOp};
use erabasic_bytecode::BytecodeStorage;

use super::{MAX_RUNTIME_FORM_BYTES, MAX_RUNTIME_FORM_NESTING, RuntimeFormContinuation};
use crate::interpreter::{StepError, map_vm_error};
use crate::{Fiber, FrameId, Vm, VmFaultCode, VmValue};

impl RuntimeFormContinuation {
    pub(super) fn read_variable(
        &self,
        vm: &Vm,
        fiber: &Fiber,
        name: &str,
        indices: &[u64],
    ) -> Result<VmValue, StepError> {
        let generation = vm.generations.get(&self.generation).ok_or_else(|| {
            StepError::new(VmFaultCode::MissingSymbol, "STRFORM generation is missing")
        })?;
        let definition = generation
            .scoped_variable(self.function, name)
            .ok_or_else(|| {
                StepError::new(
                    VmFaultCode::MissingSymbol,
                    format!("STRFORM variable {name} is missing"),
                )
            })?;
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
            indices
        };
        vm.read_variable_resolved(
            fiber,
            self.generation,
            definition,
            value_indices,
            character,
            (definition.storage == BytecodeStorage::FunctionLocal).then_some(self.frame),
        )
        .map_err(map_vm_error)
    }

    pub(super) fn take_indices(&mut self, count: usize) -> Result<Vec<u64>, StepError> {
        self.take_values(count)?
            .into_iter()
            .map(|value| match value {
                VmValue::Integer(value) => u64::try_from(value).map_err(|_| {
                    StepError::new(VmFaultCode::Bounds, "STRFORM variable index is negative")
                }),
                _ => Err(StepError::new(
                    VmFaultCode::TypeMismatch,
                    "STRFORM variable index is not an integer",
                )),
            })
            .collect()
    }

    pub(super) fn take_values(&mut self, count: usize) -> Result<Vec<VmValue>, StepError> {
        let start = self.values.len().checked_sub(count).ok_or_else(|| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                "STRFORM value stack underflow",
            )
        })?;
        Ok(self.values.drain(start..).collect())
    }

    pub(super) fn pop_value(&mut self, message: &str) -> Result<VmValue, StepError> {
        self.values
            .pop()
            .ok_or_else(|| StepError::new(VmFaultCode::InvalidInstruction, message))
    }

    pub(super) fn pop_integer(&mut self, message: &str) -> Result<i64, StepError> {
        let VmValue::Integer(value) = self.pop_value(message)? else {
            return Err(StepError::new(VmFaultCode::TypeMismatch, message));
        };
        Ok(value)
    }

    fn output_mut(&mut self) -> Result<&mut String, StepError> {
        self.outputs.last_mut().ok_or_else(|| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                "STRFORM output stack is empty",
            )
        })
    }

    pub(super) fn append_output(&mut self, value: &str) -> Result<(), StepError> {
        let retained = self.retained_string_bytes()?;
        if retained
            .checked_add(value.len())
            .is_none_or(|bytes| bytes > MAX_RUNTIME_FORM_BYTES)
        {
            return Err(resource_limit("STRFORM output exceeds the runtime limit"));
        }
        self.output_mut()?.push_str(value);
        Ok(())
    }

    fn retained_string_bytes(&self) -> Result<usize, StepError> {
        let (_, method_bytes) = self
            .method_resources()
            .ok_or_else(|| resource_limit("STRFORM method resource count overflowed"))?;
        self.outputs
            .iter()
            .map(String::len)
            .chain(self.values.iter().filter_map(|value| match value {
                VmValue::String(value) => Some(value.len()),
                _ => None,
            }))
            .try_fold(method_bytes, usize::checked_add)
            .ok_or_else(|| resource_limit("STRFORM retained string size overflowed"))
    }

    pub(super) fn check_resources(&self, vm: &Vm) -> Result<(), StepError> {
        let (method_slots, _) = self
            .method_resources()
            .ok_or_else(|| resource_limit("STRFORM method resource count overflowed"))?;
        if self
            .work
            .len()
            .checked_add(method_slots)
            .is_none_or(|count| count > vm.config.maximum_operand_stack)
            || self.values.len() > vm.config.maximum_operand_stack
            || self.outputs.len() > MAX_RUNTIME_FORM_NESTING
        {
            return Err(resource_limit("STRFORM continuation exceeds VM limits"));
        }
        if self.retained_string_bytes()? > MAX_RUNTIME_FORM_BYTES {
            return Err(resource_limit(
                "STRFORM retained strings exceed the runtime limit",
            ));
        }
        Ok(())
    }
}

pub(super) fn owner_frame(
    fiber: &Fiber,
    frame: FrameId,
) -> Result<&crate::state::Frame, StepError> {
    fiber
        .frames
        .iter()
        .find(|candidate| candidate.id == frame)
        .ok_or_else(|| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                "STRFORM owner frame is missing",
            )
        })
}

pub(super) fn owner_frame_mut(
    fiber: &mut Fiber,
    frame: FrameId,
) -> Result<&mut crate::state::Frame, StepError> {
    fiber
        .frames
        .iter_mut()
        .find(|candidate| candidate.id == frame)
        .ok_or_else(|| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                "STRFORM owner frame is missing",
            )
        })
}

pub(super) fn unsupported(message: impl Into<String>) -> StepError {
    StepError::new(VmFaultCode::Native, message)
}

pub(super) fn resource_limit(message: impl Into<String>) -> StepError {
    StepError::new(VmFaultCode::ResourceLimit, message)
}

pub(super) const fn unary_tag(op: UnaryOp) -> u8 {
    match op {
        UnaryOp::Plus => 0,
        UnaryOp::Minus => 1,
        UnaryOp::LogicalNot => 2,
        UnaryOp::BitNot => 3,
        UnaryOp::PreIncrement => 4,
        UnaryOp::PreDecrement => 5,
    }
}

pub(super) const fn binary_tag(op: BinaryOp) -> u8 {
    match op {
        BinaryOp::Multiply => 0,
        BinaryOp::Divide => 1,
        BinaryOp::Modulo => 2,
        BinaryOp::Add => 3,
        BinaryOp::Subtract => 4,
        BinaryOp::ShiftLeft => 5,
        BinaryOp::ShiftRight => 6,
        BinaryOp::Less => 7,
        BinaryOp::LessEqual => 8,
        BinaryOp::Greater => 9,
        BinaryOp::GreaterEqual => 10,
        BinaryOp::Equal => 11,
        BinaryOp::NotEqual => 12,
        BinaryOp::BitAnd => 13,
        BinaryOp::BitXor => 14,
        BinaryOp::BitOr => 15,
        BinaryOp::LogicalAnd => 16,
        BinaryOp::LogicalXor => 17,
        BinaryOp::LogicalOr => 18,
        BinaryOp::Nand => 19,
        BinaryOp::Nor => 20,
    }
}
