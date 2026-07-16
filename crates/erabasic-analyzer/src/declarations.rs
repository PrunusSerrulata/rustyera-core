use std::collections::BTreeMap;

use erabasic_ast::{BinaryOp, Directive, Expr, ExprKind, UnaryOp};
use erabasic_data::{
    Persistence, ProjectData, StorageScope, UserIndexRegistration, ValueType,
    VariableId as DataVariableId, VariableSchema,
};
use erabasic_hir::{ConstantValue, SourceId, SourceLocation};
use erabasic_parser::{ParserContext, parse_expression};

use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, AnalyzerOptions,
    symbols::is_reserved,
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
            match parse_dim(input, false, context, &constants, options) {
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
    options: &AnalyzerOptions,
) -> Result<DeclaredVariable, String> {
    parse_dim(input, true, context, &BTreeMap::new(), options).map_err(|error| error.to_string())
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

#[allow(clippy::too_many_lines)]
fn parse_dim(
    input: &DeclarationInput<'_>,
    private: bool,
    context: &dyn ParserContext,
    constants: &BTreeMap<String, ConstantValue>,
    options: &AnalyzerOptions,
) -> Result<DeclaredVariable, DimError> {
    let is_string = input.directive.name == "DIMS";
    let mut rest = input.directive.raw_arguments.as_str();
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
            let value = parse_constant(segment, context, constants, options)?;
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
        for segment in split_top_level(initializer, ',') {
            if segment.trim().is_empty() {
                return Err(DimError::InvalidInitializer(
                    "array initializers cannot be omitted".into(),
                ));
            }
            let value = parse_constant(segment, context, constants, options)?;
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

fn parse_constant(
    source: &str,
    context: &dyn ParserContext,
    constants: &BTreeMap<String, ConstantValue>,
    options: &AnalyzerOptions,
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
    evaluate_constant(&expression, constants, options)
}

fn evaluate_constant(
    expression: &Expr,
    constants: &BTreeMap<String, ConstantValue>,
    options: &AnalyzerOptions,
) -> Result<ConstantValue, DimError> {
    match &expression.kind {
        ExprKind::Integer(value) => Ok(ConstantValue::Integer(*value)),
        ExprKind::String(value) => Ok(ConstantValue::String(value.clone())),
        ExprKind::Identifier(name) => constants
            .get(&normalize(name, options.ignore_case))
            .cloned()
            .ok_or_else(|| DimError::UnknownConstant(name.clone())),
        ExprKind::Group(inner) => evaluate_constant(inner, constants, options),
        ExprKind::Unary { op, operand } => {
            let ConstantValue::Integer(value) = evaluate_constant(operand, constants, options)?
            else {
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
            let left = evaluate_constant(left, constants, options)?;
            let right = evaluate_constant(right, constants, options)?;
            evaluate_binary(*op, left, right)
        }
        ExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            let ConstantValue::Integer(condition) =
                evaluate_constant(condition, constants, options)?
            else {
                return Err(DimError::Invalid("integer condition required".into()));
            };
            evaluate_constant(
                if condition != 0 { then_expr } else { else_expr },
                constants,
                options,
            )
        }
        _ => Err(DimError::Invalid(
            "initializer must be a load-time constant".into(),
        )),
    }
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
