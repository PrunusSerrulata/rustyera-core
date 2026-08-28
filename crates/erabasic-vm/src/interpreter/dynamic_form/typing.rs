//! Bounded source-only type and shape analysis shared by runtime expressions.
use super::{
    BinaryOp, BytecodeFunctionKind, BytecodeType, Expr, ExprKind, FormPart, FormattedString,
    GenerationId, MAX_RUNTIME_FORM_BYTES, MAX_RUNTIME_FORM_NESTING, NativeServiceRegistry,
    StepError, SymbolKey, UnaryOp, VmFaultCode, map_vm_error, methods, resource_limit, support,
    unsupported,
};
use crate::state::user_calls::resolve_user_call;
use erabasic_bytecode::{RuntimeExpressionShape, UserArgumentSpec, UserCallMode, UserCallSpec};

fn bad_type(message: impl Into<String>) -> StepError {
    StepError::script(
        crate::ScriptFaultKind::Argument,
        VmFaultCode::TypeMismatch,
        message,
    )
}

fn missing(name: &str) -> StepError {
    StepError::script(
        crate::ScriptFaultKind::Resolve,
        VmFaultCode::MissingSymbol,
        format!("runtime expression symbol {name} is missing"),
    )
}

/// This object lives for one immutable AST analysis. It never reads a storage
/// cell or retains execution results; every child is visited exactly once.
pub(super) struct TypeAnalysis<'a> {
    program: &'a crate::ProgramGeneration,
    function: SymbolKey,
    generation: GenerationId,
    natives: Option<&'a NativeServiceRegistry>,
    nodes: usize,
    limit: usize,
}

impl<'a> TypeAnalysis<'a> {
    pub(super) fn new(
        program: &'a crate::ProgramGeneration,
        function: SymbolKey,
        generation: GenerationId,
            limit: usize,
        natives: Option<&'a NativeServiceRegistry>,
    ) -> Self {
        Self {
            program,
            function,
            generation,
            natives,
            nodes: 0,
            limit,
        }
    }

    pub(super) const fn nodes(&self) -> usize {
        self.nodes
    }

    fn visit(&mut self, depth: usize) -> Result<(), StepError> {
        if depth > MAX_RUNTIME_FORM_NESTING {
            return Err(resource_limit(
                "runtime expression type nesting exceeds limit",
            ));
        }
        if self.nodes >= self.limit {
            return Err(resource_limit(
                "runtime expression AST exceeds the VM operand limit",
            ));
        }
        self.nodes += 1;
        Ok(())
    }

    pub(super) fn expression(
        &mut self,
        expression: &Expr,
        depth: usize,
    ) -> Result<BytecodeType, StepError> {
        self.visit(depth)?;
        match &expression.kind {
            ExprKind::Integer(_) => Ok(BytecodeType::Integer),
            ExprKind::String(_) => Ok(BytecodeType::String),
            ExprKind::Formatted(form) => {
                self.form(form, depth + 1)?;
                Ok(BytecodeType::String)
            }
            ExprKind::Identifier(name) | ExprKind::Variable { name, .. } => {
                let definition = self
                    .program
                    .scoped_variable(self.function, name)
                    .ok_or_else(|| missing(name))?;
                let indices = match &expression.kind {
                    ExprKind::Variable { indices, .. } => indices.as_slice(),
                    _ => &[],
                };
                for index in indices {
                    let kind = self.expression(index, depth + 1)?;
                    if kind != BytecodeType::Integer {
                        return Err(bad_type("runtime variable index must be an integer"));
                    }
                }
                Ok(definition.value_type)
            }
            ExprKind::Group(inner) => self.expression(inner, depth + 1),
            ExprKind::Unary { op, operand } => {
                let kind = self.expression(operand, depth + 1)?;
                if matches!(op, UnaryOp::PreIncrement | UnaryOp::PreDecrement) {
                    self.mutable_integer(operand)?;
                }
                require_type(Some(kind), BytecodeType::Integer)?;
                Ok(BytecodeType::Integer)
            }
            ExprKind::Postfix { operand, .. } => {
                let kind = self.expression(operand, depth + 1)?;
                self.mutable_integer(operand)?;
                require_type(Some(kind), BytecodeType::Integer)?;
                Ok(BytecodeType::Integer)
            }
            ExprKind::Call { name, args } => {
                let shapes = args
                    .iter()
                    .map(|argument| {
                        argument
                            .as_ref()
                            .map(|expression| {
                                let value_type = self.expression(expression, depth + 1)?;
                                let variable = variable(self.program, self.function, expression);
                                Ok(RuntimeExpressionShape {
                                    value_type,
                                    variable: variable.is_some(),
                                    mutable: variable.is_some_and(|definition| definition.mutable),
                                })
                            })
                            .transpose()
                    })
                    .collect::<Result<Vec<_>, StepError>>()?;
                self.call(name, args, &shapes)
            }
            ExprKind::Binary { op, left, right } => {
                let left = self.expression(left, depth + 1)?;
                let right = self.expression(right, depth + 1)?;
                binary_result_type(*op, left, right)
            }
            ExprKind::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                let condition = self.expression(condition, depth + 1)?;
                require_type(Some(condition), BytecodeType::Integer)?;
                let then_type = self.expression(then_expr, depth + 1)?;
                let else_type = self.expression(else_expr, depth + 1)?;
                require_type(Some(else_type), then_type)?;
                Ok(then_type)
            }
            ExprKind::Error => Err(bad_type("runtime expression contains a parser error")),
        }
    }

    pub(super) fn form(&mut self, form: &FormattedString, depth: usize) -> Result<(), StepError> {
        self.visit(depth)?;
        for part in &form.parts {
            self.visit(depth)?;
            match part {
                FormPart::Text(_) => {}
                FormPart::Triple { symbol, .. } => {
                    let names = match symbol {
                        '*' => ("NAME", "TARGET"),
                        '+' => ("CALLNAME", "MASTER"),
                        '=' => ("CALLNAME", "PLAYER"),
                        '/' => ("NAME", "ASSI"),
                        '$' => ("CALLNAME", "TARGET"),
                        _ => {
                            return Err(unsupported(format!(
                                "STRFORM triple symbol {symbol:?} is unsupported"
                            )));
                        }
                    };
                    for name in [names.0, names.1] {
                        if self.program.scoped_variable(self.function, name).is_none() {
                            return Err(missing(name));
                        }
                    }
                }
                FormPart::StringInterpolation {
                    expression, width, ..
                }
                | FormPart::IntegerInterpolation {
                    expression, width, ..
                } => {
                    let expected = if matches!(part, FormPart::StringInterpolation { .. }) {
                        BytecodeType::String
                    } else {
                        BytecodeType::Integer
                    };
                    require_type(Some(self.expression(expression, depth + 1)?), expected)?;
                    if let Some(width) = width {
                        require_type(
                            Some(self.expression(width, depth + 1)?),
                            BytecodeType::Integer,
                        )?;
                    }
                }
                FormPart::Conditional {
                    condition,
                    then_value,
                    else_value,
                    ..
                } => {
                    require_type(
                        Some(self.expression(condition, depth + 1)?),
                        BytecodeType::Integer,
                    )?;
                    self.form(then_value, depth + 1)?;
                    if let Some(else_value) = else_value {
                        self.form(else_value, depth + 1)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn mutable_integer(&self, expression: &Expr) -> Result<(), StepError> {
        let definition = variable(self.program, self.function, expression)
            .ok_or_else(|| bad_type("increment/decrement needs a variable"))?;
        if !definition.mutable || definition.value_type != BytecodeType::Integer {
            return Err(bad_type("increment/decrement needs a writable Integer"));
        }
        {
            let expression = ungroup(expression);
            if let ExprKind::Variable { indices, .. } = &expression.kind
                && indices.len()
                    > definition.dimensions.len()
                        + usize::from(
                            definition.storage == erabasic_bytecode::BytecodeStorage::Character,
                        )
            {
                return Err(bad_type("mutation index count exceeds variable rank"));
            }
        }
        Ok(())
    }

    fn call(
        &self,
        name: &str,
        args: &[Option<Expr>],
        shapes: &[Option<RuntimeExpressionShape>],
    ) -> Result<BytecodeType, StepError> {
        if let Some(target) = self.program.function_by_name(name) {
            if target.kind != BytecodeFunctionKind::Method {
                return Err(bad_type(format!("runtime target {name} is not a method")));
            }
            let result = target
                .result
                .ok_or_else(|| bad_type("runtime method has no scalar result"))?;
            let mode = match result {
                BytecodeType::Integer => UserCallMode::MethodInteger,
                BytecodeType::String => UserCallMode::MethodString,
                _ => return Err(bad_type("runtime method result is not scalar")),
            };
            let arguments = args
                .iter()
                .zip(shapes)
                .map(|(argument, shape)| {
                    shape.as_ref().map_or(UserArgumentSpec::Omitted, |shape| {
                        shape_spec(
                            self.program,
                            self.function,
                            argument.as_ref().expect("present shape"),
                            shape.value_type,
                        )
                    })
                })
                .collect();
            resolve_user_call(
                self.program,
                self.generation,
                name,
                &UserCallSpec {
                    mode,
                    allow_missing: false,
                    missing_target: 0,
                    arguments,
                },
            )
            .map_err(map_vm_error)?;
            return Ok(result);
        }
        self.normal_call(name, shapes)
    }

    fn normal_call(
        &self,
        name: &str,
        shapes: &[Option<RuntimeExpressionShape>],
    ) -> Result<BytecodeType, StepError> {
        let types = shapes
            .iter()
            .map(|shape| shape.as_ref().map(|shape| shape.value_type))
            .collect::<Vec<_>>();
        if let Some(result) = methods::method_result(name) {
            require_type(types.first().copied().flatten(), BytecodeType::String)?;
            if let Some(Some(fallback)) = types.get(1) {
                require_type(Some(*fallback), result.bytecode_type())?;
            }
            return Ok(result.bytecode_type());
        }
        if name.eq_ignore_ascii_case("EXISTMETH") {
            if types.len() != 1 {
                return Err(bad_type("EXISTMETH expects one String"));
            }
            require_type(types[0], BytecodeType::String)?;
            return Ok(BytecodeType::Integer);
        }
        if name.eq_ignore_ascii_case("STRFORM") || name.eq_ignore_ascii_case("STRFORMCHECK") {
            let checked = name.eq_ignore_ascii_case("STRFORMCHECK");
            if checked
                && !self
                    .program
                    .artifact
                    .manifest
                    .compatibility
                    .supports_checked_runtime_forms()
            {
                return Err(support::permission_denied(
                    "STRFORMCHECK is unavailable in this compatibility identity",
                ));
            }
            if types.len() != 1 {
                return Err(bad_type("formatted-string function expects one String"));
            }
            require_type(types[0], BytecodeType::String)?;
            return Ok(if checked {
                BytecodeType::Integer
            } else {
                BytecodeType::String
            });
        }
        self.native_call(name, &types)
    }

    fn native_call(
        &self,
        name: &str,
        types: &[Option<BytecodeType>],
    ) -> Result<BytecodeType, StepError> {
        if crate::structured::is_internal_column_native(name) {
            return Err(support::permission_denied(
                "STRFORM cannot invoke an internal column operation",
            ));
        }
        let actual = types
            .iter()
            .copied()
            .map(|kind| kind.ok_or_else(|| bad_type("builtin argument cannot be omitted")))
            .collect::<Result<Vec<_>, _>>()?;
        let import = self.program.artifact.native_imports.iter().find(|native| {
            native.import.name.eq_ignore_ascii_case(name)
                && native.import.parameters == actual
                && matches!(
                    native.import.result,
                    Some(BytecodeType::Integer | BytecodeType::String)
                )
        });
        if let Some(native) = import {
            if let Some(natives) = self.natives {
                require_native_provider(natives, &native.import)?;
            }
            return Ok(native.import.result.expect("scalar result selected"));
        }
        if self
            .program
            .artifact
            .host_imports
            .iter()
            .any(|host| host.import.name.eq_ignore_ascii_case(name))
        {
            return Err(support::permission_denied(format!(
                "STRFORM host callable {name} is unsupported in a template"
            )));
        }
        if self
            .program
            .artifact
            .native_imports
            .iter()
            .any(|native| native.import.name.eq_ignore_ascii_case(name))
        {
            return Err(bad_type(format!(
                "STRFORM builtin {name} has incompatible argument types or arity"
            )));
        }
        Err(missing(name))
    }
}

fn ungroup(mut expression: &Expr) -> &Expr {
    while let ExprKind::Group(inner) = &expression.kind {
        expression = inner;
    }
    expression
}

fn variable<'a>(
    program: &'a crate::ProgramGeneration,
    function: SymbolKey,
    expression: &Expr,
) -> Option<&'a erabasic_bytecode::BytecodeGlobal> {
    match &ungroup(expression).kind {
        ExprKind::Identifier(name) | ExprKind::Variable { name, .. } => {
            program.scoped_variable(function, name)
        }
        _ => None,
    }
}

pub(super) fn shape_spec(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    expression: &Expr,
    kind: BytecodeType,
) -> UserArgumentSpec {
    variable(program, function, expression).map_or(UserArgumentSpec::Value(kind), |variable| {
        UserArgumentSpec::Variable(variable.key)
    })
}

fn require_type(actual: Option<BytecodeType>, expected: BytecodeType) -> Result<(), StepError> {
    if actual != Some(expected) {
        return Err(bad_type(
            "runtime expression has an incompatible or omitted type",
        ));
    }
    Ok(())
}

pub(super) fn require_native_provider(
    natives: &NativeServiceRegistry,
    import: &erabasic_bytecode::RuntimeImport,
) -> Result<(), StepError> {
    if !natives.contains(import.key) {
        return Err(StepError::classified(
            crate::FaultCategory::HostContract,
            VmFaultCode::Native,
            format!("runtime native provider {} is missing", import.name),
        ));
    }
    Ok(())
}

pub(super) fn expression_type(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    expression: &Expr,
    depth: usize,
) -> Result<BytecodeType, StepError> {
    // Introspection returns only a type; this placeholder generation is never
    // retained or executed. Runtime entry points pass their actual generation.
    TypeAnalysis::new(
        program,
        function,
        GenerationId::default(),
        MAX_RUNTIME_FORM_BYTES,
        None,
    )
    .expression(expression, depth)
}

pub(super) fn argument_spec(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    expression: Option<&Expr>,
) -> Result<UserArgumentSpec, StepError> {
    let Some(expression) = expression else {
        return Ok(UserArgumentSpec::Omitted);
    };
    Ok(shape_spec(
        program,
        function,
        expression,
        expression_type(program, function, expression, 0)?,
    ))
}

fn binary_result_type(
    op: BinaryOp,
    left: BytecodeType,
    right: BytecodeType,
) -> Result<BytecodeType, StepError> {
    if matches!(
        op,
        BinaryOp::Less
            | BinaryOp::LessEqual
            | BinaryOp::Greater
            | BinaryOp::GreaterEqual
            | BinaryOp::Equal
            | BinaryOp::NotEqual
    ) {
        if left == right && matches!(left, BytecodeType::Integer | BytecodeType::String) {
            return Ok(BytecodeType::Integer);
        }
        return Err(bad_type(
            "STRFORM comparison operands must have the same scalar type",
        ));
    }
    if (op == BinaryOp::Add && left == BytecodeType::String && right == BytecodeType::String)
        || (op == BinaryOp::Multiply
            && matches!(
                (left, right),
                (BytecodeType::String, BytecodeType::Integer)
                    | (BytecodeType::Integer, BytecodeType::String)
            ))
    {
        return Ok(BytecodeType::String);
    }
    if left == BytecodeType::Integer && right == BytecodeType::Integer {
        return Ok(BytecodeType::Integer);
    }
    Err(bad_type("STRFORM binary operands must be integers"))
}


#[cfg(test)]
mod tests {
    use super::*;
    use erabasic_analyzer::{
        AnalysisInput, AnalyzerOptions, ExtensionRegistry, ProjectSource, SourcePayload,
        analyze_project,
    };
    use erabasic_compiler::{CompilerOptions, compile_project, default_host_registry};
    use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};
    use erabasic_parser::DefaultParserContext;
    use std::sync::Arc;

    fn program() -> crate::ProgramGeneration {
        let mut options = AnalyzerOptions::analysis_mode();
        options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        );
        let analysis = analyze_project(AnalysisInput {
            project_data: load_project(&ProjectFiles::default(), &CsvLoadOptions::default()).data.unwrap(),
            sources: vec![ProjectSource { relative_path: "typing.erb".into(), payload: SourcePayload::Utf8(
                "@SYSTEM_TITLE\nRESULT = ABS(FLAG)\nRETURN\n@ECHO(ARG)\n#FUNCTION\nRETURNF ARG\n".into()) }],
        }, &options, &ExtensionRegistry::default());
        let compiled = compile_project(
            &analysis.project.expect("source analysis"),
            &CompilerOptions::default(),
            &default_host_registry(),
            None,
        );
        crate::ProgramGeneration::new(Arc::new(compiled.artifact.expect("compiled source")))
    }

    #[test]
    fn nested_types_visit_each_node_once_in_normal_mode() {
        let program = program();
        let function = program.function_by_name("SYSTEM_TITLE").unwrap().key;
        let natives = NativeServiceRegistry::for_artifact(&program.artifact);
        for names in [
            vec!["ABS"; 24],
            vec!["ECHO"; 24],
            (0..24)
                .map(|i| if i % 2 == 0 { "ABS" } else { "ECHO" })
                .collect(),
        ] {
            let source = names
                .iter()
                .rev()
                .fold("1".to_owned(), |inner, name| format!("{name}({inner})"));
            let parsed =
                erabasic_parser::parse_expression(&source, &DefaultParserContext::default());
            let expression = parsed.value.expect("nested expression");
            {
                let mut analysis = TypeAnalysis::new(
                    &program,
                    function,
                    GenerationId::default(),
                            25,
                    Some(&natives),
                );
                assert_eq!(
                    analysis.expression(&expression, 0).unwrap(),
                    BytecodeType::Integer
                );
                assert_eq!(analysis.nodes(), 25, "{source}");
                let mut bounded = TypeAnalysis::new(
                    &program,
                    function,
                    GenerationId::default(),
                            24,
                    Some(&natives),
                );
                assert_eq!(
                    bounded.expression(&expression, 0).unwrap_err().category,
                    crate::FaultCategory::ResourceLimit
                );
                assert_eq!(bounded.nodes(), 24);
            }
        }
    }
}
