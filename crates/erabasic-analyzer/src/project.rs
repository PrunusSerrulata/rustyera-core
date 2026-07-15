use std::collections::{BTreeMap, BTreeSet, VecDeque};

use erabasic_ast::{
    Argument, Diagnostic, Expr, ExprKind, Function as AstFunction, ParseOutput, Script, Severity,
    SourceKind, Statement, StatementKind, VariableRef,
};
use erabasic_csv::{CsvDiagnosticSeverity, CsvLoadOptions, resolve_deferred_indices};
use erabasic_data::ProjectData;
use erabasic_hir::{
    EventAttributes, Function, FunctionId, FunctionKind, HirArgument, HirExprKind, HirStatement,
    HirStatementKind, InstructionTarget, LabelId, LineId, Parameter, Program, SemanticType,
    SourceFile, SourceId, SourceLocation,
};
use erabasic_parser::{parse_erb, parse_erh};
use serde::{Deserialize, Serialize};

use crate::{
    AnalysisInput, AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity,
    AnalyzerOptions, ExtensionRegistry, SourceIoErrorKind, SourcePayload, WarningPolicy,
    catalog::Catalog,
    context::AnalysisParserContext,
    control_flow::build_control_flow,
    declarations::{DeclarationInput, analyze_global_declarations, parse_private_declaration},
    expression::{ExpressionAnalyzer, IndexResolver},
    symbols::{Symbols, is_reserved},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParsedProjectSource {
    pub source: SourceFile,
    pub text: String,
    pub script: Script,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalyzedProject {
    pub data: ProjectData,
    pub program: Program,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub project: Option<AnalyzedProject>,
    pub diagnostics: Vec<AnalyzerDiagnostic>,
}

#[must_use]
pub fn analyze_project(
    input: AnalysisInput,
    options: &AnalyzerOptions,
    extensions: &ExtensionRegistry,
) -> AnalysisReport {
    let mut diagnostics = Vec::new();
    if !validate_extensions(extensions, &mut diagnostics) {
        return AnalysisReport {
            project: None,
            diagnostics,
        };
    }
    let Some(indexed) = index_sources(input.sources, options, &mut diagnostics) else {
        return AnalysisReport {
            project: None,
            diagnostics,
        };
    };
    let catalog = Catalog::build(extensions);
    let mut context = AnalysisParserContext::new(
        &input.project_data.schema,
        &catalog,
        std::iter::empty(),
        options,
    );
    let mut parsed = Vec::new();
    for source in indexed {
        let output = match source.kind {
            SourceKind::Erh => parse_erh(&source.text, &mut context),
            SourceKind::Erb => parse_erb(&source.text, &mut context),
        };
        append_parser_diagnostics(
            &mut diagnostics,
            source.id,
            &source.path,
            &source.text,
            &output,
        );
        if let Some(script) = output.value {
            let source_file = source_file(source.id, source.path, source.kind, &source.text);
            parsed.push(ParsedProjectSource {
                source: source_file,
                text: source.text,
                script,
            });
        }
    }
    analyze_with_context(
        input.project_data,
        &parsed,
        options,
        extensions,
        &catalog,
        &context,
        diagnostics,
    )
}

#[must_use]
pub fn analyze_parsed_project(
    project_data: ProjectData,
    sources: &[ParsedProjectSource],
    options: &AnalyzerOptions,
    extensions: &ExtensionRegistry,
) -> AnalysisReport {
    let mut diagnostics = Vec::new();
    if !validate_extensions(extensions, &mut diagnostics) {
        return AnalysisReport {
            project: None,
            diagnostics,
        };
    }
    let catalog = Catalog::build(extensions);
    let mut context = AnalysisParserContext::new(
        &project_data.schema,
        &catalog,
        sources
            .iter()
            .flat_map(|source| source.script.functions.iter())
            .map(|function| function.name.clone()),
        options,
    );
    // Replay headers only to reconstruct their macro environment. The submitted AST
    // remains authoritative and is not replaced by this parse.
    for source in sources
        .iter()
        .filter(|source| source.source.kind == SourceKind::Erh)
    {
        let _ = parse_erh(&source.text, &mut context);
    }
    analyze_with_context(
        project_data,
        sources,
        options,
        extensions,
        &catalog,
        &context,
        diagnostics,
    )
}

#[allow(clippy::too_many_lines)]
fn analyze_with_context(
    mut project_data: ProjectData,
    sources: &[ParsedProjectSource],
    options: &AnalyzerOptions,
    _extensions: &ExtensionRegistry,
    catalog: &Catalog,
    context: &AnalysisParserContext,
    mut diagnostics: Vec<AnalyzerDiagnostic>,
) -> AnalysisReport {
    let declarations: Vec<_> = sources
        .iter()
        .filter(|source| source.source.kind == SourceKind::Erh)
        .flat_map(|source| {
            source
                .script
                .declarations
                .iter()
                .map(move |directive| DeclarationInput {
                    source: source.source.id,
                    path: &source.source.relative_path,
                    text: &source.text,
                    directive,
                })
        })
        .collect();
    for declaration in &declarations {
        if matches!(
            declaration.directive.name.as_str(),
            "FUNCTION" | "FUNCTIONS"
        ) {
            diagnostics.push(AnalyzerDiagnostic::at(
                AnalyzerDiagnosticCode::UnsupportedReferenceFeature,
                AnalyzerDiagnosticSeverity::Error,
                2,
                declaration.source,
                declaration.path,
                declaration.text,
                declaration.directive.span,
                "the pinned reference does not implement #FUNCTION in ERH files",
            ));
        }
    }
    let declaration_output = analyze_global_declarations(
        &mut project_data,
        &declarations,
        context,
        options,
        &mut diagnostics,
    );
    if options.use_erd {
        let csv_options = CsvLoadOptions {
            ignore_case: options.ignore_case,
            use_erd: options.use_erd,
            debug_mode: options.debug_mode,
            allow_full_width_space: options.allow_full_width_space,
            sort_with_filename: options.sort_with_filename,
            ..CsvLoadOptions::default()
        };
        for diagnostic in resolve_deferred_indices(
            &mut project_data,
            &declaration_output.registrations,
            &csv_options,
        ) {
            diagnostics.push(map_csv_diagnostic(diagnostic));
        }
    }

    let index_resolver = IndexResolver::new(&project_data);
    let mut symbols = Symbols::new(&project_data.schema, &declaration_output.variables, options);
    let mut definitions = Vec::new();
    for (source_index, source) in sources.iter().enumerate() {
        if source.source.kind != SourceKind::Erb {
            continue;
        }
        for (function_index, function) in source.script.functions.iter().enumerate() {
            let (kind, return_type) = function_semantics(function);
            if is_reserved(&function.name) {
                diagnostics.push(at_function(
                    source,
                    function,
                    AnalyzerDiagnosticCode::ReservedName,
                    format!("{} is a reserved identifier", function.name),
                ));
            }
            if catalog
                .functions
                .contains_key(&key(&function.name, options.ignore_case))
                && !options.allow_function_overloading
            {
                diagnostics.push(at_function(
                    source,
                    function,
                    AnalyzerDiagnosticCode::DuplicateSymbol,
                    format!(
                        "{} conflicts with a built-in expression function",
                        function.name
                    ),
                ));
            }
            match symbols.register_function(&function.name, kind, return_type) {
                Ok(id) => definitions.push(FunctionDefinition {
                    source_index,
                    function_index,
                    id,
                    kind,
                    return_type,
                    definition_order: u32::try_from(definitions.len()).unwrap_or(u32::MAX),
                }),
                Err(_) => diagnostics.push(at_function(
                    source,
                    function,
                    AnalyzerDiagnosticCode::DuplicateSymbol,
                    format!("function {} is already declared", function.name),
                )),
            }
        }
    }

    let reachable = reachable_functions(sources, &definitions, &symbols, options);
    let mut functions = Vec::new();
    for definition in &definitions {
        let source = &sources[definition.source_index];
        let function = &source.script.functions[definition.function_index];
        symbols.prepare_function_locals(definition.id, &function.name);
        register_private_variables(
            definition.id,
            source,
            function,
            &mut symbols,
            context,
            options,
            &mut diagnostics,
        );
        let should_analyze = options.analysis_mode
            || !options.ignore_uncalled_functions
            || reachable.contains(&definition.id);
        let hir = if should_analyze {
            analyze_function(
                definition.id,
                definition.kind,
                definition.return_type,
                definition.definition_order,
                source,
                function,
                &symbols,
                catalog,
                context,
                &index_resolver,
                options,
                &mut diagnostics,
            )
        } else {
            uncalled_function(
                definition.id,
                definition.kind,
                definition.return_type,
                definition.definition_order,
                source,
                function,
            )
        };
        if !reachable.contains(&definition.id) {
            report_uncalled(source, function, options, &mut diagnostics);
        }
        functions.push(hir);
    }

    for source in sources
        .iter()
        .filter(|source| source.source.kind == SourceKind::Erb)
    {
        for statement in &source.script.top_level {
            diagnostics.push(AnalyzerDiagnostic::at(
                AnalyzerDiagnosticCode::InvalidControlFlow,
                AnalyzerDiagnosticSeverity::Warning,
                1,
                source.source.id,
                &source.source.relative_path,
                &source.text,
                statement.span,
                "executable line appears before the first function label",
            ));
        }
    }

    let mut program = Program::new(sources.iter().map(|source| source.source.clone()).collect());
    program.variables = symbols.variables;
    program.functions = functions;
    diagnostics.sort_by_key(|diagnostic| {
        diagnostic.source.as_ref().map_or(
            (u32::MAX, usize::MAX, diagnostic.reference_level),
            |source| {
                (
                    source.source.0,
                    source.byte_start,
                    diagnostic.reference_level,
                )
            },
        )
    });
    AnalysisReport {
        project: Some(AnalyzedProject {
            data: project_data,
            program,
        }),
        diagnostics,
    }
}

struct FunctionDefinition {
    source_index: usize,
    function_index: usize,
    id: FunctionId,
    kind: FunctionKind,
    return_type: SemanticType,
    definition_order: u32,
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
fn analyze_function(
    id: FunctionId,
    kind: FunctionKind,
    return_type: SemanticType,
    definition_order: u32,
    source: &ParsedProjectSource,
    function: &AstFunction,
    symbols: &Symbols,
    catalog: &Catalog,
    _context: &AnalysisParserContext,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> Function {
    let mut parameters = Vec::new();
    for parameter in &function.parameters {
        let target = parameter.target.clone().unwrap_or_else(|| VariableRef {
            name: parameter.name.clone(),
            indices: Vec::new(),
            span: parameter.span,
        });
        let target_expression = Expr {
            kind: ExprKind::Variable {
                name: target.name.clone(),
                indices: target.indices,
            },
            span: target.span,
        };
        let analyzed_target = ExpressionAnalyzer {
            symbols,
            catalog,
            options,
            function: id,
            source: source.source.id,
            path: &source.source.relative_path,
            text: &source.text,
            diagnostics,
            index_resolver,
        }
        .analyze(&target_expression);
        let HirExprKind::Variable { place } = analyzed_target.kind else {
            diagnostics.push(AnalyzerDiagnostic::at(
                AnalyzerDiagnosticCode::InvalidArgument,
                AnalyzerDiagnosticSeverity::Error,
                2,
                source.source.id,
                &source.source.relative_path,
                &source.text,
                parameter.span,
                format!("function parameter {} is not a variable", parameter.name),
            ));
            continue;
        };
        let default = parameter.default.as_ref().map(|expression| {
            ExpressionAnalyzer {
                symbols,
                catalog,
                options,
                function: id,
                source: source.source.id,
                path: &source.source.relative_path,
                text: &source.text,
                diagnostics,
                index_resolver,
            }
            .analyze(expression)
        });
        if let Some(default) = &default
            && default.value_type != place.value_type
            && default.value_type != SemanticType::Error
        {
            diagnostics.push(AnalyzerDiagnostic::at(
                AnalyzerDiagnosticCode::TypeMismatch,
                AnalyzerDiagnosticSeverity::Error,
                2,
                source.source.id,
                &source.source.relative_path,
                &source.text,
                parameter.span,
                "parameter default type does not match its variable",
            ));
        }
        parameters.push(Parameter {
            target: place,
            default,
        });
    }

    let mut lines = Vec::new();
    let mut next_label = 0u32;
    for statement in &function.body {
        let line_id = LineId(u32::try_from(lines.len()).expect("too many lines"));
        lines.push(analyze_statement(
            line_id,
            &mut next_label,
            id,
            source,
            statement,
            symbols,
            catalog,
            index_resolver,
            options,
            diagnostics,
        ));
    }
    let (labels, control_flow) = build_control_flow(
        &lines,
        symbols,
        source.source.id,
        &source.source.relative_path,
        &source.text,
        diagnostics,
    );
    Function {
        id,
        name: function.name.clone(),
        kind,
        event_attributes: event_attributes(kind, function),
        definition_order,
        return_type,
        parameters,
        lines,
        labels,
        control_flow,
        location: SourceLocation::new(source.source.id, function.span),
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn analyze_statement(
    line_id: LineId,
    next_label: &mut u32,
    function: FunctionId,
    source: &ParsedProjectSource,
    statement: &Statement,
    symbols: &Symbols,
    catalog: &Catalog,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> HirStatement {
    let location = SourceLocation::new(source.source.id, statement.span);
    let kind = match &statement.kind {
        StatementKind::Assignment { target, op, value } => {
            let target_expression = Expr {
                kind: ExprKind::Variable {
                    name: target.name.clone(),
                    indices: target.indices.clone(),
                },
                span: target.span,
            };
            let mut analyzer = ExpressionAnalyzer {
                symbols,
                catalog,
                options,
                function,
                source: source.source.id,
                path: &source.source.relative_path,
                text: &source.text,
                diagnostics,
                index_resolver,
            };
            let analyzed_target = analyzer.analyze(&target_expression);
            let analyzed_value = analyzer.analyze(value);
            let HirExprKind::Variable { place } = analyzed_target.kind else {
                return HirStatement {
                    id: line_id,
                    kind: HirStatementKind::Error,
                    location,
                };
            };
            if !place.mutable {
                diagnostics.push(AnalyzerDiagnostic::at(
                    AnalyzerDiagnosticCode::InvalidAssignment,
                    AnalyzerDiagnosticSeverity::Error,
                    2,
                    source.source.id,
                    &source.source.relative_path,
                    &source.text,
                    target.span,
                    "assignment target is immutable",
                ));
            }
            if place.value_type != analyzed_value.value_type
                && analyzed_value.value_type != SemanticType::Error
            {
                diagnostics.push(AnalyzerDiagnostic::at(
                    AnalyzerDiagnosticCode::TypeMismatch,
                    AnalyzerDiagnosticSeverity::Error,
                    2,
                    source.source.id,
                    &source.source.relative_path,
                    &source.text,
                    value.span,
                    "assignment value type does not match its target",
                ));
            }
            HirStatementKind::Assignment {
                target: place,
                op: *op,
                value: analyzed_value,
            }
        }
        StatementKind::Instruction {
            name,
            arguments,
            raw_arguments,
        } => analyze_instruction(
            function,
            source,
            statement,
            name,
            arguments,
            raw_arguments,
            symbols,
            catalog,
            index_resolver,
            options,
            diagnostics,
        ),
        StatementKind::GotoLabel { name } => {
            let label = LabelId(*next_label);
            *next_label += 1;
            HirStatementKind::Label {
                label,
                name: name.clone(),
            }
        }
        StatementKind::Directive(_) | StatementKind::Invalid => HirStatementKind::Error,
    };
    HirStatement {
        id: line_id,
        kind,
        location,
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
fn analyze_instruction(
    function: FunctionId,
    source: &ParsedProjectSource,
    statement: &Statement,
    name: &str,
    arguments: &[Argument],
    raw_arguments: &str,
    symbols: &Symbols,
    catalog: &Catalog,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> HirStatementKind {
    let key = key(name, options.ignore_case);
    let signature = catalog.instructions.get(&key);
    if signature.is_none() {
        diagnostics.push(AnalyzerDiagnostic::at(
            AnalyzerDiagnosticCode::UnknownInstruction,
            AnalyzerDiagnosticSeverity::Error,
            2,
            source.source.id,
            &source.source.relative_path,
            &source.text,
            statement.span,
            format!("unknown instruction {name}"),
        ));
    }
    let static_target = matches!(
        key.as_str(),
        "CALL" | "CALLF" | "JUMP" | "BEGIN" | "TRYCALL" | "TRYJUMP" | "GOTO" | "TRYGOTO"
    );
    let mut analyzer = ExpressionAnalyzer {
        symbols,
        catalog,
        options,
        function,
        source: source.source.id,
        path: &source.source.relative_path,
        text: &source.text,
        diagnostics,
        index_resolver,
    };
    let mut lowered = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        if static_target && index == 0 {
            lowered.push(HirArgument::Raw(
                raw_arguments
                    .split(',')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned(),
            ));
            continue;
        }
        lowered.push(match argument {
            Argument::Expression(expression) => {
                let expression = analyzer.analyze(expression);
                let mutable = signature
                    .and_then(|signature| signature.arguments.get(index))
                    .is_some_and(|constraint| {
                        matches!(
                            constraint,
                            crate::ArgumentConstraint::MutableInteger
                                | crate::ArgumentConstraint::MutableString
                                | crate::ArgumentConstraint::MutableAny
                        )
                    });
                if mutable {
                    if let HirExprKind::Variable { place } = expression.kind {
                        HirArgument::Place(place)
                    } else {
                        HirArgument::Expression(expression)
                    }
                } else {
                    HirArgument::Expression(expression)
                }
            }
            Argument::Formatted(formatted) => {
                HirArgument::Formatted(analyzer.analyze_formatted(formatted))
            }
            Argument::Raw(value) => HirArgument::Raw(value.clone()),
            Argument::Omitted(_) => HirArgument::Omitted,
        });
    }
    if static_target && lowered.is_empty() && !raw_arguments.trim().is_empty() {
        lowered.push(HirArgument::Raw(
            raw_arguments
                .split(',')
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned(),
        ));
    }
    if let Some(signature) = signature {
        let expression_arguments: Vec<_> = lowered
            .iter()
            .map(|argument| match argument {
                HirArgument::Expression(expression) => Some(expression.clone()),
                HirArgument::Place(place) => Some(erabasic_hir::HirExpr {
                    kind: HirExprKind::Variable {
                        place: place.clone(),
                    },
                    value_type: place.value_type,
                    constant: None,
                    location: place.location,
                }),
                HirArgument::Omitted | HirArgument::Formatted(_) | HirArgument::Raw(_) => None,
            })
            .collect();
        if !matches!(
            signature.argument_style,
            erabasic_parser::ArgumentStyle::Formatted | erabasic_parser::ArgumentStyle::Raw
        ) && !static_target
        {
            analyzer.check_arguments(
                &expression_arguments,
                &signature.arguments,
                signature.minimum_arguments,
                signature.variadic,
                signature.allow_omitted,
                SourceLocation::new(source.source.id, statement.span),
            );
        } else if lowered.len() < signature.minimum_arguments {
            diagnostics.push(AnalyzerDiagnostic::at(
                AnalyzerDiagnosticCode::InvalidArgumentCount,
                AnalyzerDiagnosticSeverity::Error,
                2,
                source.source.id,
                &source.source.relative_path,
                &source.text,
                statement.span,
                format!(
                    "{} requires at least {} arguments",
                    key, signature.minimum_arguments
                ),
            ));
        }
    }
    let target = if signature.is_none() {
        InstructionTarget::Unresolved(key)
    } else if catalog.extension_instructions.contains(&key) {
        InstructionTarget::Extension(key)
    } else {
        InstructionTarget::Builtin(key)
    };
    HirStatementKind::Instruction {
        target,
        arguments: lowered,
    }
}

fn source_file(id: SourceId, relative_path: String, kind: SourceKind, text: &str) -> SourceFile {
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

fn register_private_variables(
    function_id: FunctionId,
    source: &ParsedProjectSource,
    function: &AstFunction,
    symbols: &mut Symbols,
    context: &AnalysisParserContext,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) {
    for directive in function
        .attributes
        .iter()
        .filter(|directive| matches!(directive.name.as_str(), "DIM" | "DIMS"))
    {
        let input = DeclarationInput {
            source: source.source.id,
            path: &source.source.relative_path,
            text: &source.text,
            directive,
        };
        match parse_private_declaration(&input, context, options) {
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
}

fn reachable_functions(
    sources: &[ParsedProjectSource],
    definitions: &[FunctionDefinition],
    symbols: &Symbols,
    options: &AnalyzerOptions,
) -> BTreeSet<FunctionId> {
    if options.analysis_mode || !options.ignore_uncalled_functions {
        return definitions.iter().map(|definition| definition.id).collect();
    }
    let mut reachable: BTreeSet<_> = definitions
        .iter()
        .filter(|definition| matches!(definition.kind, FunctionKind::Event | FunctionKind::System))
        .map(|definition| definition.id)
        .collect();
    let by_id: BTreeMap<_, _> = definitions
        .iter()
        .map(|definition| (definition.id, definition))
        .collect();
    let mut queue: VecDeque<_> = reachable.iter().copied().collect();
    while let Some(id) = queue.pop_front() {
        let Some(definition) = by_id.get(&id) else {
            continue;
        };
        let function =
            &sources[definition.source_index].script.functions[definition.function_index];
        if function.body.iter().any(uses_dynamic_call) {
            reachable.extend(definitions.iter().map(|definition| definition.id));
            break;
        }
        let mut calls = Vec::new();
        for statement in &function.body {
            collect_statement_calls(statement, &mut calls);
        }
        for call in calls {
            if let Some(target) = symbols.function(&call)
                && reachable.insert(target.id)
            {
                queue.push_back(target.id);
            }
        }
    }
    reachable
}

fn collect_statement_calls(statement: &Statement, calls: &mut Vec<String>) {
    match &statement.kind {
        StatementKind::Instruction {
            name,
            raw_arguments,
            arguments,
        } => {
            if matches!(
                name.as_str(),
                "CALL" | "CALLF" | "JUMP" | "BEGIN" | "TRYCALL" | "TRYJUMP"
            ) {
                let target = raw_arguments
                    .split(',')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .trim_matches('"');
                if !target.is_empty() {
                    calls.push(target.to_owned());
                }
            }
            for argument in arguments {
                if let Argument::Expression(expression) = argument {
                    collect_expression_calls(expression, calls);
                }
            }
        }
        StatementKind::Assignment { value, target, .. } => {
            collect_expression_calls(value, calls);
            for index in &target.indices {
                collect_expression_calls(index, calls);
            }
        }
        StatementKind::GotoLabel { .. } | StatementKind::Directive(_) | StatementKind::Invalid => {}
    }
}

fn collect_expression_calls(expression: &Expr, calls: &mut Vec<String>) {
    match &expression.kind {
        ExprKind::Call { name, args } => {
            calls.push(name.clone());
            for argument in args.iter().flatten() {
                collect_expression_calls(argument, calls);
            }
        }
        ExprKind::Variable { indices, .. } => {
            for index in indices {
                collect_expression_calls(index, calls);
            }
        }
        ExprKind::Unary { operand, .. }
        | ExprKind::Postfix { operand, .. }
        | ExprKind::Group(operand) => {
            collect_expression_calls(operand, calls);
        }
        ExprKind::Binary { left, right, .. } => {
            collect_expression_calls(left, calls);
            collect_expression_calls(right, calls);
        }
        ExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_expression_calls(condition, calls);
            collect_expression_calls(then_expr, calls);
            collect_expression_calls(else_expr, calls);
        }
        ExprKind::Integer(_)
        | ExprKind::String(_)
        | ExprKind::Identifier(_)
        | ExprKind::Formatted(_)
        | ExprKind::Error => {}
    }
}

fn uses_dynamic_call(statement: &Statement) -> bool {
    matches!(
        &statement.kind,
        StatementKind::Instruction { name, .. }
            if matches!(name.as_str(), "CALLFORM" | "CALLFORMF" | "JUMPFORM" | "TRYCALLFORM" | "TRYJUMPFORM")
    )
}

fn uncalled_function(
    id: FunctionId,
    kind: FunctionKind,
    return_type: SemanticType,
    definition_order: u32,
    source: &ParsedProjectSource,
    function: &AstFunction,
) -> Function {
    Function {
        id,
        name: function.name.clone(),
        kind,
        event_attributes: event_attributes(kind, function),
        definition_order,
        return_type,
        parameters: Vec::new(),
        lines: Vec::new(),
        labels: Vec::new(),
        control_flow: Vec::new(),
        location: SourceLocation::new(source.source.id, function.span),
    }
}

fn event_attributes(kind: FunctionKind, function: &AstFunction) -> EventAttributes {
    if kind != FunctionKind::Event {
        return EventAttributes::default();
    }
    let mut attributes = EventAttributes::default();
    for directive in &function.attributes {
        match directive.name.as_str() {
            "ONLY" if !attributes.only => {
                attributes = EventAttributes {
                    only: true,
                    ..EventAttributes::default()
                };
            }
            "PRI" if !attributes.only => attributes.priority = true,
            "LATER" if !attributes.only => attributes.later = true,
            "SINGLE" if !attributes.only => attributes.single = true,
            _ => {}
        }
    }
    attributes
}

fn report_uncalled(
    source: &ParsedProjectSource,
    function: &AstFunction,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) {
    if matches!(
        options.function_not_called,
        WarningPolicy::Ignore | WarningPolicy::Later
    ) {
        return;
    }
    diagnostics.push(AnalyzerDiagnostic::at(
        AnalyzerDiagnosticCode::UncalledFunction,
        AnalyzerDiagnosticSeverity::Warning,
        1,
        source.source.id,
        &source.source.relative_path,
        &source.text,
        function.span,
        format!("function {} is never called", function.name),
    ));
}

fn function_semantics(function: &AstFunction) -> (FunctionKind, SemanticType) {
    if function
        .attributes
        .iter()
        .any(|directive| directive.name == "FUNCTIONS")
    {
        return (FunctionKind::Method, SemanticType::String);
    }
    if function
        .attributes
        .iter()
        .any(|directive| directive.name == "FUNCTION")
    {
        return (FunctionKind::Method, SemanticType::Integer);
    }
    let upper = function.name.to_ascii_uppercase();
    if is_event_name(&upper) {
        (FunctionKind::Event, SemanticType::Void)
    } else if is_system_name(&upper) {
        (FunctionKind::System, SemanticType::Void)
    } else {
        (FunctionKind::Normal, SemanticType::Void)
    }
}

fn is_event_name(name: &str) -> bool {
    matches!(
        name,
        "EVENTFIRST"
            | "EVENTTRAIN"
            | "EVENTSHOP"
            | "EVENTBUY"
            | "EVENTCOM"
            | "EVENTTURNEND"
            | "EVENTCOMEND"
            | "EVENTEND"
            | "EVENTLOAD"
    )
}

fn is_system_name(name: &str) -> bool {
    is_event_name(name)
        || matches!(
            name,
            "SHOW_STATUS"
                | "SHOW_USERCOM"
                | "USERCOM"
                | "SOURCE_CHECK"
                | "CALLTRAINEND"
                | "SHOW_JUEL"
                | "SHOW_ABLUP_SELECT"
                | "USERABLUP"
                | "SHOW_SHOP"
                | "SAVEINFO"
                | "USERSHOP"
                | "TITLE_LOADGAME"
                | "SYSTEM_AUTOSAVE"
                | "SYSTEM_TITLE"
                | "SYSTEM_LOADEND"
        )
        || numbered_system_name(name, "COM")
        || numbered_system_name(name, "COM_ABLE")
        || numbered_system_name(name, "ABLUP")
}

fn numbered_system_name(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

struct IndexedSource {
    id: SourceId,
    path: String,
    text: String,
    kind: SourceKind,
    input_order: usize,
    priority: bool,
}

fn index_sources(
    sources: Vec<crate::ProjectSource>,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> Option<Vec<IndexedSource>> {
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    let mut fatal = false;
    for (input_order, source) in sources.into_iter().enumerate() {
        let Some(path) = normalize_path(&source.relative_path) else {
            diagnostics.push(AnalyzerDiagnostic::project_fatal(
                AnalyzerDiagnosticCode::InvalidPath,
                &source.relative_path,
                "source paths must be relative and may not contain '..'",
            ));
            fatal = true;
            continue;
        };
        let path_key = path.to_ascii_uppercase();
        if !seen.insert(path_key) {
            diagnostics.push(AnalyzerDiagnostic::project_fatal(
                AnalyzerDiagnosticCode::DuplicatePath,
                &path,
                "duplicate normalized source path",
            ));
            fatal = true;
            continue;
        }
        let SourcePayload::Utf8(text) = source.payload else {
            if let SourcePayload::IoError(error) = source.payload
                && error.kind != SourceIoErrorKind::NotFound
            {
                diagnostics.push(AnalyzerDiagnostic {
                    code: AnalyzerDiagnosticCode::IoError,
                    parser_code: None,
                    severity: AnalyzerDiagnosticSeverity::Error,
                    reference_level: 2,
                    source: None,
                    message: format!("frontend I/O error for {path}: {}", error.message),
                });
            }
            continue;
        };
        let extension = std::path::Path::new(&path).extension();
        let kind = if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("ERH")) {
            SourceKind::Erh
        } else if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("ERB")) {
            SourceKind::Erb
        } else {
            diagnostics.push(AnalyzerDiagnostic {
                code: AnalyzerDiagnosticCode::UnsupportedSource,
                parser_code: None,
                severity: AnalyzerDiagnosticSeverity::Warning,
                reference_level: 1,
                source: None,
                message: format!("ignored unsupported source {path}"),
            });
            continue;
        };
        let priority = kind == SourceKind::Erb
            && path
                .rsplit_once('/')
                .is_some_and(|(parent, _)| parent.split('/').any(|part| part.contains('#')));
        result.push(IndexedSource {
            id: SourceId::default(),
            path,
            text,
            kind,
            input_order,
            priority,
        });
    }
    if fatal {
        return None;
    }
    result.sort_by(|left, right| {
        let phase = |source: &IndexedSource| match (source.kind, source.priority) {
            (SourceKind::Erh, _) => 0,
            (SourceKind::Erb, true) => 1,
            (SourceKind::Erb, false) => 2,
        };
        phase(left).cmp(&phase(right)).then_with(|| {
            if options.sort_with_filename {
                left.path.cmp(&right.path)
            } else {
                left.input_order.cmp(&right.input_order)
            }
        })
    });
    for (index, source) in result.iter_mut().enumerate() {
        source.id = SourceId(u32::try_from(index).expect("too many source files"));
    }
    Some(result)
}

fn validate_extensions(
    extensions: &ExtensionRegistry,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> bool {
    let builtins = Catalog::build(&ExtensionRegistry::default());
    let mut valid = true;
    for name in extensions.instructions.keys() {
        if builtins
            .instructions
            .contains_key(&name.to_ascii_uppercase())
        {
            diagnostics.push(AnalyzerDiagnostic::project_fatal(
                AnalyzerDiagnosticCode::DuplicateSymbol,
                "",
                format!("extension instruction {name} conflicts with a built-in"),
            ));
            valid = false;
        }
    }
    for name in extensions.functions.keys() {
        if builtins.functions.contains_key(&name.to_ascii_uppercase()) {
            diagnostics.push(AnalyzerDiagnostic::project_fatal(
                AnalyzerDiagnosticCode::DuplicateSymbol,
                "",
                format!("extension function {name} conflicts with a built-in"),
            ));
            valid = false;
        }
    }
    valid
}

fn append_parser_diagnostics<T>(
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
    source: SourceId,
    path: &str,
    text: &str,
    output: &ParseOutput<T>,
) {
    for diagnostic in &output.diagnostics {
        diagnostics.push(map_parser_diagnostic(source, path, text, diagnostic));
    }
}

fn map_parser_diagnostic(
    source: SourceId,
    path: &str,
    text: &str,
    diagnostic: &Diagnostic,
) -> AnalyzerDiagnostic {
    let mut mapped = AnalyzerDiagnostic::at(
        AnalyzerDiagnosticCode::Syntax,
        match diagnostic.severity {
            Severity::Warning => AnalyzerDiagnosticSeverity::Warning,
            Severity::Error => AnalyzerDiagnosticSeverity::Error,
        },
        if diagnostic.severity == Severity::Error {
            2
        } else {
            1
        },
        source,
        path,
        text,
        diagnostic.span,
        diagnostic.message.clone(),
    );
    mapped.parser_code = Some(diagnostic.code.clone());
    mapped
}

fn map_csv_diagnostic(diagnostic: erabasic_csv::CsvDiagnostic) -> AnalyzerDiagnostic {
    AnalyzerDiagnostic {
        code: AnalyzerDiagnosticCode::DeferredIndex,
        parser_code: None,
        severity: match diagnostic.severity {
            CsvDiagnosticSeverity::Notice => AnalyzerDiagnosticSeverity::Notice,
            CsvDiagnosticSeverity::Warning => AnalyzerDiagnosticSeverity::Warning,
            CsvDiagnosticSeverity::Error => AnalyzerDiagnosticSeverity::Error,
            CsvDiagnosticSeverity::Fatal => AnalyzerDiagnosticSeverity::Fatal,
        },
        reference_level: diagnostic.reference_level,
        source: diagnostic
            .source
            .map(|source| crate::AnalyzerSourceLocation {
                source: SourceId::default(),
                relative_path: source.relative_path,
                physical_line: source.physical_line,
                byte_start: source.byte_start,
                byte_end: source.byte_end,
            }),
        message: diagnostic.message,
    }
}

fn at_function(
    source: &ParsedProjectSource,
    function: &AstFunction,
    code: AnalyzerDiagnosticCode,
    message: impl Into<String>,
) -> AnalyzerDiagnostic {
    AnalyzerDiagnostic::at(
        code,
        AnalyzerDiagnosticSeverity::Error,
        2,
        source.source.id,
        &source.source.relative_path,
        &source.text,
        function.span,
        message,
    )
}

fn key(name: &str, ignore_case: bool) -> String {
    if ignore_case {
        name.to_ascii_uppercase()
    } else {
        name.to_owned()
    }
}

fn normalize_path(path: &str) -> Option<String> {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') {
        return None;
    }
    let replaced = path.replace('\\', "/");
    if replaced.len() >= 2 && replaced.as_bytes()[1] == b':' {
        return None;
    }
    let mut parts = Vec::new();
    for part in replaced.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            other => parts.push(other),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}
