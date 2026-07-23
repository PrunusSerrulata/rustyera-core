use std::collections::BTreeMap;

use erabasic_ast::{BinaryOp, Directive, Expr, ExprKind, FormPart, FormattedString, UnaryOp};
use erabasic_data::{
    Persistence, ProjectData, StorageScope, UserIndexRegistration, ValueType,
    VariableId as DataVariableId, VariableSchema,
};
use erabasic_hir::{ConstantValue, SourceId, SourceLocation};
use erabasic_parser::{ParserContext, parse_expression};

use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, AnalyzerOptions,
    expression::IndexResolver, symbols::is_reserved,
};

pub(crate) struct DeclarationInput<'a> {
    pub source: SourceId,
    pub path: &'a str,
    pub text: &'a str,
    pub directive: &'a Directive,
}

#[derive(Clone, Debug)]
pub(crate) struct DeclaredVariable {
    pub schema: VariableSchema,
    pub initial_values: Vec<ConstantValue>,
    pub location: SourceLocation,
    pub reference: bool,
    pub static_lifetime: bool,
}

#[derive(Default)]
pub(crate) struct DeclarationOutput {
    pub variables: BTreeMap<String, DeclaredVariable>,
    pub registrations: Vec<UserIndexRegistration>,
}

pub(crate) fn analyze_global_declarations(
    project: &mut ProjectData,
    inputs: &[DeclarationInput<'_>],
    context: &dyn ParserContext,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> DeclarationOutput {
    let mut output = DeclarationOutput::default();
    let mut constants = BTreeMap::new();
    let index_resolver = IndexResolver::new(project);
    let mut variable_dimensions: BTreeMap<_, _> = project
        .schema
        .variables
        .values()
        .map(|schema| {
            (
                normalize(schema.id.name(), options.ignore_case),
                schema.dimensions.clone(),
            )
        })
        .collect();
    let mut pending: Vec<_> = inputs
        .iter()
        .filter(|input| matches!(input.directive.name.as_str(), "DIM" | "DIMS"))
        .collect();

    // Emuera queues every header DIM and retries the queue while another declaration
    // was resolved. This permits dimensions to refer to constants declared later.
    while !pending.is_empty() {
        let before = pending.len();
        let mut deferred = Vec::new();
        for input in pending {
            match parse_dim(
                input,
                false,
                context,
                &constants,
                &variable_dimensions,
                &index_resolver,
                options,
            ) {
                Ok(variable) => {
                    let key = normalize(variable.schema.id.name(), options.ignore_case);
                    if is_reserved(variable.schema.id.name()) {
                        diagnostics.push(at_input(
                            input,
                            AnalyzerDiagnosticCode::ReservedName,
                            format!("{} is a reserved identifier", variable.schema.id.name()),
                        ));
                        continue;
                    }
                    if project.schema.variable(&key).is_some()
                        || output.variables.contains_key(&key)
                    {
                        diagnostics.push(at_input(
                            input,
                            AnalyzerDiagnosticCode::DuplicateSymbol,
                            format!("variable {} is already declared", variable.schema.id.name()),
                        ));
                        continue;
                    }
                    if variable.schema.storage == StorageScope::Constant
                        && let Some(value) = variable.initial_values.first()
                    {
                        constants.insert(key.clone(), value.clone());
                    }
                    project
                        .schema
                        .register_user_variable(variable.schema.clone());
                    variable_dimensions.insert(key.clone(), variable.schema.dimensions.clone());
                    add_registrations(&variable.schema, &mut output.registrations);
                    output.variables.insert(key, variable);
                }
                Err(DimError::UnknownConstant(_)) => deferred.push(input),
                Err(error) => diagnostics.push(at_input(input, error.code(), error.to_string())),
            }
        }
        if deferred.len() == before {
            for input in deferred {
                diagnostics.push(at_input(
                    input,
                    AnalyzerDiagnosticCode::UnknownIdentifier,
                    "a #DIM constant or dimension could not be resolved",
                ));
            }
            break;
        }
        pending = deferred;
    }
    output
}

pub(crate) fn parse_private_declaration(
    input: &DeclarationInput<'_>,
    context: &dyn ParserContext,
    constants: &BTreeMap<String, ConstantValue>,
    variable_dimensions: &BTreeMap<String, Vec<usize>>,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
) -> Result<DeclaredVariable, String> {
    parse_dim(
        input,
        true,
        context,
        constants,
        variable_dimensions,
        index_resolver,
        options,
    )
    .map_err(|error| error.to_string())
}

#[derive(Clone, Debug)]
enum DimError {
    Invalid(String),
    UnknownConstant(String),
    InvalidInitializer(String),
}

impl DimError {
    fn code(&self) -> AnalyzerDiagnosticCode {
        match self {
            Self::Invalid(_) => AnalyzerDiagnosticCode::InvalidDeclaration,
            Self::UnknownConstant(_) => AnalyzerDiagnosticCode::UnknownIdentifier,
            Self::InvalidInitializer(_) => AnalyzerDiagnosticCode::InvalidInitializer,
        }
    }
}

impl std::fmt::Display for DimError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message)
            | Self::UnknownConstant(message)
            | Self::InvalidInitializer(message) => formatter.write_str(message),
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn parse_dim(
    input: &DeclarationInput<'_>,
    private: bool,
    context: &dyn ParserContext,
    constants: &BTreeMap<String, ConstantValue>,
    variable_dimensions: &BTreeMap<String, Vec<usize>>,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
) -> Result<DeclaredVariable, DimError> {
    let constant_evaluation = ConstantEvaluation {
        constants,
        variable_dimensions,
        index_resolver,
        options,
    };
    let is_string = input.directive.name == "DIMS";
    // Directive arguments are kept raw by the syntax parser. Emuera's declaration
    // lexer still terminates them at an unescaped semicolon, including when no space
    // separates the variable name and its comment.
    let mut rest = strip_declaration_comment(&input.directive.raw_arguments);
    let mut is_const = false;
    let mut reference = false;
    let mut global = false;
    let mut saved = false;
    let mut character = false;
    let mut is_static = true;
    let mut static_seen = false;
    let name;

    loop {
        let Some((word, remaining)) = take_word(rest) else {
            return Err(DimError::Invalid("#DIM requires a variable name".into()));
        };
        rest = remaining;
        match word.to_ascii_uppercase().as_str() {
            "CONST" if !is_const => is_const = true,
            "REF" if !reference => {
                reference = true;
                is_static = false;
            }
            "GLOBAL" if !global => global = true,
            "SAVEDATA" if !saved => saved = true,
            "CHARADATA" if !character => character = true,
            "STATIC" if !static_seen => {
                is_static = true;
                static_seen = true;
            }
            "DYNAMIC" if !static_seen => {
                is_static = false;
                static_seen = true;
            }
            "CONST" | "REF" | "GLOBAL" | "SAVEDATA" | "CHARADATA" | "STATIC" | "DYNAMIC" => {
                return Err(DimError::Invalid(format!(
                    "duplicate or conflicting keyword {word}"
                )));
            }
            _ => {
                name = word.to_owned();
                break;
            }
        }
    }

    if private && (global || saved || character) {
        return Err(DimError::Invalid(
            "GLOBAL, SAVEDATA and CHARADATA are not valid private variable attributes".into(),
        ));
    }
    if !private && static_seen {
        return Err(DimError::Invalid(
            "STATIC and DYNAMIC are only valid for private variables".into(),
        ));
    }
    if reference && (is_const || global || saved || character || static_seen) {
        return Err(DimError::Invalid(
            "REF conflicts with CONST, STATIC, GLOBAL, SAVEDATA or CHARADATA".into(),
        ));
    }
    if is_const && (global || saved || character || !is_static) {
        return Err(DimError::Invalid(
            "CONST conflicts with GLOBAL, SAVEDATA, CHARADATA or DYNAMIC".into(),
        ));
    }
    if !private && reference {
        return Err(DimError::Invalid(
            "the pinned reference does not implement global REF declarations".into(),
        ));
    }

    let (dimension_text, initializer_text) = split_assignment(rest);
    let mut dimensions = Vec::new();
    let dimension_text = dimension_text.trim();
    if !dimension_text.is_empty() {
        if !dimension_text.starts_with(',') {
            return Err(DimError::Invalid(
                "a comma is required before a #DIM size".into(),
            ));
        }
        for segment in split_top_level(&dimension_text[1..], ',') {
            if reference && segment.trim().is_empty() {
                dimensions.push(0);
                continue;
            }
            let value = parse_constant(segment, context, &constant_evaluation)?;
            let ConstantValue::Integer(value) = value else {
                return Err(DimError::Invalid("array size must be an integer".into()));
            };
            if reference {
                if value != 0 {
                    return Err(DimError::Invalid("REF array sizes must be zero".into()));
                }
            } else if !(1..=1_000_000).contains(&value) {
                return Err(DimError::Invalid(
                    "array size must be between 1 and 1000000".into(),
                ));
            }
            dimensions.push(usize::try_from(value).unwrap_or_default());
        }
    }

    let mut initial_values = Vec::new();
    if let Some(initializer) = initializer_text {
        if reference || character || dimensions.len() >= 2 {
            return Err(DimError::InvalidInitializer(
                "this declaration cannot have an initializer".into(),
            ));
        }
        let mut segments = split_top_level(initializer, ',');
        if segments
            .last()
            .is_some_and(|segment| segment.trim().is_empty())
        {
            segments.pop();
        }
        for segment in segments {
            if segment.trim().is_empty() {
                return Err(DimError::InvalidInitializer(
                    "array initializers cannot be omitted".into(),
                ));
            }
            let value = parse_constant(segment, context, &constant_evaluation)?;
            if matches!(value, ConstantValue::String(_)) != is_string {
                return Err(DimError::InvalidInitializer(
                    "initializer type does not match the variable".into(),
                ));
            }
            initial_values.push(value);
        }
        if dimensions.is_empty() {
            dimensions.push(initial_values.len());
        }
        if initial_values.len() > dimensions[0]
            || (is_const && initial_values.len() != dimensions[0])
        {
            return Err(DimError::InvalidInitializer(
                "initializer count does not match the declared size".into(),
            ));
        }
    } else if is_const {
        return Err(DimError::InvalidInitializer(
            "CONST variables require an initializer".into(),
        ));
    }
    if dimensions.is_empty() {
        dimensions.push(1);
    }
    if dimensions.len() > 3 || (character && dimensions.len() > 2) {
        return Err(DimError::Invalid(
            "EraBasic user variables support at most three dimensions (two for CHARADATA)".into(),
        ));
    }
    let total = dimensions
        .iter()
        .try_fold(1usize, |total, size| total.checked_mul(*size));
    if !reference && total.is_none_or(|total| total == 0 || total > 1_000_000) {
        return Err(DimError::Invalid(
            "the declared array contains too many elements".into(),
        ));
    }
    if character && !options.system_save_in_binary {
        return Err(DimError::Invalid(
            "user-defined CHARADATA variables require binary saves".into(),
        ));
    }
    if saved && is_string && dimensions.len() > 1 && !options.system_save_in_binary {
        return Err(DimError::Invalid(
            "multi-dimensional saved string variables require binary saves".into(),
        ));
    }

    let storage = if private {
        StorageScope::Local
    } else if character {
        StorageScope::Character
    } else if global {
        StorageScope::Global
    } else if is_const {
        StorageScope::Constant
    } else {
        StorageScope::Normal
    };
    let persistence = if global {
        Persistence::GlobalSave
    } else if saved || character {
        Persistence::GameSave
    } else {
        Persistence::None
    };
    Ok(DeclaredVariable {
        schema: VariableSchema {
            id: DataVariableId::user(name),
            value_type: if is_string {
                ValueType::String
            } else {
                ValueType::Integer
            },
            storage,
            dimensions,
            mutable: !is_const,
            persistence,
            can_forbid: false,
        },
        initial_values,
        location: SourceLocation::new(input.source, input.directive.span),
        reference,
        static_lifetime: is_static,
    })
}

fn take_word(source: &str) -> Option<(&str, &str)> {
    let trimmed = source.trim_start_matches(char::is_whitespace);
    let end = trimmed
        .find(|character: char| character.is_whitespace() || matches!(character, ',' | '='))
        .unwrap_or(trimmed.len());
    (end > 0).then(|| (&trimmed[..end], &trimmed[end..]))
}

fn strip_declaration_comment(source: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    let mut escaped_semicolon_until = 0;
    for (index, character) in source.char_indices() {
        if index < escaped_semicolon_until {
            continue;
        }
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            ';' if source[index..].starts_with(";!;") => {
                escaped_semicolon_until = index + 3;
            }
            ';' => return &source[..index],
            _ => {}
        }
    }
    source
}

fn split_assignment(source: &str) -> (&str, Option<&str>) {
    find_top_level(source, '=').map_or((source, None), |index| {
        (&source[..index], Some(&source[index + 1..]))
    })
}

fn split_top_level(source: &str, separator: char) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in source.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            value if value == separator && depth == 0 => {
                result.push(&source[start..index]);
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    result.push(&source[start..]);
    result
}

fn find_top_level(source: &str, target: char) -> Option<usize> {
    split_top_level_indices(source, target).into_iter().next()
}

fn split_top_level_indices(source: &str, target: char) -> Vec<usize> {
    let mut indices = Vec::new();
    let mut depth = 0usize;
    let mut quote = None;
    for (index, character) in source.char_indices() {
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            value if value == target && depth == 0 => indices.push(index),
            _ => {}
        }
    }
    indices
}

struct ConstantEvaluation<'a> {
    constants: &'a BTreeMap<String, ConstantValue>,
    variable_dimensions: &'a BTreeMap<String, Vec<usize>>,
    index_resolver: &'a IndexResolver,
    options: &'a AnalyzerOptions,
}

fn parse_constant(
    source: &str,
    context: &dyn ParserContext,
    evaluation: &ConstantEvaluation<'_>,
) -> Result<ConstantValue, DimError> {
    let output = parse_expression(source.trim(), context);
    if output.has_errors() {
        return Err(DimError::Invalid(format!(
            "invalid constant expression {source:?}"
        )));
    }
    let expression = output
        .value
        .ok_or_else(|| DimError::Invalid("constant expression is empty".into()))?;
    evaluate_constant(&expression, evaluation)
}

fn evaluate_constant(
    expression: &Expr,
    evaluation: &ConstantEvaluation<'_>,
) -> Result<ConstantValue, DimError> {
    match &expression.kind {
        ExprKind::Integer(value) => Ok(ConstantValue::Integer(*value)),
        ExprKind::String(value) => Ok(ConstantValue::String(value.clone())),
        ExprKind::Identifier(name) => evaluation
            .constants
            .get(&normalize(name, evaluation.options.ignore_case))
            .cloned()
            .ok_or_else(|| DimError::UnknownConstant(name.clone())),
        ExprKind::Group(inner) => evaluate_constant(inner, evaluation),
        ExprKind::Unary { op, operand } => {
            let ConstantValue::Integer(value) = evaluate_constant(operand, evaluation)? else {
                return Err(DimError::Invalid("integer unary operand required".into()));
            };
            let value = match op {
                UnaryOp::Plus => value,
                UnaryOp::Minus => value.wrapping_neg(),
                UnaryOp::LogicalNot => i64::from(value == 0),
                UnaryOp::BitNot => !value,
                UnaryOp::PreIncrement | UnaryOp::PreDecrement => {
                    return Err(DimError::Invalid(
                        "increment is not a constant expression".into(),
                    ));
                }
            };
            Ok(ConstantValue::Integer(value))
        }
        ExprKind::Binary { op, left, right } => {
            let left = evaluate_constant(left, evaluation)?;
            let right = evaluate_constant(right, evaluation)?;
            evaluate_binary(*op, left, right)
        }
        ExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            let ConstantValue::Integer(condition) = evaluate_constant(condition, evaluation)?
            else {
                return Err(DimError::Invalid("integer condition required".into()));
            };
            evaluate_constant(
                if condition != 0 { then_expr } else { else_expr },
                evaluation,
            )
        }
        ExprKind::Call { name, args } if name.eq_ignore_ascii_case("VARSIZE") => {
            evaluate_varsize(args, evaluation)
        }
        ExprKind::Call { name, args } if name.eq_ignore_ascii_case("GETNUM") => {
            evaluate_getnum(args, evaluation)
        }
        ExprKind::Call { name, args }
            if name.eq_ignore_ascii_case("GETDEFCOLOR") && args.is_empty() =>
        {
            Ok(ConstantValue::Integer(
                evaluation.options.default_foreground_color,
            ))
        }
        ExprKind::Call { name, args }
            if matches!(name.to_ascii_uppercase().as_str(), "STRLENS" | "STRLENSU")
                && args.len() == 1 =>
        {
            evaluate_string_length(name, args, evaluation)
        }
        ExprKind::Call { name, args }
            if name.eq_ignore_ascii_case("UNICODE") && args.len() == 1 =>
        {
            let argument = args[0]
                .as_ref()
                .ok_or_else(|| DimError::Invalid("UNICODE requires an argument".into()))?;
            let ConstantValue::Integer(value) = evaluate_constant(argument, evaluation)? else {
                return Err(DimError::Invalid(
                    "UNICODE requires an integer argument".into(),
                ));
            };
            let value = u32::try_from(value)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| DimError::Invalid("UNICODE argument is out of range".into()))?;
            Ok(ConstantValue::String(value.to_string()))
        }
        ExprKind::Formatted(formatted) => evaluate_formatted(formatted, evaluation),
        _ => Err(DimError::Invalid(
            "initializer must be a load-time constant".into(),
        )),
    }
}

fn evaluate_string_length(
    name: &str,
    arguments: &[Option<Expr>],
    evaluation: &ConstantEvaluation<'_>,
) -> Result<ConstantValue, DimError> {
    let name = name.to_ascii_uppercase();
    let argument = arguments[0]
        .as_ref()
        .ok_or_else(|| DimError::Invalid(format!("{name} requires an argument")))?;
    let ConstantValue::String(value) = evaluate_constant(argument, evaluation)? else {
        return Err(DimError::Invalid(format!(
            "{name} requires a constant string argument"
        )));
    };
    let length = if name == "STRLENS" {
        evaluation.index_resolver.legacy_encoded_len(&value)
    } else {
        value.encode_utf16().count()
    };
    Ok(ConstantValue::Integer(
        i64::try_from(length).unwrap_or(i64::MAX),
    ))
}

fn evaluate_varsize(
    arguments: &[Option<Expr>],
    evaluation: &ConstantEvaluation<'_>,
) -> Result<ConstantValue, DimError> {
    if !(1..=2).contains(&arguments.len()) {
        return Err(DimError::Invalid(
            "VARSIZE requires one or two arguments".into(),
        ));
    }
    let variable_argument = arguments[0]
        .as_ref()
        .ok_or_else(|| DimError::Invalid("VARSIZE variable name cannot be omitted".into()))?;
    let ConstantValue::String(variable_name) = evaluate_constant(variable_argument, evaluation)?
    else {
        return Err(DimError::Invalid(
            "VARSIZE variable name must be a constant string".into(),
        ));
    };
    let dimensions = evaluation
        .variable_dimensions
        .get(&normalize(&variable_name, evaluation.options.ignore_case))
        .ok_or_else(|| DimError::UnknownConstant(variable_name.clone()))?;
    let mut dimension = if let Some(argument) = arguments.get(1) {
        let argument = argument
            .as_ref()
            .ok_or_else(|| DimError::Invalid("VARSIZE dimension cannot be omitted".into()))?;
        let ConstantValue::Integer(value) = evaluate_constant(argument, evaluation)? else {
            return Err(DimError::Invalid(
                "VARSIZE dimension must be a constant integer".into(),
            ));
        };
        value
    } else {
        0
    };
    if evaluation.options.varsize_dimension_is_one_based && dimension > 0 {
        dimension -= 1;
    }
    let dimension = usize::try_from(dimension)
        .map_err(|_| DimError::Invalid("VARSIZE dimension must be non-negative".into()))?;
    let length = dimensions
        .get(dimension)
        .copied()
        .ok_or_else(|| DimError::Invalid("VARSIZE dimension exceeds the variable rank".into()))?;
    Ok(ConstantValue::Integer(
        i64::try_from(length).unwrap_or(i64::MAX),
    ))
}

fn evaluate_getnum(
    arguments: &[Option<Expr>],
    evaluation: &ConstantEvaluation<'_>,
) -> Result<ConstantValue, DimError> {
    if !(2..=3).contains(&arguments.len()) {
        return Err(DimError::Invalid(
            "GETNUM requires two or three arguments".into(),
        ));
    }
    let variable = arguments[0]
        .as_ref()
        .and_then(constant_variable_name)
        .ok_or_else(|| DimError::Invalid("GETNUM argument 1 must be a variable name".into()))?;
    if !evaluation
        .variable_dimensions
        .contains_key(&normalize(variable, evaluation.options.ignore_case))
    {
        return Err(DimError::UnknownConstant(variable.into()));
    }
    let key_argument = arguments[1]
        .as_ref()
        .ok_or_else(|| DimError::Invalid("GETNUM key cannot be omitted".into()))?;
    let ConstantValue::String(key) = evaluate_constant(key_argument, evaluation)? else {
        return Err(DimError::Invalid(
            "GETNUM key must be a constant string".into(),
        ));
    };
    let dimension = if let Some(argument) = arguments.get(2) {
        let argument = argument
            .as_ref()
            .ok_or_else(|| DimError::Invalid("GETNUM dimension cannot be omitted".into()))?;
        let ConstantValue::Integer(value) = evaluate_constant(argument, evaluation)? else {
            return Err(DimError::Invalid(
                "GETNUM dimension must be a constant integer".into(),
            ));
        };
        let value = if value > 0 { value - 1 } else { value };
        usize::try_from(value)
            .map_err(|_| DimError::Invalid("GETNUM dimension must be non-negative".into()))?
    } else {
        0
    };
    Ok(ConstantValue::Integer(
        evaluation
            .index_resolver
            .resolve(variable, dimension, &key)
            .unwrap_or(-1),
    ))
}

fn constant_variable_name(expression: &Expr) -> Option<&str> {
    match &expression.kind {
        ExprKind::Identifier(name) => Some(name),
        ExprKind::Variable { name, indices } if indices.is_empty() => Some(name),
        ExprKind::Group(inner) => constant_variable_name(inner),
        _ => None,
    }
}

fn evaluate_formatted(
    formatted: &FormattedString,
    evaluation: &ConstantEvaluation<'_>,
) -> Result<ConstantValue, DimError> {
    let mut result = String::new();
    for part in &formatted.parts {
        match part {
            FormPart::Text(value) => result.push_str(value),
            FormPart::StringInterpolation { expression, .. } => {
                match evaluate_constant(expression, evaluation)? {
                    ConstantValue::String(value) => result.push_str(&value),
                    ConstantValue::Integer(value) => result.push_str(&value.to_string()),
                }
            }
            FormPart::IntegerInterpolation { expression, .. } => {
                let ConstantValue::Integer(value) = evaluate_constant(expression, evaluation)?
                else {
                    return Err(DimError::Invalid(
                        "integer interpolation requires an integer".into(),
                    ));
                };
                result.push_str(&value.to_string());
            }
            FormPart::Conditional {
                condition,
                then_value,
                else_value,
                ..
            } => {
                let ConstantValue::Integer(condition) = evaluate_constant(condition, evaluation)?
                else {
                    return Err(DimError::Invalid(
                        "formatted condition requires an integer".into(),
                    ));
                };
                let selected = if condition != 0 {
                    Some(then_value.as_ref())
                } else {
                    else_value.as_deref()
                };
                if let Some(selected) = selected {
                    let ConstantValue::String(value) = evaluate_formatted(selected, evaluation)?
                    else {
                        unreachable!("formatted evaluation always returns a string");
                    };
                    result.push_str(&value);
                }
            }
            FormPart::Triple { .. } => {
                return Err(DimError::Invalid(
                    "triple interpolation is not a load-time constant".into(),
                ));
            }
        }
    }
    Ok(ConstantValue::String(result))
}

#[allow(clippy::too_many_lines)]
fn evaluate_binary(
    op: BinaryOp,
    left: ConstantValue,
    right: ConstantValue,
) -> Result<ConstantValue, DimError> {
    if let (ConstantValue::String(left), ConstantValue::String(right)) = (&left, &right) {
        return match op {
            BinaryOp::Add => Ok(ConstantValue::String(format!("{left}{right}"))),
            BinaryOp::Equal => Ok(ConstantValue::Integer(i64::from(left == right))),
            BinaryOp::NotEqual => Ok(ConstantValue::Integer(i64::from(left != right))),
            BinaryOp::Less => Ok(ConstantValue::Integer(i64::from(left < right))),
            BinaryOp::LessEqual => Ok(ConstantValue::Integer(i64::from(left <= right))),
            BinaryOp::Greater => Ok(ConstantValue::Integer(i64::from(left > right))),
            BinaryOp::GreaterEqual => Ok(ConstantValue::Integer(i64::from(left >= right))),
            _ => Err(DimError::Invalid("invalid string constant operator".into())),
        };
    }
    let (ConstantValue::Integer(left), ConstantValue::Integer(right)) = (left, right) else {
        return Err(DimError::Invalid("constant operand types differ".into()));
    };
    let value = match op {
        BinaryOp::Multiply => left.wrapping_mul(right),
        BinaryOp::Divide if right != 0 => left.wrapping_div(right),
        BinaryOp::Modulo if right != 0 => left.wrapping_rem(right),
        BinaryOp::Add => left.wrapping_add(right),
        BinaryOp::Subtract => left.wrapping_sub(right),
        BinaryOp::ShiftLeft => left.wrapping_shl(u32::try_from(right).unwrap_or_default()),
        BinaryOp::ShiftRight => left.wrapping_shr(u32::try_from(right).unwrap_or_default()),
        BinaryOp::Less => i64::from(left < right),
        BinaryOp::LessEqual => i64::from(left <= right),
        BinaryOp::Greater => i64::from(left > right),
        BinaryOp::GreaterEqual => i64::from(left >= right),
        BinaryOp::Equal => i64::from(left == right),
        BinaryOp::NotEqual => i64::from(left != right),
        BinaryOp::BitAnd => left & right,
        BinaryOp::BitXor => left ^ right,
        BinaryOp::BitOr => left | right,
        BinaryOp::LogicalAnd => i64::from(left != 0 && right != 0),
        BinaryOp::LogicalXor => i64::from((left != 0) ^ (right != 0)),
        BinaryOp::LogicalOr => i64::from(left != 0 || right != 0),
        BinaryOp::Nand => i64::from(!(left != 0 && right != 0)),
        BinaryOp::Nor => i64::from(!(left != 0 || right != 0)),
        BinaryOp::Divide | BinaryOp::Modulo => {
            return Err(DimError::Invalid("division by zero".into()));
        }
    };
    Ok(ConstantValue::Integer(value))
}

fn add_registrations(schema: &VariableSchema, registrations: &mut Vec<UserIndexRegistration>) {
    if schema.dimensions.len() == 1 {
        registrations.push(UserIndexRegistration {
            variable_name: schema.id.name().to_owned(),
            source_stem: schema.id.name().to_owned(),
            dimension: None,
            length: schema.dimensions[0],
        });
    } else {
        for (index, length) in schema.dimensions.iter().copied().enumerate() {
            let dimension = index + 1;
            registrations.push(UserIndexRegistration {
                variable_name: schema.id.name().to_owned(),
                source_stem: format!("{}@{dimension}", schema.id.name()),
                dimension: Some(dimension),
                length,
            });
        }
    }
}

fn normalize(name: &str, ignore_case: bool) -> String {
    if ignore_case {
        name.to_ascii_uppercase()
    } else {
        name.to_owned()
    }
}

fn at_input(
    input: &DeclarationInput<'_>,
    code: AnalyzerDiagnosticCode,
    message: impl Into<String>,
) -> AnalyzerDiagnostic {
    AnalyzerDiagnostic::at(
        code,
        AnalyzerDiagnosticSeverity::Error,
        2,
        input.source,
        input.path,
        input.text,
        input.directive.span,
        message,
    )
}
