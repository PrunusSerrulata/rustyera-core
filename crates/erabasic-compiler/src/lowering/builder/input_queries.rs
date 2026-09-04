//! Reference focus gate occurs before the key-code argument is evaluated.
use super::super::{BytecodeType, HirCallArgument, Opcode, SourceLocation, opcode};
use super::Builder;
impl Builder<'_> {
    pub(super) fn lower_key_query(
        &mut self,
        name: &str,
        arguments: &[HirCallArgument],
        location: SourceLocation,
    ) -> Option<BytecodeType> {
        if !self.context.program.snake_input || !matches!(name, "GETKEY" | "GETKEYTRIGGERED") {
            return None;
        }
        let [HirCallArgument::Value(code)] = arguments else {
            return None;
        };
        self.emit_runtime_call(
            "__GETKEY_ACTIVE",
            &[],
            Some(BytecodeType::Integer),
            false,
            location,
        );
        let inactive = self.code.len();
        self.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
        self.lower_expression(code, location);
        self.emit_runtime_call(
            name,
            &[BytecodeType::Integer],
            Some(BytecodeType::Integer),
            false,
            location,
        );
        let end = self.code.len();
        self.emit(opcode::jump(Opcode::Jump, 0), location);
        self.patch_jump(inactive, self.code.len());
        self.emit(opcode::push_integer(0), location);
        self.patch_jump(end, self.code.len());
        Some(BytecodeType::Integer)
    }
}
