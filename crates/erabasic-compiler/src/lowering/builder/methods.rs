//! Shared lazy user calls: resolve the signature before capturing retained actuals.

use erabasic_bytecode::{
    MethodResult, UserArgumentAdvance, UserArgumentSpec, UserCallMode, UserCallSpec,
};
use erabasic_hir::{
    HirArgument, HirCallArgument, HirExpr, HirExprKind, HirPlace, SemanticType, SourceLocation,
};

use super::super::{
    BytecodeType, CompilerDiagnostic, CompilerDiagnosticCode, EncodedInstruction, Opcode,
    bytecode_type, opcode,
};
use super::Builder;

impl Builder<'_> {
    pub(super) fn lower_expression_method_statement(
        &mut self,
        name: &str,
        arguments: &[HirArgument],
        location: SourceLocation,
    ) {
        let arguments = Self::method_statement_arguments(arguments, location);
        self.lower_expression_method(
            &arguments,
            if name == "GETMETHS" {
                MethodResult::String
            } else {
                MethodResult::Integer
            },
            location,
        );
    }

    pub(super) fn method_statement_arguments(
        arguments: &[HirArgument],
        location: SourceLocation,
    ) -> Vec<HirCallArgument> {
        arguments
            .iter()
            .map(|argument| match argument {
                HirArgument::Expression(expression)
                | HirArgument::MixedExpression { expression, .. } => {
                    HirCallArgument::Value(expression.clone())
                }
                HirArgument::Place(place) => HirCallArgument::Place(place.clone()),
                HirArgument::Omitted => HirCallArgument::Omitted,
                HirArgument::Formatted(value) => HirCallArgument::Value(HirExpr {
                    kind: HirExprKind::Formatted {
                        value: value.clone(),
                    },
                    value_type: SemanticType::String,
                    constant: None,
                    location,
                }),
                HirArgument::Raw(value) => HirCallArgument::Value(HirExpr {
                    kind: HirExprKind::String {
                        value: value.clone(),
                    },
                    value_type: SemanticType::String,
                    constant: None,
                    location,
                }),
            })
            .collect::<Vec<_>>()
    }

    pub(super) fn lower_expression_method(
        &mut self,
        arguments: &[HirCallArgument],
        result: MethodResult,
        location: SourceLocation,
    ) {
        let Some(name) = arguments
            .first()
            .filter(|arg| !matches!(arg, HirCallArgument::Omitted))
        else {
            self.invalid_user_call("expression method requires a target name", location);
            return;
        };
        let actuals = arguments.get(2..).unwrap_or_default();
        let fallback = arguments
            .get(1)
            .filter(|arg| !matches!(arg, HirCallArgument::Omitted));
        if self.lower_user_argument_value(name, location) != BytecodeType::String {
            self.invalid_user_call("expression method target name must be a string", location);
            return;
        }
        let Some((resolve, mut spec)) =
            self.lower_user_call_actuals(actuals, result.into(), fallback.is_some(), location)
        else {
            return;
        };
        if let Some(fallback) = fallback {
            let join = self.code.len();
            self.emit(opcode::jump(Opcode::Jump, 0), location);
            spec.missing_target = u32::try_from(self.code.len()).unwrap_or(u32::MAX);
            self.code[resolve] = opcode::resolve_user_call(&spec);
            self.emit(
                opcode::abandon_user_call(u32::try_from(resolve).unwrap_or(u32::MAX)),
                location,
            );
            if self.lower_user_argument_value(fallback, location) != result.bytecode_type() {
                self.invalid_user_call(
                    "expression method fallback has an incompatible type",
                    location,
                );
            }
            self.patch_jump(join, self.code.len());
        }
    }

    /// Target String is already on the stack. The caller patches the missing branch
    /// to an `AbandonUserCall` before the finished function is validated.
    pub(super) fn lower_user_call_actuals(
        &mut self,
        actuals: &[HirCallArgument],
        mode: UserCallMode,
        allow_missing: bool,
        location: SourceLocation,
    ) -> Option<(usize, UserCallSpec)> {
        if actuals.len() > usize::from(u16::MAX) {
            self.invalid_user_call("user call exceeds the argument slot limit", location);
            return None;
        }
        let Some(arguments) = actuals
            .iter()
            .map(|argument| self.user_argument_spec(argument))
            .collect::<Option<Vec<_>>>()
        else {
            self.invalid_user_call("user call has an invalid argument shape", location);
            return None;
        };
        let spec = UserCallSpec {
            mode,
            allow_missing,
            missing_target: 0,
            arguments,
        };
        let resolve_index = self.code.len();
        let resolve = u32::try_from(resolve_index).unwrap_or(u32::MAX);
        self.emit(opcode::resolve_user_call(&spec), location);
        for (index, argument) in actuals.iter().enumerate() {
            let slot = u16::try_from(index).expect("user-call slot count was checked");
            if matches!(spec.arguments[index], UserArgumentSpec::Omitted) {
                self.emit(
                    opcode::advance_user_argument(resolve, slot, UserArgumentAdvance::Omitted),
                    location,
                );
                continue;
            }
            let guard = self.code.len();
            self.emit(opcode::guard_user_argument(resolve, slot, 0), location);
            if let UserArgumentSpec::Variable(key) = &spec.arguments[index] {
                let select = self.code.len();
                self.emit(opcode::select_user_argument(resolve, slot, 0), location);
                // Value formals evaluate indices and read now, before later actuals.
                self.lower_user_argument_value(argument, location);
                self.emit(
                    opcode::capture_user_argument(resolve, slot, false),
                    location,
                );
                let value_join = self.code.len();
                self.emit(opcode::jump(Opcode::Jump, 0), location);
                self.code[select] = opcode::select_user_argument(
                    resolve,
                    slot,
                    u32::try_from(self.code.len()).unwrap_or(u32::MAX),
                );
                // Array REF retains the original variable; element indices are not evaluated.
                self.emit(opcode::variable(Opcode::MakePlace, *key, 0, 0), location);
                self.emit(opcode::capture_user_argument(resolve, slot, true), location);
                self.patch_jump(value_join, self.code.len());
            } else {
                self.lower_user_argument_value(argument, location);
                self.emit(
                    opcode::capture_user_argument(resolve, slot, false),
                    location,
                );
            }
            let retained_join = self.code.len();
            self.emit(opcode::jump(Opcode::Jump, 0), location);
            self.code[guard] = opcode::guard_user_argument(
                resolve,
                slot,
                u32::try_from(self.code.len()).unwrap_or(u32::MAX),
            );
            self.emit(
                opcode::advance_user_argument(resolve, slot, UserArgumentAdvance::Discarded),
                location,
            );
            self.patch_jump(retained_join, self.code.len());
        }
        self.emit(opcode::invoke_user_call(resolve), location);
        Some((resolve_index, spec))
    }

    fn user_argument_spec(&self, argument: &HirCallArgument) -> Option<UserArgumentSpec> {
        if let Some(place) = method_argument_place(argument) {
            return self
                .context
                .variable_keys
                .get(place.variable.0)
                .copied()
                .map(UserArgumentSpec::Variable);
        }
        match argument {
            HirCallArgument::Omitted => Some(UserArgumentSpec::Omitted),
            HirCallArgument::Value(value) => {
                bytecode_type(value.value_type).map(UserArgumentSpec::Value)
            }
            HirCallArgument::Place(_) => None,
        }
    }

    fn lower_user_argument_value(
        &mut self,
        argument: &HirCallArgument,
        location: SourceLocation,
    ) -> BytecodeType {
        match argument {
            HirCallArgument::Value(value) => self.lower_expression(value, location),
            HirCallArgument::Place(place) => self.lower_expression(
                &HirExpr {
                    kind: HirExprKind::Variable {
                        place: place.clone(),
                    },
                    value_type: place.value_type,
                    constant: None,
                    location: place.location,
                },
                location,
            ),
            HirCallArgument::Omitted => unreachable!("omitted method slots never evaluate a value"),
        }
    }

    pub(super) fn invalid_user_call(&mut self, message: &str, location: SourceLocation) {
        self.diagnostics.push(CompilerDiagnostic::at(
            CompilerDiagnosticCode::InvalidHir,
            location,
            message,
        ));
        self.emit(
            EncodedInstruction::new(Opcode::Trap, message.as_bytes().to_vec()),
            location,
        );
    }
}

fn method_argument_place(argument: &HirCallArgument) -> Option<&HirPlace> {
    match argument {
        HirCallArgument::Place(place)
        | HirCallArgument::Value(HirExpr {
            kind: HirExprKind::Variable { place },
            ..
        }) => Some(place),
        HirCallArgument::Value(_) | HirCallArgument::Omitted => None,
    }
}
