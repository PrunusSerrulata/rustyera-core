#[allow(clippy::wildcard_imports)]
use super::*;
use erabasic_bytecode::BytecodeStorage;
pub(super) fn parse_runtime_form(
    vm: &Vm,
    natives: &NativeServiceRegistry,
    generation: GenerationId,
    function: SymbolKey,
    source: &str,
    node_limit: usize,
) -> Result<(FormattedString, usize), StepError> {
    if source.len() > MAX_RUNTIME_FORM_BYTES {
        return Err(resource_limit(
            "STRFORM source exceeds the runtime parser limit",
        ));
    }
    preflight_nesting(source)?;
    let program = vm.generations.get(&generation).ok_or_else(|| {
        StepError::new(VmFaultCode::MissingSymbol, "STRFORM generation is missing")
    })?;
    let compatibility = program.artifact.call_compatibility;
    let mut context = DefaultParserContext::default();
    context.set_lexer_compatibility(
        compatibility.allow_full_width_space,
        compatibility.debug_semicolon,
        compatibility.ignore_triple_symbols,
    );
    let parsed = parse_formatted_at(source, 0, &context);
    if parsed.has_errors() {
        let message = parsed
            .diagnostics
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(StepError::new(
            VmFaultCode::Native,
            format!("STRFORM expansion failed: {message}"),
        ));
    }
    let mut formatted = parsed.value.ok_or_else(|| {
        StepError::new(
            VmFaultCode::Native,
            "STRFORM expansion produced no formatted string",
        )
    })?;
    resolve_named_indices(program, function, &mut formatted, 0)?;
    let nodes = validate_form(vm, natives, generation, function, &formatted, node_limit)?;
    Ok((formatted, nodes))
}

fn resolve_named_indices(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    formatted: &mut FormattedString,
    depth: usize,
) -> Result<(), StepError> {
    if depth > MAX_RUNTIME_FORM_NESTING {
        return Err(resource_limit(
            "STRFORM named-index resolution exceeds the nesting limit",
        ));
    }
    for part in &mut formatted.parts {
        match part {
            FormPart::Text(_) | FormPart::Triple { .. } => {}
            FormPart::StringInterpolation {
                expression, width, ..
            }
            | FormPart::IntegerInterpolation {
                expression, width, ..
            } => {
                resolve_expression_named_indices(program, function, expression, depth + 1)?;
                if let Some(width) = width {
                    resolve_expression_named_indices(program, function, width, depth + 1)?;
                }
            }
            FormPart::Conditional {
                condition,
                then_value,
                else_value,
                ..
            } => {
                resolve_expression_named_indices(program, function, condition, depth + 1)?;
                resolve_named_indices(program, function, then_value, depth + 1)?;
                if let Some(else_value) = else_value {
                    resolve_named_indices(program, function, else_value, depth + 1)?;
                }
            }
        }
    }
    Ok(())
}

fn resolve_expression_named_indices(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    expression: &mut Expr,
    depth: usize,
) -> Result<(), StepError> {
    if depth > MAX_RUNTIME_FORM_NESTING {
        return Err(resource_limit(
            "STRFORM named-index resolution exceeds the nesting limit",
        ));
    }
    match &mut expression.kind {
        ExprKind::Integer(_) | ExprKind::String(_) | ExprKind::Identifier(_) | ExprKind::Error => {}
        ExprKind::Variable { name, indices } => {
            for index in indices.iter_mut() {
                resolve_expression_named_indices(program, function, index, depth + 1)?;
            }
            let Some(definition) = program.scoped_variable(function, name) else {
                return Ok(());
            };
            let dimensions = definition.dimensions.len();
            let explicit_character =
                definition.storage == BytecodeStorage::Character && indices.len() > dimensions;
            for (position, index) in indices.iter_mut().enumerate() {
                if explicit_character && position == 0 {
                    continue;
                }
                let ExprKind::Identifier(candidate) = &index.kind else {
                    continue;
                };
                // Emuera gives an in-scope variable or zero-argument function precedence over
                // the symbolic CSV key used by an array subscript.
                if program.scoped_variable(function, candidate).is_some() {
                    continue;
                }
                if program.function_by_name(candidate).is_some() {
                    index.kind = ExprKind::Call {
                        name: candidate.clone(),
                        args: Vec::new(),
                    };
                    continue;
                }
                if let Some(value) = crate::interpreter::native_ops::resolve_named_index_value(
                    program, name, candidate,
                ) {
                    index.kind = ExprKind::Integer(value);
                }
            }
        }
        ExprKind::Call { args, .. } => {
            for argument in args.iter_mut().flatten() {
                resolve_expression_named_indices(program, function, argument, depth + 1)?;
            }
        }
        ExprKind::Unary { operand, .. }
        | ExprKind::Postfix { operand, .. }
        | ExprKind::Group(operand) => {
            resolve_expression_named_indices(program, function, operand, depth + 1)?;
        }
        ExprKind::Binary { left, right, .. } => {
            resolve_expression_named_indices(program, function, left, depth + 1)?;
            resolve_expression_named_indices(program, function, right, depth + 1)?;
        }
        ExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            resolve_expression_named_indices(program, function, condition, depth + 1)?;
            resolve_expression_named_indices(program, function, then_expr, depth + 1)?;
            resolve_expression_named_indices(program, function, else_expr, depth + 1)?;
        }
        ExprKind::Formatted(formatted) => {
            resolve_named_indices(program, function, formatted, depth + 1)?;
        }
    }
    Ok(())
}

// Validation is an exhaustive, iterative walk over every supported FORM AST node.
#[allow(clippy::too_many_lines)]
fn validate_form(
    vm: &Vm,
    natives: &NativeServiceRegistry,
    generation: GenerationId,
    function: SymbolKey,
    formatted: &FormattedString,
    node_limit: usize,
) -> Result<usize, StepError> {
    enum Node<'a> {
        Form(&'a FormattedString),
        Expr(&'a Expr),
    }

    let program = vm.generations.get(&generation).ok_or_else(|| {
        StepError::new(VmFaultCode::MissingSymbol, "STRFORM generation is missing")
    })?;
    let mut pending = vec![Node::Form(formatted)];
    let mut nodes = 0usize;
    while let Some(node) = pending.pop() {
        nodes = nodes.saturating_add(1);
        if nodes > node_limit {
            return Err(resource_limit("STRFORM AST exceeds the VM operand limit"));
        }
        match node {
            Node::Form(formatted) => {
                for part in &formatted.parts {
                    match part {
                        FormPart::Text(_) => {}
                        FormPart::Triple { symbol, .. } => {
                            let names = match symbol {
                                '*' => Some(("NAME", "TARGET")),
                                '+' => Some(("CALLNAME", "MASTER")),
                                '=' => Some(("CALLNAME", "PLAYER")),
                                '/' => Some(("NAME", "ASSI")),
                                '$' => Some(("CALLNAME", "TARGET")),
                                _ => None,
                            }
                            .ok_or_else(|| {
                                unsupported(format!(
                                    "STRFORM triple symbol {symbol:?} is unsupported"
                                ))
                            })?;
                            for name in [names.0, names.1] {
                                if program.scoped_variable(function, name).is_none() {
                                    return Err(StepError::new(
                                        VmFaultCode::MissingSymbol,
                                        format!("STRFORM variable {name} is missing"),
                                    ));
                                }
                            }
                        }
                        FormPart::StringInterpolation {
                            expression, width, ..
                        }
                        | FormPart::IntegerInterpolation {
                            expression, width, ..
                        } => {
                            pending.push(Node::Expr(expression));
                            if let Some(width) = width {
                                pending.push(Node::Expr(width));
                            }
                        }
                        FormPart::Conditional {
                            condition,
                            then_value,
                            else_value,
                            ..
                        } => {
                            pending.push(Node::Expr(condition));
                            pending.push(Node::Form(then_value));
                            if let Some(else_value) = else_value {
                                pending.push(Node::Form(else_value));
                            }
                        }
                    }
                }
            }
            Node::Expr(expression) => match &expression.kind {
                ExprKind::Integer(_) | ExprKind::String(_) => {}
                ExprKind::Identifier(name) => {
                    ensure_variable(program, function, name)?;
                }
                ExprKind::Variable { name, indices } => {
                    ensure_variable(program, function, name)?;
                    pending.extend(indices.iter().map(Node::Expr));
                }
                ExprKind::Call { name, args } => {
                    if let Some(target) = program.function_by_name(name) {
                        if target.kind != BytecodeFunctionKind::Method || target.result.is_none() {
                            return Err(StepError::new(
                                VmFaultCode::TypeMismatch,
                                format!("STRFORM target {name} is not a value-returning function"),
                            ));
                        }
                        if target
                            .parameters
                            .iter()
                            .any(|parameter| parameter.by_reference)
                        {
                            return Err(unsupported(format!(
                                "STRFORM target {name} requires a reference argument"
                            )));
                        }
                        if args.len() > target.parameters.len()
                            || target
                                .parameters
                                .iter()
                                .enumerate()
                                .any(|(index, parameter)| {
                                    args.get(index).is_none_or(Option::is_none)
                                        && parameter.default.is_none()
                                        && !program
                                            .artifact
                                            .call_compatibility
                                            .allow_omitted_arguments
                                })
                        {
                            return Err(StepError::new(
                                VmFaultCode::TypeMismatch,
                                format!("STRFORM target {name} has incompatible arguments"),
                            ));
                        }
                    } else {
                        let supported_native =
                            program.artifact.native_imports.iter().any(|native| {
                                native.import.name.eq_ignore_ascii_case(name)
                                    && native.import.result.is_some()
                                    && !matches!(
                                        native.import.result,
                                        Some(
                                            BytecodeType::IntegerPlace | BytecodeType::StringPlace
                                        )
                                    )
                                    && native.import.parameters.len() == args.len()
                                    && (name.eq_ignore_ascii_case("STRFORM")
                                        || natives.contains(native.import.key))
                            });
                        if !supported_native {
                            let host_only = program
                                .artifact
                                .host_imports
                                .iter()
                                .any(|host| host.import.name.eq_ignore_ascii_case(name));
                            return Err(if host_only {
                                unsupported(format!(
                                    "STRFORM host callable {name} is unsupported in a template"
                                ))
                            } else {
                                StepError::new(
                                    VmFaultCode::MissingSymbol,
                                    format!("STRFORM callable {name} is missing"),
                                )
                            });
                        }
                    }
                    pending.extend(args.iter().flatten().map(Node::Expr));
                }
                ExprKind::Unary { op, operand } => {
                    if matches!(op, UnaryOp::PreIncrement | UnaryOp::PreDecrement) {
                        return Err(unsupported("STRFORM increment expressions are unsupported"));
                    }
                    pending.push(Node::Expr(operand));
                }
                ExprKind::Postfix { .. } => {
                    return Err(unsupported("STRFORM increment expressions are unsupported"));
                }
                ExprKind::Binary { left, right, .. } => {
                    pending.push(Node::Expr(left));
                    pending.push(Node::Expr(right));
                }
                ExprKind::Ternary {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    pending.push(Node::Expr(condition));
                    pending.push(Node::Expr(then_expr));
                    pending.push(Node::Expr(else_expr));
                }
                ExprKind::Formatted(formatted) => pending.push(Node::Form(formatted)),
                ExprKind::Group(inner) => pending.push(Node::Expr(inner)),
                ExprKind::Error => {
                    return Err(unsupported("STRFORM contains an invalid expression"));
                }
            },
        }
    }
    Ok(nodes)
}

fn ensure_variable(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    name: &str,
) -> Result<(), StepError> {
    if program.scoped_variable(function, name).is_none() {
        return Err(StepError::new(
            VmFaultCode::MissingSymbol,
            format!("STRFORM variable {name} is missing"),
        ));
    }
    Ok(())
}

fn preflight_nesting(source: &str) -> Result<(), StepError> {
    let mut braces = 0usize;
    let mut parentheses = 0usize;
    let mut percent_expression = false;
    let mut quote = None;
    let mut escaped = false;
    let mut unary_run = 0usize;
    let mut ternaries = 0usize;
    for character in source.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"') && (braces > 0 || percent_expression) {
            quote = Some(character);
            continue;
        }
        match character {
            '%' if braces == 0 => percent_expression = !percent_expression,
            '{' => braces = braces.saturating_add(1),
            '}' => braces = braces.saturating_sub(1),
            '(' => {
                parentheses = parentheses.saturating_add(1);
            }
            ')' => {
                parentheses = parentheses.saturating_sub(1);
            }
            _ => {}
        }
        if matches!(character, '!' | '~' | '+' | '-') {
            unary_run = unary_run.saturating_add(1);
        } else if !character.is_whitespace() {
            unary_run = 0;
        }
        if character == '?' {
            ternaries = ternaries.saturating_add(1);
        }
        if braces.saturating_add(parentheses) > MAX_RUNTIME_FORM_NESTING
            || unary_run > MAX_RUNTIME_FORM_NESTING
            || ternaries > MAX_RUNTIME_FORM_NESTING
        {
            return Err(resource_limit("STRFORM parser nesting limit exceeded"));
        }
    }
    Ok(())
}
