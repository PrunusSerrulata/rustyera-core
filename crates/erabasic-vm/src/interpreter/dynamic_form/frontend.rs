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
    let context = parser_context(program);
    let parsed = parse_formatted_at(source, 0, &context);
    if parsed.has_errors() {
        let message = parsed
            .diagnostics
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(StepError::script(
            crate::ScriptFaultKind::Parse,
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

pub(super) fn resolve_expression_named_indices(
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
                let data_dimension = position.saturating_sub(usize::from(explicit_character));
                if let Some(value) = crate::interpreter::native_ops::resolve_named_index_value(
                    program,
                    name,
                    candidate,
                    data_dimension,
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

fn validate_form(
    vm: &Vm,
    natives: &NativeServiceRegistry,
    generation: GenerationId,
    function: SymbolKey,
    formatted: &FormattedString,
    node_limit: usize,
) -> Result<usize, StepError> {
    let program = vm.generations.get(&generation).ok_or_else(|| {
        StepError::new(VmFaultCode::MissingSymbol, "STRFORM generation is missing")
    })?;
    let mut analysis = super::typing::TypeAnalysis::new(
        program,
        function,
        generation,
        node_limit,
        Some(natives),
    );
    analysis.form(formatted, 0)?;
    Ok(analysis.nodes())
}

pub(super) fn validate_runtime_expression(
    vm: &Vm,
    natives: &NativeServiceRegistry,
    generation: GenerationId,
    function: SymbolKey,
    expression: &Expr,
    node_limit: usize,
) -> Result<(usize, erabasic_bytecode::UserArgumentSpec), StepError> {
    let program = vm.generations.get(&generation).ok_or_else(|| {
        StepError::new(
            VmFaultCode::MissingSymbol,
            "runtime expression generation is missing",
        )
    })?;
    let mut analysis = super::typing::TypeAnalysis::new(
        program,
        function,
        generation,
        node_limit,
        Some(natives),
    );
    let kind = analysis.expression(expression, 0)?;
    Ok((
        analysis.nodes(),
        super::typing::shape_spec(program, function, expression, kind),
    ))
}

pub(super) fn preflight_nesting(source: &str) -> Result<(), StepError> {
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


pub(super) fn parser_context(program: &crate::ProgramGeneration) -> DefaultParserContext {
    let compatibility = program.artifact.call_compatibility;
    let mut context = DefaultParserContext::default();
    context.set_compatibility(program.artifact.manifest.compatibility.clone());
    context.set_lexer_compatibility(
        compatibility.allow_full_width_space,
        compatibility.debug_semicolon,
        compatibility.ignore_triple_symbols,
    );
    context
}
