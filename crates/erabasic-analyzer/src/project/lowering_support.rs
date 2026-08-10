use erabasic_ast::{Expr, ExprKind, Function as AstFunction, SourceKind, Statement, StatementKind};
use erabasic_hir::{
    FunctionId, FunctionKind, HirArgument, HirExprKind, HirStatementKind, InstructionTarget,
    SemanticType, SourceFile, SourceId,
};

use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, AnalyzerOptions,
    catalog::Catalog,
    context::AnalysisParserContext,
    declarations::{
        DeclarationInput, parse_integer_constant, parse_private_declaration,
        parse_scoped_declaration,
    },
    expression::{ExpressionAnalyzer, IndexResolver},
    symbols::Symbols,
};

use super::{
    ParsedProjectSource,
    source_support::{key, map_parser_diagnostic},
};

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) fn analyze_scoped_declaration_statement(
    function: FunctionId,
    source: &ParsedProjectSource,
    statement: &Statement,
    name: &str,
    raw_arguments: &str,
    symbols: &Symbols,
    catalog: &Catalog,
    context: &AnalysisParserContext,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> HirStatementKind {
    let constants = symbols.constant_values();
    let dimensions = symbols.variable_dimensions(function);
    let Ok(scoped) = parse_scoped_declaration(
        source.source.id,
        &source.source.relative_path,
        &source.text,
        name,
        raw_arguments,
        statement.span,
        context,
        &constants,
        &dimensions,
        index_resolver,
        options,
    ) else {
        return HirStatementKind::Error;
    };
    let variable_name = scoped.declaration.schema.id.name();
    let target_expression = Expr {
        kind: ExprKind::Variable {
            name: variable_name.to_owned(),
            indices: Vec::new(),
        },
        span: statement.span,
    };
    let target = ExpressionAnalyzer {
        symbols,
        catalog,
        options,
        function,
        source: source.source.id,
        path: &source.source.relative_path,
        text: &source.text,
        diagnostics,
        index_resolver,
    }
    .analyze(&target_expression);
    let HirExprKind::Variable { place } = target.kind else {
        return HirStatementKind::Error;
    };
    // The reference declaration instruction initializes only scalars. Array
    // storage is zero-filled when the function frame is created.
    let scalar = symbols
        .variables
        .get(place.variable.0 as usize)
        .is_some_and(|variable| variable.dimensions == [1]);
    if !scalar {
        return HirStatementKind::Instruction {
            target: InstructionTarget::Builtin(name.to_owned()),
            arguments: Vec::new(),
        };
    }
    let value = if let Some(initializer) = scoped.initializer {
        let statement_text = &source.text[statement.span.start..statement.span.end];
        let raw_offset = statement_text.find(raw_arguments).unwrap_or(0);
        let base =
            statement.span.start + raw_offset + scoped.initializer_offset.unwrap_or_default();
        let mut parsed = erabasic_parser::parse_expression_list_at(&initializer, base, context);
        for diagnostic in parsed.diagnostics.drain(..) {
            diagnostics.push(map_parser_diagnostic(
                source.source.id,
                &source.source.relative_path,
                &source.text,
                &diagnostic,
            ));
        }
        parsed
            .value
            .and_then(|mut values| (values.len() == 1).then(|| values.remove(0)))
    } else {
        Some(Expr {
            kind: if name == "VARS" {
                ExprKind::String(String::new())
            } else {
                ExprKind::Integer(0)
            },
            span: statement.span,
        })
    };
    let Some(value) = value else {
        diagnostics.push(AnalyzerDiagnostic::at(
            AnalyzerDiagnosticCode::InvalidInitializer,
            AnalyzerDiagnosticSeverity::Error,
            2,
            source.source.id,
            &source.source.relative_path,
            &source.text,
            statement.span,
            "scoped variable initializer must contain exactly one expression",
        ));
        return HirStatementKind::Error;
    };
    let value = ExpressionAnalyzer {
        symbols,
        catalog,
        options,
        function,
        source: source.source.id,
        path: &source.source.relative_path,
        text: &source.text,
        diagnostics,
        index_resolver,
    }
    .analyze(&value);
    if place.value_type != value.value_type && value.value_type != SemanticType::Error {
        diagnostics.push(AnalyzerDiagnostic::at(
            AnalyzerDiagnosticCode::TypeMismatch,
            AnalyzerDiagnosticSeverity::Error,
            2,
            source.source.id,
            &source.source.relative_path,
            &source.text,
            value.location.span,
            "scoped variable initializer type does not match its declaration",
        ));
    }
    HirStatementKind::Assignment {
        target: place,
        op: erabasic_ast::AssignOp::Assign,
        value,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn analyze_case_arguments(
    function: FunctionId,
    source: &ParsedProjectSource,
    statement: &Statement,
    raw: &str,
    symbols: &Symbols,
    catalog: &Catalog,
    context: &AnalysisParserContext,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> Vec<HirArgument> {
    let raw = strip_case_comment(raw);
    let statement_text = &source.text[statement.span.start..statement.span.end];
    let raw_base = statement
        .span
        .start
        .saturating_add(statement_text.find(raw).unwrap_or(0));
    let mut lowered = Vec::new();
    for (segment_offset, segment) in split_case_segments(raw) {
        let leading = segment.len().saturating_sub(segment.trim_start().len());
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        let base = raw_base + segment_offset + leading;
        let (tag, operands) = if let Some(rest) = strip_ascii_keyword(segment, "IS") {
            let rest = rest.trim_start();
            let (tag, expression) = [
                (">=", "ge"),
                ("<=", "le"),
                ("!=", "ne"),
                ("==", "eq"),
                (">", "gt"),
                ("<", "lt"),
                ("=", "eq"),
                ("&", "and"),
            ]
            .into_iter()
            .find_map(|(operator, tag)| {
                rest.strip_prefix(operator)
                    .map(|expression| (tag, expression.trim_start()))
            })
            .unwrap_or(("eq", rest));
            (tag, vec![expression])
        } else if let Some(to) = find_case_to(segment) {
            ("range", vec![&segment[..to], &segment[to + 2..]])
        } else {
            ("eq", vec![segment])
        };
        lowered.push(HirArgument::Raw(tag.into()));
        for operand in operands {
            let operand_leading = operand.len().saturating_sub(operand.trim_start().len());
            let operand = operand.trim();
            let operand_offset = segment.find(operand).unwrap_or(0);
            let mut parsed = erabasic_parser::parse_expression_list_at(
                operand,
                base + operand_offset + operand_leading,
                context,
            );
            for diagnostic in parsed.diagnostics.drain(..) {
                diagnostics.push(map_parser_diagnostic(
                    source.source.id,
                    &source.source.relative_path,
                    &source.text,
                    &diagnostic,
                ));
            }
            let expression = parsed
                .value
                .and_then(|mut values| (!values.is_empty()).then(|| values.remove(0)));
            let Some(expression) = expression else {
                continue;
            };
            lowered.push(HirArgument::Expression(
                ExpressionAnalyzer {
                    symbols,
                    catalog,
                    options,
                    function,
                    source: source.source.id,
                    path: &source.source.relative_path,
                    text: &source.text,
                    diagnostics,
                    index_resolver,
                }
                .analyze(&expression),
            ));
        }
    }
    lowered
}

fn split_case_segments(source: &str) -> Vec<(usize, &str)> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    let mut quoted = false;
    for (index, character) in source.char_indices() {
        match character {
            '"' if !is_era_escaped(source, index) => quoted = !quoted,
            '(' | '[' | '{' if !quoted => depth = depth.saturating_add(1),
            ')' | ']' | '}' if !quoted => depth = depth.saturating_sub(1),
            ',' if !quoted && depth == 0 => {
                result.push((start, &source[start..index]));
                start = index + 1;
            }
            _ => {}
        }
    }
    result.push((start, &source[start..]));
    result
}

fn strip_case_comment(source: &str) -> &str {
    let mut quoted = false;
    for (index, character) in source.char_indices() {
        match character {
            '"' if !is_era_escaped(source, index) => quoted = !quoted,
            ';' if !quoted => return &source[..index],
            _ => {}
        }
    }
    source
}

fn strip_ascii_keyword<'a>(source: &'a str, keyword: &str) -> Option<&'a str> {
    source
        .get(..keyword.len())
        .filter(|prefix| prefix.eq_ignore_ascii_case(keyword))
        .and_then(|_| source.get(keyword.len()..))
        .filter(|rest| {
            rest.starts_with(char::is_whitespace) || rest.starts_with(['>', '<', '=', '!', '&'])
        })
}

fn find_case_to(source: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0_u32;
    let mut quoted = false;
    let mut index = 0;
    while index + 1 < bytes.len() {
        match bytes[index] {
            b'"' if !is_era_escaped(source, index) => quoted = !quoted,
            b'(' | b'[' | b'{' if !quoted => depth = depth.saturating_add(1),
            b')' | b']' | b'}' if !quoted => depth = depth.saturating_sub(1),
            _ => {}
        }
        if !quoted
            && depth == 0
            && bytes[index].eq_ignore_ascii_case(&b't')
            && bytes[index + 1].eq_ignore_ascii_case(&b'o')
            && index > 0
            && index + 2 < bytes.len()
            && bytes[index - 1].is_ascii_whitespace()
            && bytes[index + 2].is_ascii_whitespace()
        {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn is_era_escaped(source: &str, index: usize) -> bool {
    source.as_bytes()[..index]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

pub(super) fn static_target_source(raw: &str) -> &str {
    let mut brackets = 0_u32;
    let mut quoted = false;
    let end = raw
        .char_indices()
        .find_map(|(index, character)| match character {
            '"' => {
                quoted = !quoted;
                None
            }
            '[' if !quoted => {
                brackets = brackets.saturating_add(1);
                None
            }
            ']' if !quoted => {
                brackets = brackets.saturating_sub(1);
                None
            }
            ',' | '(' | ';' if !quoted && brackets == 0 => Some(index),
            _ => None,
        })
        .unwrap_or(raw.len());
    &raw[..end]
}

pub(super) fn resolve_static_target(raw: &str, index_resolver: &IndexResolver) -> String {
    let target = static_target_source(raw).trim().trim_matches('"');
    let mut result = String::with_capacity(target.len());
    let mut rest = target;
    while let Some(start) = rest.find("[[") {
        result.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else {
            result.push_str(&rest[start..]);
            return result;
        };
        let name = &after[..end];
        if let Some(value) = index_resolver.resolve_rename(name) {
            result.push_str(&value.to_string());
        } else {
            result.push_str(&rest[start..start + 2 + end + 2]);
        }
        rest = &after[end + 2..];
    }
    result.push_str(rest);
    result
}

pub(super) fn source_file(
    id: SourceId,
    relative_path: String,
    kind: SourceKind,
    text: &str,
) -> SourceFile {
    let mut line_starts = vec![0];
    line_starts.extend(
        text.bytes()
            .enumerate()
            .filter_map(|(index, byte)| (byte == b'\n').then_some((index + 1) as u64)),
    );
    SourceFile {
        id,
        relative_path,
        kind,
        content_hash: *blake3::hash(text.as_bytes()).as_bytes(),
        byte_len: text.len() as u64,
        line_starts,
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) fn register_function_declarations(
    function_id: FunctionId,
    function_kind: FunctionKind,
    source: &ParsedProjectSource,
    function: &AstFunction,
    symbols: &mut Symbols,
    context: &AnalysisParserContext,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) {
    let has_declarations = function.attributes.iter().any(|directive| {
        matches!(
            directive.name.as_str(),
            "DIM" | "DIMS" | "LOCALSIZE" | "LOCALSSIZE"
        )
    });
    let scoped_statements = function.body.iter().filter_map(|statement| {
        let StatementKind::Instruction {
            name,
            raw_arguments,
            ..
        } = &statement.kind
        else {
            return None;
        };
        matches!(name.as_str(), "VARI" | "VARS").then_some((
            statement,
            name.as_str(),
            raw_arguments.as_str(),
        ))
    });
    if !has_declarations && scoped_statements.clone().next().is_none() {
        return;
    }

    // Building the constant lookup clones every constant name and value. Most
    // functions have no private declarations, so defer that work until the
    // declaration parser can actually consume it.
    let mut constants = symbols.constant_values();
    let mut variable_dimensions = symbols.variable_dimensions(function_id);
    let mut integer_size = None;
    let mut string_size = None;
    for directive in &function.attributes {
        if matches!(directive.name.as_str(), "LOCALSIZE" | "LOCALSSIZE") {
            if function_kind == FunctionKind::Event {
                diagnostics.push(AnalyzerDiagnostic::at(
                    AnalyzerDiagnosticCode::InvalidDeclaration,
                    AnalyzerDiagnosticSeverity::Warning,
                    1,
                    source.source.id,
                    &source.source.relative_path,
                    &source.text,
                    directive.span,
                    format!("event function ignores #{}", directive.name),
                ));
                continue;
            }
            let (variable_name, previous_size) = if directive.name == "LOCALSIZE" {
                ("LOCAL", &mut integer_size)
            } else {
                ("LOCALS", &mut string_size)
            };
            let size = match parse_integer_constant(
                &directive.raw_arguments,
                context,
                &constants,
                &variable_dimensions,
                index_resolver,
                options,
            ) {
                Ok(size) if size > 0 && size < i64::from(i32::MAX) => {
                    usize::try_from(size).expect("positive i32 local size fits usize")
                }
                Ok(size) => {
                    diagnostics.push(AnalyzerDiagnostic::at(
                        AnalyzerDiagnosticCode::InvalidDimension,
                        AnalyzerDiagnosticSeverity::Warning,
                        1,
                        source.source.id,
                        &source.source.relative_path,
                        &source.text,
                        directive.span,
                        format!(
                            "#{} size {size} is outside the supported range",
                            directive.name
                        ),
                    ));
                    continue;
                }
                Err(message) => {
                    diagnostics.push(AnalyzerDiagnostic::at(
                        AnalyzerDiagnosticCode::InvalidDeclaration,
                        AnalyzerDiagnosticSeverity::Error,
                        2,
                        source.source.id,
                        &source.source.relative_path,
                        &source.text,
                        directive.span,
                        message,
                    ));
                    continue;
                }
            };
            if symbols.resize_era_local(function_id, variable_name, size) {
                if previous_size.replace(size).is_some() {
                    diagnostics.push(AnalyzerDiagnostic::at(
                        AnalyzerDiagnosticCode::InvalidDeclaration,
                        AnalyzerDiagnosticSeverity::Warning,
                        1,
                        source.source.id,
                        &source.source.relative_path,
                        &source.text,
                        directive.span,
                        format!("#{} replaces an earlier size declaration", directive.name),
                    ));
                }
                variable_dimensions.insert(key(variable_name, options.ignore_case), vec![size]);
            } else {
                diagnostics.push(AnalyzerDiagnostic::at(
                    AnalyzerDiagnosticCode::InvalidDeclaration,
                    AnalyzerDiagnosticSeverity::Error,
                    2,
                    source.source.id,
                    &source.source.relative_path,
                    &source.text,
                    directive.span,
                    format!("{variable_name} is prohibited by the project variable schema"),
                ));
            }
            continue;
        }
        if !matches!(directive.name.as_str(), "DIM" | "DIMS") {
            continue;
        }
        let input = DeclarationInput {
            source: source.source.id,
            path: &source.source.relative_path,
            text: &source.text,
            directive,
        };
        match parse_private_declaration(
            &input,
            context,
            &constants,
            &variable_dimensions,
            index_resolver,
            options,
        ) {
            Ok(declaration) => {
                if symbols.register_private(function_id, &declaration).is_err() {
                    diagnostics.push(AnalyzerDiagnostic::at(
                        AnalyzerDiagnosticCode::DuplicateSymbol,
                        AnalyzerDiagnosticSeverity::Error,
                        2,
                        source.source.id,
                        &source.source.relative_path,
                        &source.text,
                        directive.span,
                        format!(
                            "private variable {} is already declared",
                            declaration.schema.id.name()
                        ),
                    ));
                } else {
                    let key = key(declaration.schema.id.name(), options.ignore_case);
                    variable_dimensions.insert(key.clone(), declaration.schema.dimensions.clone());
                    if declaration.schema.storage == erabasic_data::StorageScope::Constant
                        && let Some(value) = declaration.initial_values.first()
                    {
                        constants.insert(key, value.clone());
                    }
                }
            }
            Err(message) => diagnostics.push(AnalyzerDiagnostic::at(
                AnalyzerDiagnosticCode::InvalidDeclaration,
                AnalyzerDiagnosticSeverity::Error,
                2,
                source.source.id,
                &source.source.relative_path,
                &source.text,
                directive.span,
                message,
            )),
        }
    }
    for (statement, name, raw_arguments) in scoped_statements {
        match parse_scoped_declaration(
            source.source.id,
            &source.source.relative_path,
            &source.text,
            name,
            raw_arguments,
            statement.span,
            context,
            &constants,
            &variable_dimensions,
            index_resolver,
            options,
        ) {
            Ok(scoped) => {
                let declaration = scoped.declaration;
                // Emuera.NET keeps the first scoped declaration with a given
                // function-local name and permits later declaration statements
                // to reinitialize that same scalar.
                if symbols.register_private(function_id, &declaration).is_ok() {
                    let key = key(declaration.schema.id.name(), options.ignore_case);
                    variable_dimensions.insert(key, declaration.schema.dimensions.clone());
                }
            }
            Err(message) => diagnostics.push(AnalyzerDiagnostic::at(
                AnalyzerDiagnosticCode::InvalidDeclaration,
                AnalyzerDiagnosticSeverity::Error,
                2,
                source.source.id,
                &source.source.relative_path,
                &source.text,
                statement.span,
                message,
            )),
        }
    }
}
