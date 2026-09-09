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

    #[inline]
    pub(super) fn instruction_position_at<'cursor>(
        &self,
        generation: crate::GenerationId,
        function_key: SymbolKey,
        instruction: usize,
        cursor: &'cursor mut Option<FunctionCursor>,
    ) -> Result<InstructionPosition<'cursor>, VmError> {
        if cursor
            .as_ref()
            .is_none_or(|cursor| cursor.generation != generation || cursor.function != function_key)
        {
            self.refresh_function_cursor(generation, function_key, cursor)?;
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
            resolved_program: Some((&cursor.program, cursor.index)),
            generation,
            function: function_key,
            instruction,
            variable: if encoded.opcode == Opcode::LoadVariable as u16
                || encoded.opcode == Opcode::StoreVariable as u16
                || encoded.opcode == Opcode::MakePlace as u16
            {
                cursor.program.instruction_global(cursor.index, instruction)
            } else {
                None
            },
            literal_group_match: if encoded.opcode == Opcode::PushInteger as u16
                || encoded.opcode == Opcode::PushString as u16
            {
                cursor
                    .program
                    .literal_group_match_plan(cursor.index, instruction)
            } else {
                None
            },
            encoded: DispatchInstruction {
                opcode: encoded.opcode,
                payload: &encoded.payload,
            },
        })
    }

    // Generation/function changes are uncommon relative to instruction dispatch. Keep
    // hash lookup, Arc ownership and error construction out of the per-instruction path.
    #[inline(never)]
    fn refresh_function_cursor(
        &self,
        generation: crate::GenerationId,
        function: SymbolKey,
        cursor: &mut Option<FunctionCursor>,
    ) -> Result<(), VmError> {
        if let Some(cursor) = cursor
            .as_mut()
            .filter(|cursor| cursor.generation == generation)
        {
            cursor.index = *cursor
                .program
                .function_index(function)
                .ok_or(VmError::MissingFunction(function))?;
            cursor.function = function;
        } else {
            let program =
                Arc::clone(self.generations.get(&generation).ok_or_else(|| {
                    VmError::InvalidState("frame generation was reclaimed".into())
                })?);
            let index = *program
                .function_index(function)
                .ok_or(VmError::MissingFunction(function))?;
            *cursor = Some(FunctionCursor {
                generation,
                function,
                index,
                program,
            });
        }
        Ok(())
    }
}
