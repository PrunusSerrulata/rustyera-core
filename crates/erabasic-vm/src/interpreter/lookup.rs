#[allow(clippy::wildcard_imports)]
use super::*;

impl Vm {
    pub(super) fn instruction_position<'cursor>(
        &self,
        fiber: &Fiber,
        cursor: &'cursor mut Option<FunctionCursor>,
    ) -> Result<InstructionPosition<'cursor>, VmError> {
        let frame = fiber
            .frames
            .last()
            .ok_or_else(|| VmError::InvalidState("runnable fiber has no frame".into()))?;
        self.instruction_position_at(frame.generation, frame.function, frame.instruction, cursor)
    }

    pub(super) fn instruction_position_at<'cursor>(
        &self,
        generation: crate::GenerationId,
        function_key: SymbolKey,
        instruction: usize,
        cursor: &'cursor mut Option<FunctionCursor>,
    ) -> Result<InstructionPosition<'cursor>, VmError> {
        if cursor
            .as_ref()
            .is_none_or(|cursor| cursor.generation != generation)
        {
            let program =
                Arc::clone(self.generations.get(&generation).ok_or_else(|| {
                    VmError::InvalidState("frame generation was reclaimed".into())
                })?);
            let index = *program
                .function_index(function_key)
                .ok_or(VmError::MissingFunction(function_key))?;
            *cursor = Some(FunctionCursor {
                generation,
                function: function_key,
                index,
                program,
            });
        } else if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.function != function_key)
        {
            let cursor = cursor.as_mut().expect("the generation cursor exists");
            cursor.index = *cursor
                .program
                .function_index(function_key)
                .ok_or(VmError::MissingFunction(function_key))?;
            cursor.function = function_key;
        }
        let cursor = cursor
            .as_ref()
            .expect("the generation cursor was initialized");
        let function = cursor
            .program
            .artifact
            .functions
            .get(cursor.index)
            .filter(|function| function.key == function_key)
            .ok_or(VmError::MissingFunction(function_key))?;
        let encoded = function
            .code
            .get(instruction)
            .ok_or_else(|| VmError::InvalidState("instruction pointer left its function".into()))?;
        // The cursor owns the generation Arc, so this payload borrow is independent
        // of `self` and remains valid across mutable VM dispatch for this instruction.
        Ok(InstructionPosition {
            generation,
            function: function_key,
            instruction,
            variable: cursor.program.instruction_global(cursor.index, instruction),
            encoded: DispatchInstruction {
                opcode: encoded.opcode,
                payload: &encoded.payload,
            },
        })
    }
}
