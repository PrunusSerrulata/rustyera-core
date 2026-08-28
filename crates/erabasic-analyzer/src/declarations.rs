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
    expression::IndexResolver, identifiers::identifier_key, symbols::is_reserved,
};

mod registrations;

use registrations::add_registrations;

mod constants;

pub(crate) use constants::ConstantWarnings;
use constants::{ConstantEvaluation, parse_constant};

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
    pub arithmetic_diagnostics: Vec<AnalyzerDiagnostic>,
}

pub(crate) struct ScopedDeclaration {
    pub declaration: DeclaredVariable,
    pub initializer: Option<String>,
    pub initializer_offset: Option<usize>,
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
    progress: Option<&dyn crate::project::AnalysisProgressCallback>,
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

    let progress = crate::project::ProgressCounter::new(
        crate::project::AnalysisProgressStage::DeclaringGlobals,
        pending.len(),
        progress,
    );
    // Emuera queues every header DIM and retries the queue while another declaration
    // was resolved. This permits dimensions to refer to constants declared later.
    while !pending.is_empty() {
        let before = pending.len();
        let mut deferred = Vec::new();
        for input in pending {
            let parsed = parse_dim(
                input,
                false,
                context,
                &constants,
                &variable_dimensions,
                &index_resolver,
                options,
            );
            // Count declarations only once they resolve or produce a final error.
            if !matches!(&parsed, Err(DimError::UnknownConstant(_))) {
                progress.advance();
            }
            match parsed {
                Ok(variable) => {
                    diagnostics.extend(variable.arithmetic_diagnostics.iter().cloned());
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
                progress.advance();
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

pub(crate) fn parse_integer_constant(
    source: &str,
    context: &dyn ParserContext,
    constants: &BTreeMap<String, ConstantValue>,
    variable_dimensions: &BTreeMap<String, Vec<usize>>,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
) -> Result<(i64, ConstantWarnings), String> {
    let evaluation = ConstantEvaluation {
        constants,
        variable_dimensions,
        index_resolver,
        options,
        warnings: std::cell::RefCell::default(),
    };
    let value = parse_constant(strip_declaration_comment(source), context, &evaluation)
        .map_err(|error| error.to_string())?;
    let ConstantValue::Integer(value) = value else {
        return Err("an integer constant expression is required".into());
    };
    Ok((value, evaluation.warnings.into_inner()))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn parse_scoped_declaration(
    source: SourceId,
    path: &str,
    text: &str,
    instruction: &str,
    raw_arguments: &str,
    span: erabasic_ast::Span,
    context: &dyn ParserContext,
    constants: &BTreeMap<String, ConstantValue>,
    variable_dimensions: &BTreeMap<String, Vec<usize>>,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
) -> Result<ScopedDeclaration, String> {
    let raw = strip_declaration_comment(raw_arguments);
    let assignment = find_top_level(raw, '=');
    let declaration_arguments = assignment.map_or(raw, |index| &raw[..index]);
    let initializer = assignment.and_then(|index| {
        let tail = &raw[index + 1..];
        let leading = tail.len().saturating_sub(tail.trim_start().len());
        let value = tail.trim();
        (!value.is_empty()).then(|| (value.to_owned(), index + 1 + leading))
    });
    let directive = Directive {
        name: if instruction.eq_ignore_ascii_case("VARS") {
            "DIMS".into()
        } else {
            "DIM".into()
        },
        arguments: Vec::new(),
        raw_arguments: format!("DYNAMIC {declaration_arguments}"),
        span,
    };
    let input = DeclarationInput {
        source,
        path,
        text,
        directive: &directive,
    };
    let declaration = parse_private_declaration(
        &input,
        context,
        constants,
        variable_dimensions,
        index_resolver,
        options,
    )?;
    Ok(ScopedDeclaration {
        declaration,
        initializer: initializer.as_ref().map(|(value, _)| value.clone()),
        initializer_offset: initializer.map(|(_, offset)| offset),
    })
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
        warnings: std::cell::RefCell::default(),
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
        arithmetic_diagnostics: constant_warnings(
            constant_evaluation.warnings.into_inner(),
            input.source,
            input.path,
            input.text,
            input.directive.span,
        ),
    })
}

pub(crate) fn constant_warnings(
    warnings: ConstantWarnings,
    source: SourceId,
    path: &str,
    text: &str,
    span: erabasic_ast::Span,
) -> Vec<AnalyzerDiagnostic> {
    warnings
        .into_iter()
        .map(|(warning, message)| {
            AnalyzerDiagnostic::at(
                match warning {
                    erabasic_compat::IntegerArithmeticWarning::Overflow => {
                        AnalyzerDiagnosticCode::IntegerOverflow
                    }
                    erabasic_compat::IntegerArithmeticWarning::DivideByZero => {
                        AnalyzerDiagnosticCode::IntegerDivideByZero
                    }
                },
                AnalyzerDiagnosticSeverity::Warning,
                1,
                source,
                path,
                text,
                span,
                message,
            )
        })
        .collect()
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

fn normalize(name: &str, ignore_case: bool) -> String {
    identifier_key(name, ignore_case)
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
