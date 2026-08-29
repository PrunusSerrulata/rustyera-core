//! Bounded source-only type and shape analysis shared by runtime expressions.
use super::{
    BinaryOp, BytecodeFunctionKind, BytecodeType, Expr, ExprKind, FormPart, FormattedString,
    GenerationId, MAX_RUNTIME_FORM_NESTING, NativeServiceRegistry, StepError, SymbolKey, UnaryOp,
    VmFaultCode, map_vm_error, methods, resource_limit, support, unsupported,
};
use crate::state::user_calls::resolve_user_call;
use erabasic_bytecode::{RuntimeExpressionShape, UserArgumentSpec, UserCallMode, UserCallSpec};

#[cfg(test)]
thread_local! { static TYPE_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

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
    probe: bool,
    natives: Option<&'a NativeServiceRegistry>,
    nodes: usize,
    limit: usize,
    pub(super) reference_terms: bool,
    pub(super) source_types: Vec<(erabasic_ast::Span, BytecodeType)>,
    pub(super) bound_calls: Vec<(erabasic_ast::Span, super::call_plan::RuntimeBoundCall)>,
    pub(super) expression_types: std::collections::BTreeMap<usize, BytecodeType>,
}

impl<'a> TypeAnalysis<'a> {
    pub(super) fn new(
        program: &'a crate::ProgramGeneration,
        function: SymbolKey,
        generation: GenerationId,
        probe: bool,
        limit: usize,
        natives: Option<&'a NativeServiceRegistry>,
    ) -> Self {
        Self {
            program,
            function,
            generation,
            probe,
            natives,
            nodes: 0,
            limit,
            reference_terms: false,
            source_types: Vec::new(),
            bound_calls: Vec::new(),
            expression_types: std::collections::BTreeMap::new(),
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
        #[cfg(test)]
        TYPE_VISITS.with(|visits| visits.set(visits.get() + 1));
        Ok(())
    }

    pub(super) fn expression(
        &mut self,
        expression: &Expr,
        depth: usize,
    ) -> Result<BytecodeType, StepError> {
        self.visit(depth)?;
        let result = match &expression.kind {
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
                if self.probe {
                    probe_variable_shape(self.program, definition, indices)?;
                }
                for index in indices {
                    let kind = self.expression(index, depth + 1)?;
                    if kind != BytecodeType::Integer
                        && !(self.probe && kind == BytecodeType::String)
                    {
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
                let shapes = self.arguments(args, depth + 1)?;
                self.call(name, args, &shapes, expression.span)
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
        }?;
        self.source_types.push((expression.span, result));
        if self.reference_terms {
            self.expression_types
                .insert(std::ptr::from_ref(expression) as usize, result);
        }
        Ok(result)
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
        if !self.probe {
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

    #[allow(clippy::too_many_lines)] // Ordered callable resolution is intentionally one dispatch chain.
    fn call(
        &mut self,
        name: &str,
        args: &[Option<Expr>],
        shapes: &[Option<RuntimeExpressionShape>],
        span: erabasic_ast::Span,
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
        if let Some(operation) = erabasic_bytecode::BitOperation::from_name(name) {
            if !self.probe {
                super::staged_binding::authorize(
                    self.program,
                    name,
                    erabasic_bytecode::RuntimeStagedKind::Bit(operation),
                    shapes,
                )?;
            }
            let definition = args
                .first()
                .and_then(Option::as_ref)
                .and_then(|expression| variable(self.program, self.function, expression));
            let spec = super::bit_calls::validate_shapes(operation, definition, shapes)?;
            if !self.probe {
                self.bound_calls
                    .push((span, super::call_plan::RuntimeBoundCall::Bit(spec)));
                return Ok(BytecodeType::Integer);
            }
        }
        if super::matching::is_match(name) {
            if !self.probe {
                super::staged_binding::authorize(
                    self.program,
                    name,
                    erabasic_bytecode::RuntimeStagedKind::from_name(name)
                        .expect("recognized MATCH"),
                    shapes,
                )?;
            }
            let types = shapes
                .iter()
                .map(|shape| shape.as_ref().map(|shape| shape.value_type))
                .collect::<Vec<_>>();
            let spec =
                super::matching::match_spec(self.program, self.function, name, args, &types)?;
            if !self.probe {
                self.bound_calls
                    .push((span, super::call_plan::RuntimeBoundCall::Match(spec)));
                return Ok(BytecodeType::Integer);
            }
        }
        if let Some(kind) = erabasic_bytecode::MapCallKind::from_name(name) {
            let output = args
                .get(1)
                .and_then(Option::as_ref)
                .and_then(|expression| variable(self.program, self.function, expression));
            super::map_calls::validate_map_output_definition(kind, args.len(), output)?;
            if !self.probe {
                if !self
                    .program
                    .artifact
                    .manifest
                    .compatibility
                    .supports_map_extensions()
                {
                    return Err(support::permission_denied(
                        "MAP extensions are unavailable in this identity",
                    ));
                }
                let bound = super::native_binding::bind(self.program, name, shapes, self.natives)?;
                if !bound.omitted_arguments.is_empty()
                    || args.iter().any(Option::is_none)
                    || !kind.valid_parameters(&bound.import.parameters)
                {
                    return Err(bad_type("MAP extension source overload differs"));
                }
                self.bound_calls
                    .push((span, super::call_plan::RuntimeBoundCall::Native(bound)));
                return Ok(kind.result_type());
            }
        }
        if super::input_host::allowed(name) {
            if !self
                .program
                .artifact
                .manifest
                .compatibility
                .supports_snake_input()
            {
                return Err(support::permission_denied(
                    "snake input form API unavailable",
                ));
            }
            let types = shapes
                .iter()
                .map(|shape| shape.as_ref().map(|shape| shape.value_type))
                .collect::<Vec<_>>();
            let valid = self.program.artifact.host_imports.iter().any(|host| {
                host.import.namespace == "rustyera.input"
                    && host.import.name.eq_ignore_ascii_case(name)
                    && host.import.result == Some(BytecodeType::Integer)
                    && host.import.parameters.len() == types.len()
                    && host
                        .import
                        .parameters
                        .iter()
                        .zip(&types)
                        .all(|(expected, actual)| Some(*expected) == *actual)
            });
            return if valid {
                Ok(BytecodeType::Integer)
            } else {
                Err(bad_type("input form callable signature differs"))
            };
        }
        if self.probe {
            let symbol = self
                .program
                .artifact
                .runtime_builtins
                .iter()
                .find(|symbol| symbol.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| missing(name))?;
            return if symbol.shapes.iter().any(|shape| shape.accepts(shapes)) {
                Ok(symbol.result)
            } else {
                Err(bad_type(format!(
                    "expression callable {name} has incompatible arguments"
                )))
            };
        }
        if self
            .program
            .artifact
            .runtime_host_authorizations
            .iter()
            .any(|family| family.name.eq_ignore_ascii_case(name))
        {
            super::host_calls::validate_source_tokens(self.program, self.function, name, args)?;
            let bound = super::host_calls::bind(self.program, name, shapes)?;
            let result = bound.import.result.expect("Host source result is scalar");
            self.bound_calls
                .push((span, super::call_plan::RuntimeBoundCall::Host(bound)));
            return Ok(result);
        }
        self.normal_call(name, shapes, span)
    }

    fn normal_call(
        &mut self,
        name: &str,
        shapes: &[Option<RuntimeExpressionShape>],
        span: erabasic_ast::Span,
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
            super::native_binding::authorization(self.program, name)?;
            if types.len() != 1 {
                return Err(bad_type("EXISTMETH expects one String"));
            }
            require_type(types[0], BytecodeType::String)?;
            return Ok(BytecodeType::Integer);
        }
        if name.eq_ignore_ascii_case("EXISTVAR") {
            let max = if self
                .program
                .artifact
                .manifest
                .compatibility
                .supports_existvar_expression_probe()
            {
                2
            } else {
                1
            };
            if types.is_empty() || types.len() > max {
                return Err(bad_type(
                    "EXISTVAR expects String and optional Integer mode",
                ));
            }
            require_type(types[0], BytecodeType::String)?;
            if let Some(Some(mode)) = types.get(1) {
                require_type(Some(*mode), BytecodeType::Integer)?;
            }
            return Ok(BytecodeType::Integer);
        }
        if name.eq_ignore_ascii_case("STRFORM") || name.eq_ignore_ascii_case("STRFORMCHECK") {
            let family = super::native_binding::authorization(self.program, name)?;
            if let Some(natives) = self.natives {
                super::native_binding::require_provider(natives, family)?;
            }
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
        self.native_call(name, shapes, span)
    }

    fn native_call(
        &mut self,
        name: &str,
        shapes: &[Option<RuntimeExpressionShape>],
        span: erabasic_ast::Span,
    ) -> Result<BytecodeType, StepError> {
        let bound = super::native_binding::bind(self.program, name, shapes, self.natives)?;
        let result = bound.import.result.expect("scalar Native result");
        self.bound_calls
            .push((span, super::call_plan::RuntimeBoundCall::Native(bound)));
        Ok(result)
    }
    pub(super) fn arguments(
        &mut self,
        args: &[Option<Expr>],
        depth: usize,
    ) -> Result<Vec<Option<RuntimeExpressionShape>>, StepError> {
        if args.len() > 65_535 || args.len() > self.limit.saturating_sub(self.nodes) {
            return Err(resource_limit(
                "runtime Native source arguments exceed operand limit",
            ));
        }
        args.iter()
            .map(|argument| {
                argument
                    .as_ref()
                    .map(|expression| {
                        let value_type = self.expression(expression, depth)?;
                        Ok(source_shape(
                            self.program,
                            self.function,
                            expression,
                            value_type,
                        ))
                    })
                    .transpose()
            })
            .collect()
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

fn probe_variable_shape(
    program: &crate::ProgramGeneration,
    definition: &erabasic_bytecode::BytecodeGlobal,
    indices: &[Expr],
) -> Result<(), StepError> {
    let rank = definition.dimensions.len();
    let character = definition.storage == erabasic_bytecode::BytecodeStorage::Character;
    let policy = program.artifact.call_compatibility;
    let valid = if character && rank == 1 {
        indices.is_empty() || indices.len() == 2 || (indices.len() == 1 && !policy.system_no_target)
    } else if rank >= 2 {
        indices.is_empty() || indices.len() == rank + usize::from(character)
    } else {
        indices.len() <= rank + usize::from(character)
    };
    if !valid {
        return Err(bad_type(
            "expression variable index count differs from its schema",
        ));
    }
    // The reference rejects omitted/literal zero RAND while constructing VariableTerm,
    // before any Restructure or value access. Unary plus/group preserve SingleLongTerm;
    // unary minus and binary expressions do not, so they must not be folded here.
    if definition.storage == erabasic_bytecode::BytecodeStorage::Calculated
        && definition.name.eq_ignore_ascii_case("RAND")
        && !policy.compatible_rand
        && (indices.is_empty() || indices.first().is_some_and(probe_literal_zero))
    {
        return Err(bad_type("RAND argument is omitted or literal zero"));
    }
    Ok(())
}
fn probe_literal_zero(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::Integer(0) => true,
        ExprKind::Group(inner)
        | ExprKind::Unary {
            op: UnaryOp::Plus,
            operand: inner,
        } => probe_literal_zero(inner),
        _ => false,
    }
}

pub(super) fn source_shape(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    expression: &Expr,
    value_type: BytecodeType,
) -> RuntimeExpressionShape {
    let definition = variable(program, function, expression);
    RuntimeExpressionShape {
        value_type,
        variable: definition.is_some(),
        mutable: definition.is_some_and(|variable| variable.mutable),
    }
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

    fn program_source(source: &str) -> crate::ProgramGeneration {
        let mut options = AnalyzerOptions::analysis_mode();
        options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        );
        let analysis = analyze_project(
            AnalysisInput {
                project_data: load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
                    .data
                    .unwrap(),
                sources: vec![ProjectSource {
                    relative_path: "typing.erb".into(),
                    payload: SourcePayload::Utf8(source.into()),
                }],
            },
            &options,
            &ExtensionRegistry::default(),
        );
        let compiled = compile_project(
            &analysis.project.expect("source analysis"),
            &CompilerOptions::default(),
            &default_host_registry(),
            None,
        );
        crate::ProgramGeneration::new(Arc::new(compiled.artifact.expect("compiled source")))
    }

    fn program() -> crate::ProgramGeneration {
        program_source(
            "@SYSTEM_TITLE\nRESULT = ABS(FLAG)\nRETURN\n@ECHO(ARG)\n#FUNCTION\nRETURNF ARG\n",
        )
    }

    #[test]
    fn runtime_nested_native_and_user_calls_consume_one_root_type_analysis() {
        struct NoHost;
        impl crate::VmHost for NoHost {
            fn call(&mut self, _: crate::HostCallRequest) -> crate::HostCallResult {
                panic!("unexpected Host service")
            }
        }
        for pattern in [0, 1, 2] {
            let expression = (0..24).fold("1".to_owned(), |inner, index| {
                let name = if pattern == 0 || pattern == 2 && index % 2 == 0 {
                    "ABS"
                } else {
                    "ECHO"
                };
                format!("{name}({inner})")
            });
            let program = program_source(&format!(
                "@SYSTEM_TITLE\nRESULTS '= STRFORM(\"{{{expression}}}\")\nRETURN\n@ECHO(ARG)\n#FUNCTION\nRETURNF ARG\n"
            ));
            let entry = program.function_by_name("SYSTEM_TITLE").unwrap().key;
            let context = erabasic_compiler::runtime_native_validation_context(
                &program.artifact,
                &default_host_registry(),
            );
            let validated = erabasic_validator::validate_bytecode(
                (*program.artifact).clone().into_unvalidated(),
                &context,
            )
            .value
            .unwrap();
            let mut vm = crate::Vm::new(validated, crate::VmConfig::default());
            let mut natives = NativeServiceRegistry::for_artifact(&program.artifact);
            vm.spawn_entry(entry, Vec::new()).unwrap();
            TYPE_VISITS.with(|visits| visits.set(0));
            let report = vm.run_slice(&mut NoHost, &mut natives, crate::RunBudget::default());
            assert!(
                report
                    .events
                    .iter()
                    .any(|event| matches!(event, crate::VmEvent::FiberCompleted { .. })),
                "{report:?}"
            );
            // Form + interpolation + 24 calls + one literal. No call or return retypes its actual tree.
            assert_eq!(
                TYPE_VISITS.with(std::cell::Cell::get),
                27,
                "pattern {pattern}"
            );
        }
    }

    #[test]
    fn nested_types_visit_each_node_once_in_normal_and_probe_modes() {
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
            for probe in [false, true] {
                let mut analysis = TypeAnalysis::new(
                    &program,
                    function,
                    GenerationId::default(),
                    probe,
                    25,
                    Some(&natives),
                );
                assert_eq!(
                    analysis.expression(&expression, 0).unwrap(),
                    BytecodeType::Integer
                );
                assert_eq!(analysis.nodes(), 25, "{probe}: {source}");
                let mut bounded = TypeAnalysis::new(
                    &program,
                    function,
                    GenerationId::default(),
                    probe,
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
