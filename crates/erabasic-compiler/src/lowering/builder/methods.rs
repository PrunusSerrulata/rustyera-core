//! Lazy expression-method calls: resolve the signature before capturing arguments.

use erabasic_bytecode::{MethodArgumentSpec, MethodCallSpec, MethodResult};
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
        let arguments = arguments
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
            .collect::<Vec<_>>();
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
            self.invalid_method_call("expression method requires a target name", location);
            return;
        };
        let actuals = arguments.get(2..).unwrap_or_default();
        if actuals.len() > usize::from(u16::MAX) {
            self.invalid_method_call(
                "expression method exceeds the argument slot limit",
                location,
            );
            return;
        }
        let specs = actuals
            .iter()
            .map(|argument| self.method_argument_spec(argument))
            .collect::<Option<Vec<_>>>();
        let Some(specs) = specs else {
            self.invalid_method_call("expression method has an invalid argument shape", location);
            return;
        };
        let fallback = arguments
            .get(1)
            .filter(|arg| !matches!(arg, HirCallArgument::Omitted));
        let mut spec = MethodCallSpec {
            result,
            allow_missing: fallback.is_some(),
            missing_target: 0,
            arguments: specs,
        };
        if self.lower_method_value(name, location) != BytecodeType::String {
            self.invalid_method_call("expression method target name must be a string", location);
            return;
        }
        let resolve_index = self.code.len();
        let resolve = u32::try_from(resolve_index).unwrap_or(u32::MAX);
        self.emit(opcode::resolve_method(&spec), location);
        for (slot, argument) in actuals.iter().enumerate() {
            let slot = u16::try_from(slot).expect("method slot count was checked");
            match &spec.arguments[usize::from(slot)] {
                MethodArgumentSpec::Omitted => {}
                MethodArgumentSpec::Value(_) => {
                    self.lower_method_value(argument, location);
                    self.emit(
                        opcode::capture_method_argument(resolve, slot, false),
                        location,
                    );
                }
                MethodArgumentSpec::Variable(key) => {
                    let select = self.code.len();
                    self.emit(opcode::select_method_argument(resolve, slot, 0), location);
                    // A value formal observes indices and the value now, before later actuals.
                    self.lower_method_value(argument, location);
                    self.emit(
                        opcode::capture_method_argument(resolve, slot, false),
                        location,
                    );
                    let join = self.code.len();
                    self.emit(opcode::jump(Opcode::Jump, 0), location);
                    self.code[select] = opcode::select_method_argument(
                        resolve,
                        slot,
                        u32::try_from(self.code.len()).unwrap_or(u32::MAX),
                    );
                    // Existing array REF semantics ignore element indices entirely.
                    self.emit(opcode::variable(Opcode::MakePlace, *key, 0, 0), location);
                    self.emit(
                        opcode::capture_method_argument(resolve, slot, true),
                        location,
                    );
                    self.patch_jump(join, self.code.len());
                }
            }
        }
        self.emit(opcode::invoke_method(resolve), location);
        if let Some(fallback) = fallback {
            let join = self.code.len();
            self.emit(opcode::jump(Opcode::Jump, 0), location);
            spec.missing_target = u32::try_from(self.code.len()).unwrap_or(u32::MAX);
            self.code[resolve_index] = opcode::resolve_method(&spec);
            self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
            if self.lower_method_value(fallback, location) != result.bytecode_type() {
                self.invalid_method_call(
                    "expression method fallback has an incompatible type",
                    location,
                );
            }
            self.patch_jump(join, self.code.len());
        }
    }

    fn method_argument_spec(&self, argument: &HirCallArgument) -> Option<MethodArgumentSpec> {
        if let Some(place) = method_argument_place(argument) {
            return self
                .context
                .variable_keys
                .get(place.variable.0)
                .copied()
                .map(MethodArgumentSpec::Variable);
        }
        match argument {
            HirCallArgument::Omitted => Some(MethodArgumentSpec::Omitted),
            HirCallArgument::Value(value) => {
                bytecode_type(value.value_type).map(MethodArgumentSpec::Value)
            }
            HirCallArgument::Place(_) => None,
        }
    }

    fn lower_method_value(
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

    fn invalid_method_call(&mut self, message: &str, location: SourceLocation) {
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
