use std::collections::BTreeSet;

use erabasic_data::ProjectData;
use erabasic_hir::{
    CallTarget, HIR_FORMAT_VERSION, HirCallArgument, HirExpr, HirExprKind, HirPlace,
    HirStatementKind, InstructionTarget, Program, SemanticType,
};

use crate::{ValidationCode, ValidationDiagnostic, ValidationReport};

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn validate_hir(program: &Program, _data: &ProjectData) -> ValidationReport<()> {
    let mut diagnostics = Vec::new();
    if let Err(error) = program.compatibility.validate() {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::UnsupportedVersion,
            format!("unsupported HIR compatibility: {error}"),
        ));
    }
    if program.format_version != HIR_FORMAT_VERSION {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::UnsupportedVersion,
            format!("unsupported HIR format {}", program.format_version),
        ));
    }
    let source_ids: BTreeSet<_> = program.sources.iter().map(|source| source.id).collect();
    if source_ids.len() != program.sources.len() {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::DuplicateIdentity,
            "source IDs are not unique",
        ));
    }
    for source in &program.sources {
        if source.line_starts.first() != Some(&0)
            || !source.line_starts.windows(2).all(|pair| pair[0] < pair[1])
            || source
                .line_starts
                .last()
                .is_some_and(|offset| *offset > source.byte_len)
        {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::InvalidSourceMap,
                format!("source {} has an invalid line table", source.relative_path),
            ));
        }
    }
    let variable_ids: BTreeSet<_> = program
        .variables
        .iter()
        .map(|variable| variable.id)
        .collect();
    if variable_ids.len() != program.variables.len() {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::DuplicateIdentity,
            "variable IDs are not unique",
        ));
    }
    for variable in &program.variables {
        if (variable.reference_semantics.can_restructure && !variable.reference_semantics.is_const)
            || (variable.reference && variable.reference_semantics.is_const)
        {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::InvalidHir,
                format!(
                    "variable {} has inconsistent reference token semantics",
                    variable.name
                ),
            ));
        }
    }
    let function_ids: BTreeSet<_> = program
        .functions
        .iter()
        .map(|function| function.id)
        .collect();
    if function_ids.len() != program.functions.len() {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::DuplicateIdentity,
            "function IDs are not unique",
        ));
    }
    for function in &program.functions {
        if !source_ids.contains(&function.location.source) {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::MissingReference,
                format!("function {} refers to an unknown source", function.name),
            ));
        }
        let line_ids: BTreeSet<_> = function.lines.iter().map(|line| line.id).collect();
        for (index, line) in function.lines.iter().enumerate() {
            if !source_ids.contains(&line.location.source) {
                diagnostics.push(ValidationDiagnostic::instruction(
                    ValidationCode::MissingReference,
                    &function.name,
                    index,
                    "line refers to an unknown source",
                ));
            }
            match &line.kind {
                HirStatementKind::Error => diagnostics.push(ValidationDiagnostic::instruction(
                    ValidationCode::InvalidHir,
                    &function.name,
                    index,
                    "error statement cannot be compiled",
                )),
                HirStatementKind::Instruction {
                    target: InstructionTarget::Unresolved(name),
                    ..
                } => diagnostics.push(ValidationDiagnostic::instruction(
                    ValidationCode::MissingReference,
                    &function.name,
                    index,
                    format!("unresolved instruction {name}"),
                )),
                HirStatementKind::Assignment { target, value, .. } => {
                    validate_expression(
                        value,
                        &function_ids,
                        &variable_ids,
                        &function.name,
                        index,
                        &mut diagnostics,
                    );
                    if !variable_ids.contains(&target.variable) {
                        diagnostics.push(ValidationDiagnostic::instruction(
                            ValidationCode::MissingReference,
                            &function.name,
                            index,
                            "assignment refers to an unknown variable",
                        ));
                    }
                }
                HirStatementKind::Instruction { arguments, .. } => {
                    for argument in arguments {
                        match argument {
                            erabasic_hir::HirArgument::Expression(expression)
                            | erabasic_hir::HirArgument::MixedExpression { expression, .. } => {
                                validate_expression(
                                    expression,
                                    &function_ids,
                                    &variable_ids,
                                    &function.name,
                                    index,
                                    &mut diagnostics,
                                );
                            }
                            erabasic_hir::HirArgument::Place(place) => {
                                if !variable_ids.contains(&place.variable) {
                                    diagnostics.push(ValidationDiagnostic::instruction(
                                        ValidationCode::MissingReference,
                                        &function.name,
                                        index,
                                        "place argument refers to an unknown variable",
                                    ));
                                }
                                for expression in &place.indices {
                                    validate_expression(
                                        expression,
                                        &function_ids,
                                        &variable_ids,
                                        &function.name,
                                        index,
                                        &mut diagnostics,
                                    );
                                }
                            }
                            erabasic_hir::HirArgument::Formatted(_)
                            | erabasic_hir::HirArgument::Raw(_)
                            | erabasic_hir::HirArgument::Omitted => {}
                        }
                    }
                }
                HirStatementKind::Label { .. } => {}
            }
        }
        for edge in &function.control_flow {
            if !line_ids.contains(&edge.from)
                || edge.to.is_some_and(|line| !line_ids.contains(&line))
                || edge
                    .function
                    .is_some_and(|target| !function_ids.contains(&target))
            {
                diagnostics.push(ValidationDiagnostic::project(
                    ValidationCode::InvalidControlFlow,
                    format!(
                        "function {} has a dangling control-flow edge",
                        function.name
                    ),
                ));
            }
        }
    }
    ValidationReport {
        value: diagnostics.is_empty().then_some(()),
        diagnostics,
    }
}

fn validate_expression(
    expression: &HirExpr,
    functions: &BTreeSet<erabasic_hir::FunctionId>,
    variables: &BTreeSet<erabasic_hir::VariableId>,
    function_name: &str,
    instruction: usize,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    ExpressionValidator {
        functions,
        variables,
        function_name,
        instruction,
        diagnostics,
    }
    .validate(expression);
}

struct ExpressionValidator<'a, 'd> {
    functions: &'a BTreeSet<erabasic_hir::FunctionId>,
    variables: &'a BTreeSet<erabasic_hir::VariableId>,
    function_name: &'a str,
    instruction: usize,
    diagnostics: &'d mut Vec<ValidationDiagnostic>,
}

impl ExpressionValidator<'_, '_> {
    fn validate(&mut self, expression: &HirExpr) {
        if expression.value_type == SemanticType::Error
            || matches!(expression.kind, HirExprKind::Error)
        {
            self.diagnostic(
                ValidationCode::InvalidHir,
                "error expression cannot be compiled",
            );
            return;
        }
        match &expression.kind {
            HirExprKind::Variable { place } => {
                self.validate_place(place, "expression refers to an unknown variable");
            }
            HirExprKind::Call { target, arguments } => {
                match target {
                    CallTarget::User { function } if !self.functions.contains(function) => {
                        self.diagnostic(
                            ValidationCode::MissingReference,
                            "call refers to an unknown function",
                        );
                    }
                    CallTarget::Unresolved { name } => self.diagnostic(
                        ValidationCode::MissingReference,
                        format!("unresolved function {name}"),
                    ),
                    _ => {}
                }
                for argument in arguments {
                    match argument {
                        HirCallArgument::Value(argument) => self.validate(argument),
                        HirCallArgument::Place(place) => {
                            self.validate_place(place, "call place refers to an unknown variable");
                        }
                        HirCallArgument::Omitted => {}
                    }
                }
            }
            HirExprKind::Unary { operand, .. } | HirExprKind::Postfix { operand, .. } => {
                self.validate(operand);
            }
            HirExprKind::Binary { left, right, .. } => {
                self.validate(left);
                self.validate(right);
            }
            HirExprKind::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                self.validate(condition);
                self.validate(then_expr);
                self.validate(else_expr);
            }
            HirExprKind::Integer { .. }
            | HirExprKind::String { .. }
            | HirExprKind::Formatted { .. }
            | HirExprKind::Error => {}
        }
    }

    fn validate_place(&mut self, place: &HirPlace, missing_message: &'static str) {
        if !self.variables.contains(&place.variable) {
            self.diagnostic(ValidationCode::MissingReference, missing_message);
        }
        for index in &place.indices {
            self.validate(index);
        }
    }

    fn diagnostic(&mut self, code: ValidationCode, message: impl Into<String>) {
        self.diagnostics.push(ValidationDiagnostic::instruction(
            code,
            self.function_name,
            self.instruction,
            message,
        ));
    }
}
