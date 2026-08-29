//! MAP captures an object before tail evaluation; VALUES resolves output only when enabled.
use super::super::{
    BytecodeType, EncodedInstruction, ExecutionBinding, HirCallArgument, HirExprKind, Opcode,
    SourceLocation, bytecode_type, opcode,
};
use super::Builder;
use erabasic_bytecode::MapCallKind;
impl Builder<'_> {
    fn map_parameter_types(kind: MapCallKind, arguments: &[HirCallArgument]) -> Vec<BytecodeType> {
        arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                if kind == MapCallKind::Values && arguments.len() == 3 && index == 1 {
                    return BytecodeType::StringPlace;
                }
                match argument {
                    HirCallArgument::Value(value) => {
                        bytecode_type(value.value_type).unwrap_or(BytecodeType::Integer)
                    }
                    HirCallArgument::Place(place) => {
                        bytecode_type(place.value_type).unwrap_or(BytecodeType::Integer)
                    }
                    HirCallArgument::Omitted => BytecodeType::Integer,
                }
            })
            .collect()
    }

    pub(super) fn lower_map_call(
        &mut self,
        name: &str,
        arguments: &[HirCallArgument],
        location: SourceLocation,
    ) -> Option<BytecodeType> {
        let kind = MapCallKind::from_name(name)?;
        let result = kind.result_type();
        let parameters = Self::map_parameter_types(kind, arguments);
        let Some(ExecutionBinding::Native(contract)) =
            self.context.host_registry.classification(name)
        else {
            self.invalid_user_call("MAP extension has no Native catalog binding", location);
            return Some(result);
        };
        let contract = *contract;
        if !kind.valid_parameters(&parameters) {
            self.invalid_user_call("invalid MAP extension overload", location);
            return Some(result);
        }
        let HirCallArgument::Value(first) = &arguments[0] else {
            self.invalid_user_call("MAP name must be a string value", location);
            return Some(result);
        };
        self.lower_expression(first, location);
        let begin = self.code.len();
        // Reuse the ordinary import/contract builder, replacing only its eager call instruction.
        self.emit_native_call(name, &parameters, Some(result), contract, location);
        let Ok(begin_token) = u32::try_from(begin) else {
            self.invalid_user_call("MAP call offset exceeds the bytecode format", location);
            return Some(result);
        };
        let import = self.code[begin].payload[..4].to_vec();
        self.code[begin] = EncodedInstruction::new(Opcode::BeginMapCall, import);
        let missing = self.code.len();
        self.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
        let mut disabled = None;
        if kind == MapCallKind::Values && arguments.len() > 1 {
            let enabled = if arguments.len() == 3 { 2 } else { 1 };
            let HirCallArgument::Value(value) = &arguments[enabled] else {
                self.invalid_user_call("MAP_VALUES mode must be a value", location);
                return Some(result);
            };
            self.lower_expression(value, location);
            self.emit(EncodedInstruction::new(Opcode::Dup, Vec::new()), location);
            disabled = Some(self.code.len());
            self.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
            if arguments.len() == 3 {
                let place = match &arguments[1] {
                    HirCallArgument::Place(place) => Some(place),
                    HirCallArgument::Value(value) => {
                        if let HirExprKind::Variable { place } = &value.kind {
                            Some(place)
                        } else {
                            None
                        }
                    }
                    HirCallArgument::Omitted => None,
                };
                let Some(key) = place
                    .and_then(|place| self.context.variable_keys.get(place.variable.0))
                    .copied()
                else {
                    self.invalid_user_call(
                        "MAP_VALUES output must be a string array token",
                        location,
                    );
                    return Some(result);
                };
                self.emit(opcode::variable(Opcode::MakePlace, key, 0, 0), location);
            }
        } else {
            for argument in &arguments[1..] {
                if let HirCallArgument::Value(value) = argument {
                    self.lower_expression(value, location);
                } else {
                    self.invalid_user_call("MAP string argument must be a value", location);
                    return Some(result);
                }
            }
        }
        self.emit(
            EncodedInstruction::new(Opcode::FinishMapCall, begin_token.to_le_bytes().to_vec()),
            location,
        );
        let end = self.code.len();
        self.emit(opcode::jump(Opcode::Jump, 0), location);
        if let Some(disabled) = disabled {
            self.patch_jump(disabled, self.code.len());
            self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
        }
        self.patch_jump(missing, self.code.len());
        self.emit(
            EncodedInstruction::new(Opcode::AbandonMapCall, begin_token.to_le_bytes().to_vec()),
            location,
        );
        self.patch_jump(end, self.code.len());
        Some(result)
    }
}
