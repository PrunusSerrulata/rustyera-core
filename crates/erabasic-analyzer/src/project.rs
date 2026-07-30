use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use erabasic_ast::{
    Argument, Diagnostic, Expr, ExprKind, FormPart, FormattedString, Function as AstFunction,
    ParseOutput, Script, Severity, SourceKind, Span, Statement, StatementKind, VariableRef,
};
use erabasic_csv::{CsvDiagnosticSeverity, CsvLoadOptions, resolve_deferred_indices};
use erabasic_data::ProjectData;
use erabasic_hir::{
    ConstantValue, EventAttributes, Function, FunctionId, FunctionKind, HirArgument, HirExpr,
    HirExprKind, HirStatement, HirStatementKind, InstructionTarget, LabelId, LineId, Parameter,
    Program, SemanticType, SourceFile, SourceId, SourceLocation,
};
use erabasic_parser::{parse_erb, parse_erh};
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    AnalysisInput, AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity,
    AnalyzerOptions, ExtensionRegistry, SourceIoErrorKind, SourcePayload, WarningPolicy,
    catalog::Catalog,
    context::AnalysisParserContext,
    control_flow::build_control_flow,
    declarations::{
        DeclarationInput, analyze_global_declarations, parse_private_declaration,
        parse_scoped_declaration,
    },
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnalysisProgressStage {
    Parsing,
    Analyzing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnalysisProgress {
    pub stage: AnalysisProgressStage,
    pub completed: usize,
    pub total: usize,
}

struct ProgressCounter<'a> {
    stage: AnalysisProgressStage,
    total: usize,
    completed: AtomicUsize,
    reported_percent: AtomicUsize,
    callback_lock: Mutex<()>,
    callback: Option<&'a dyn AnalysisProgressCallback>,
}

impl<'a> ProgressCounter<'a> {
    fn new(
        stage: AnalysisProgressStage,
        total: usize,
        callback: Option<&'a dyn AnalysisProgressCallback>,
    ) -> Self {
        if let Some(callback) = callback {
            callback(AnalysisProgress {
                stage,
                completed: 0,
                total,
            });
        }
        Self {
            stage,
            total,
            completed: AtomicUsize::new(0),
            reported_percent: AtomicUsize::new(0),
            callback_lock: Mutex::new(()),
            callback,
        }
    }

    fn advance(&self) {
        let completed = self.completed.fetch_add(1, Ordering::Relaxed) + 1;
        let Some(callback) = self.callback else {
            return;
        };
        let percent = completed
            .saturating_mul(100)
            .checked_div(self.total)
            .unwrap_or(100);
        let _guard = self
            .callback_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = self.reported_percent.load(Ordering::Relaxed);
        if percent > previous || completed == self.total {
            self.reported_percent.store(percent, Ordering::Relaxed);
            callback(AnalysisProgress {
                stage: self.stage,
                completed,
                total: self.total,
            });
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub trait AnalysisProgressCallback: Fn(AnalysisProgress) + Sync {}

#[cfg(not(target_arch = "wasm32"))]
impl<T> AnalysisProgressCallback for T where T: Fn(AnalysisProgress) + Sync {}

#[cfg(target_arch = "wasm32")]
pub trait AnalysisProgressCallback: Fn(AnalysisProgress) {}

#[cfg(target_arch = "wasm32")]
impl<T> AnalysisProgressCallback for T where T: Fn(AnalysisProgress) {}

#[must_use]
pub fn analyze_project(
    input: AnalysisInput,
    options: &AnalyzerOptions,
    extensions: &ExtensionRegistry,
) -> AnalysisReport {
    analyze_project_inner(input, options, extensions, None)
}

#[must_use]
pub fn analyze_project_with_progress(
    input: AnalysisInput,
    options: &AnalyzerOptions,
    extensions: &ExtensionRegistry,
    progress: &dyn AnalysisProgressCallback,
) -> AnalysisReport {
    analyze_project_inner(input, options, extensions, Some(progress))
}

fn analyze_project_inner(
    input: AnalysisInput,
    options: &AnalyzerOptions,
    extensions: &ExtensionRegistry,
    progress: Option<&dyn AnalysisProgressCallback>,
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
    let mut indexed = indexed;
    let first_erb = indexed.partition_point(|source| source.kind == SourceKind::Erh);
    let erb_sources = indexed.split_off(first_erb);
    let source_count = indexed.len() + erb_sources.len();
    let parsing_progress =
        ProgressCounter::new(AnalysisProgressStage::Parsing, source_count, progress);
    let mut parsed = Vec::with_capacity(source_count);
    for source in indexed {
        let output = parse_erh(&source.text, &mut context);
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
        parsing_progress.advance();
    }
    // ERH parsing above establishes the shared macro and variable environment.
    // ERB parsing never mutates it, so each worker receives a cheap copy-on-write
    // context and indexed collection preserves the source/diagnostic order.
    #[cfg(not(target_arch = "wasm32"))]
    let erb_outputs = erb_sources
        .into_par_iter()
        .map(|source| {
            let mut local_context = context.clone();
            let output = parse_erb(&source.text, &mut local_context);
            parsing_progress.advance();
            (source, output)
        })
        .collect::<Vec<_>>();
    #[cfg(target_arch = "wasm32")]
    let erb_outputs = erb_sources
        .into_iter()
        .map(|source| {
            let mut local_context = context.clone();
            let output = parse_erb(&source.text, &mut local_context);
            parsing_progress.advance();
            (source, output)
        })
        .collect::<Vec<_>>();
    for (source, output) in erb_outputs {
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
        progress,
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
        None,
    )
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn analyze_with_context(
    mut project_data: ProjectData,
    sources: &[ParsedProjectSource],
    options: &AnalyzerOptions,
    _extensions: &ExtensionRegistry,
    catalog: &Catalog,
    context: &AnalysisParserContext,
    mut diagnostics: Vec<AnalyzerDiagnostic>,
    progress: Option<&dyn AnalysisProgressCallback>,
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
    let mut symbols = Symbols::new(&project_data, &declaration_output.variables, options);
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
            let previous = symbols.function(&function.name).cloned();
            match symbols.register_function(&function.name, kind, return_type) {
                Ok(id) => {
                    if previous.is_some()
                        && kind != FunctionKind::Event
                        && (options.warn_function_overloading || options.analysis_mode)
                    {
                        diagnostics.push(AnalyzerDiagnostic::at(
                            AnalyzerDiagnosticCode::DuplicateSymbol,
                            AnalyzerDiagnosticSeverity::Warning,
                            1,
                            source.source.id,
                            &source.source.relative_path,
                            &source.text,
                            function.span,
                            format!("function {} is already declared", function.name),
                        ));
                    }
                    definitions.push(FunctionDefinition {
                        source_index,
                        function_index,
                        id,
                        kind,
                        return_type,
                        shadowed: previous.is_some() && kind != FunctionKind::Event,
                        definition_order: u32::try_from(definitions.len()).unwrap_or(u32::MAX),
                    });
                }
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
    for definition in &definitions {
        if !should_analyze_function(definition, &reachable, options) {
            continue;
        }
        let source = &sources[definition.source_index];
        let function = &source.script.functions[definition.function_index];
        symbols.prepare_function_locals(definition.id, &function.name);
        register_private_variables(
            definition.id,
            source,
            function,
            &mut symbols,
            context,
            &index_resolver,
            options,
            &mut diagnostics,
        );
    }
    // Function bodies only read the completed symbol table. Indexed Rayon collection keeps
    // HIR ordering deterministic while large projects analyze independent bodies in parallel.
    let analyzing_progress = ProgressCounter::new(
        AnalysisProgressStage::Analyzing,
        definitions.len(),
        progress,
    );
    let analyze_definition = |definition: &FunctionDefinition| {
        let source = &sources[definition.source_index];
        let function = &source.script.functions[definition.function_index];
        let mut function_diagnostics = Vec::new();
        // A same-name normal function after the first one can never be selected
        // by Emuera's non-event dictionary. Keep its identity for deterministic
        // source ordering, but do not lower an unreachable replacement body.
        let should_analyze = should_analyze_function(definition, &reachable, options);
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
                &mut function_diagnostics,
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
            report_uncalled(source, function, options, &mut function_diagnostics);
        }
        analyzing_progress.advance();
        (hir, function_diagnostics)
    };
    #[cfg(not(target_arch = "wasm32"))]
    let analyzed_functions = definitions
        .par_iter()
        .map(analyze_definition)
        .collect::<Vec<_>>();
    #[cfg(target_arch = "wasm32")]
    let analyzed_functions = definitions
        .iter()
        .map(analyze_definition)
        .collect::<Vec<_>>();
    let mut functions = Vec::with_capacity(analyzed_functions.len());
    for (function, function_diagnostics) in analyzed_functions {
        functions.push(function);
        diagnostics.extend(function_diagnostics);
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
    program.call_compatibility = erabasic_hir::CallCompatibility {
        allow_event_as_normal: options.compatible_call_event,
        allow_omitted_arguments: options.compatible_function_argument_optional,
        auto_convert_integer_to_string: options.compatible_function_argument_auto_convert,
    };
    program.variables = symbols.variables;
    program.functions = functions;
    crate::portability::analyze(&program, sources, &mut diagnostics);
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
    shadowed: bool,
    definition_order: u32,
}

fn should_analyze_function(
    definition: &FunctionDefinition,
    reachable: &BTreeSet<FunctionId>,
    options: &AnalyzerOptions,
) -> bool {
    options.analysis_mode
        || (!definition.shadowed
            && (!options.ignore_uncalled_functions || reachable.contains(&definition.id)))
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
    context: &AnalysisParserContext,
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
        let variable = symbols.variables.get(place.variable.0 as usize);
        let reference = variable.is_some_and(|variable| variable.reference);
        let can_default = matches!(target.name.to_ascii_uppercase().as_str(), "ARG" | "ARGS")
            || variable
                .is_some_and(|variable| variable.scope == erabasic_hir::VariableScope::Function);
        let default = parameter
            .default
            .as_ref()
            .map(|expression| {
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
            })
            .or_else(|| {
                (!reference && can_default).then(|| {
                    let constant = match place.value_type {
                        SemanticType::String => ConstantValue::String(String::new()),
                        _ => ConstantValue::Integer(0),
                    };
                    HirExpr {
                        kind: match &constant {
                            ConstantValue::Integer(value) => HirExprKind::Integer { value: *value },
                            ConstantValue::String(value) => HirExprKind::String {
                                value: value.clone(),
                            },
                        },
                        value_type: place.value_type,
                        constant: Some(constant),
                        location: SourceLocation::new(source.source.id, parameter.span),
                    }
                })
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
        if let Some(default) = &default
            && default.constant.is_none()
            && default.value_type != SemanticType::Error
        {
            diagnostics.push(AnalyzerDiagnostic::at(
                AnalyzerDiagnosticCode::InvalidArgument,
                AnalyzerDiagnosticSeverity::Error,
                2,
                source.source.id,
                &source.source.relative_path,
                &source.text,
                parameter.span,
                "parameter default must be a compile-time constant",
            ));
        }
        if parameter.default.is_some() && reference {
            diagnostics.push(AnalyzerDiagnostic::at(
                AnalyzerDiagnosticCode::InvalidArgument,
                AnalyzerDiagnosticSeverity::Error,
                2,
                source.source.id,
                &source.source.relative_path,
                &source.text,
                parameter.span,
                "reference parameters cannot have defaults",
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
            context,
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
    context: &AnalysisParserContext,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> HirStatement {
    let location_span = match &statement.kind {
        StatementKind::Instruction { arguments, .. } if matches!(arguments.first(), Some(Argument::Raw(value)) if !value.is_empty()) =>
        {
            let Argument::Raw(value) = &arguments[0] else {
                unreachable!("guard requires a raw argument")
            };
            source.text[statement.span.start..statement.span.end]
                .find(value)
                .map_or(statement.span, |offset| {
                    Span::new(statement.span.start + offset, statement.span.end)
                })
        }
        _ => statement.span,
    };
    let location = SourceLocation::new(source.source.id, location_span);
    let kind = match &statement.kind {
        StatementKind::Assignment {
            target,
            op,
            value,
            additional_values,
            raw_value,
        } => {
            let target_expression = Expr {
                kind: ExprKind::Variable {
                    name: target.name.clone(),
                    indices: target.indices.clone(),
                },
                span: target.span,
            };
            let analyzed_target = ExpressionAnalyzer {
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
            let HirExprKind::Variable { place } = analyzed_target.kind else {
                return HirStatement {
                    id: line_id,
                    kind: HirStatementKind::Error,
                    location,
                };
            };
            let form_assignment =
                place.value_type == SemanticType::String && *op == erabasic_ast::AssignOp::Assign;
            let mut reparsed_values = Vec::new();
            let mut reparse_had_errors = false;
            let value = if form_assignment {
                let mut parsed =
                    erabasic_parser::parse_formatted_at(raw_value, value.span.start, context);
                for diagnostic in parsed.diagnostics.drain(..) {
                    diagnostics.push(map_parser_diagnostic(
                        source.source.id,
                        &source.source.relative_path,
                        &source.text,
                        &diagnostic,
                    ));
                }
                parsed.value.map_or_else(
                    || Expr {
                        kind: ExprKind::Error,
                        span: value.span,
                    },
                    |formatted| Expr {
                        kind: ExprKind::Formatted(formatted),
                        span: value.span,
                    },
                )
            } else if *op == erabasic_ast::AssignOp::Assign {
                let mut parsed =
                    erabasic_parser::parse_expression_list_at(raw_value, value.span.start, context);
                reparse_had_errors = parsed.has_errors();
                for diagnostic in parsed.diagnostics.drain(..) {
                    diagnostics.push(map_parser_diagnostic(
                        source.source.id,
                        &source.source.relative_path,
                        &source.text,
                        &diagnostic,
                    ));
                }
                reparsed_values = parsed.value.unwrap_or_default();
                reparsed_values.first().cloned().unwrap_or(Expr {
                    kind: ExprKind::Error,
                    span: value.span,
                })
            } else {
                value.clone()
            };
            let default_omitted = |expression: &Expr| {
                if !reparse_had_errors && matches!(expression.kind, ExprKind::Error) {
                    Expr {
                        kind: match place.value_type {
                            SemanticType::String => ExprKind::String(String::new()),
                            SemanticType::Integer | SemanticType::Void | SemanticType::Error => {
                                ExprKind::Integer(0)
                            }
                        },
                        span: expression.span,
                    }
                } else {
                    expression.clone()
                }
            };
            let value = default_omitted(&value);
            let mut values = vec![
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
                .analyze(&value),
            ];
            if !form_assignment {
                let additional = if *op == erabasic_ast::AssignOp::Assign {
                    reparsed_values.iter().skip(1).collect::<Vec<_>>()
                } else {
                    additional_values.iter().collect::<Vec<_>>()
                };
                for additional in additional {
                    values.push(
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
                        .analyze(&default_omitted(additional)),
                    );
                }
            }
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
            for analyzed_value in &values {
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
                        analyzed_value.location.span,
                        "assignment value type does not match its target",
                    ));
                }
            }
            if values.len() > 1 {
                let mut arguments = vec![HirArgument::Place(place)];
                arguments.extend(values.into_iter().map(HirArgument::Expression));
                HirStatementKind::Instruction {
                    target: InstructionTarget::Builtin("SET".into()),
                    arguments,
                }
            } else {
                HirStatementKind::Assignment {
                    target: place,
                    op: *op,
                    value: values.pop().expect("an assignment always has one value"),
                }
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
            context,
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
    context: &AnalysisParserContext,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> HirStatementKind {
    let key = key(name, options.ignore_case);
    let signature = catalog.instructions.get(&key);
    let method_signature = signature
        .is_none()
        .then(|| catalog.functions.get(&key))
        .flatten();
    if matches!(key.as_str(), "VARI" | "VARS") {
        return analyze_scoped_declaration_statement(
            function,
            source,
            statement,
            &key,
            raw_arguments,
            symbols,
            catalog,
            context,
            index_resolver,
            options,
            diagnostics,
        );
    }
    if signature.is_none() && method_signature.is_none() {
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
    if key == "CASE" {
        return HirStatementKind::Instruction {
            target: InstructionTarget::Builtin(key),
            arguments: analyze_case_arguments(
                function,
                source,
                statement,
                raw_arguments,
                symbols,
                catalog,
                context,
                index_resolver,
                options,
                diagnostics,
            ),
        };
    }
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
            lowered.push(HirArgument::Raw(resolve_static_target(
                raw_arguments,
                index_resolver,
            )));
            continue;
        }
        lowered.push(match argument {
            Argument::Expression(expression) => {
                if ((key == "ARRAYSORT" && index == 1) || (key == "SORTCHARA" && index <= 1))
                    && let erabasic_ast::ExprKind::Identifier(order) = &expression.kind
                    && matches!(order.to_ascii_uppercase().as_str(), "FORWARD" | "BACK")
                {
                    lowered.push(HirArgument::Raw(order.to_ascii_uppercase()));
                    continue;
                }
                let expression = analyzer.analyze(expression);
                let constant_form = if index == 0 && key.contains("FORMS") {
                    match &expression.constant {
                        Some(ConstantValue::String(value)) => Some(value.clone()),
                        Some(ConstantValue::Integer(_)) | None => None,
                    }
                } else {
                    None
                };
                if let Some(template) = constant_form {
                    // FORMS instructions evaluate their string expression first and then parse
                    // the result as FORM text in the current function scope. Constant templates
                    // can be lowered into ordinary formatted HIR without adding a runtime parser.
                    let expression_span = expression.location.span;
                    let mut parsed = erabasic_parser::parse_formatted_at(
                        &template,
                        expression_span.start,
                        context,
                    );
                    for diagnostic in &mut parsed.diagnostics {
                        confine_span(&mut diagnostic.span, expression_span);
                    }
                    for diagnostic in parsed.diagnostics.drain(..) {
                        analyzer.diagnostics.push(map_parser_diagnostic(
                            source.source.id,
                            &source.source.relative_path,
                            &source.text,
                            &diagnostic,
                        ));
                    }
                    if let Some(mut formatted) = parsed.value {
                        confine_formatted_spans(&mut formatted, expression_span);
                        HirArgument::Formatted(analyzer.analyze_formatted(&formatted))
                    } else {
                        HirArgument::Expression(expression)
                    }
                } else {
                    let constraint = signature
                        .and_then(|signature| {
                            signature.arguments.get(index).or_else(|| {
                                signature
                                    .variadic
                                    .then(|| signature.arguments.last())
                                    .flatten()
                            })
                        })
                        .or_else(|| {
                            method_signature.and_then(|signature| {
                                signature.arguments.get(index).or_else(|| {
                                    signature
                                        .variadic
                                        .then(|| signature.arguments.last())
                                        .flatten()
                                })
                            })
                        });
                    let mutable = constraint.is_some_and(|constraint| {
                        matches!(
                            constraint,
                            crate::ArgumentConstraint::MutableInteger
                                | crate::ArgumentConstraint::MutableString
                                | crate::ArgumentConstraint::MutableAny
                                | crate::ArgumentConstraint::ReferenceAny
                                | crate::ArgumentConstraint::ReferenceOrString
                                | crate::ArgumentConstraint::MutableReferenceOrString
                        ) || *constraint == crate::ArgumentConstraint::IntegerOrMutableString
                            && expression.value_type == SemanticType::String
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
            }
            Argument::MixedExpression { expression, is_px } => {
                let expression = analyzer.analyze(expression);
                if key == "PRINT_IMG" && expression.value_type == SemanticType::String {
                    HirArgument::Expression(expression)
                } else {
                    HirArgument::MixedExpression {
                        expression,
                        is_px: *is_px,
                    }
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
        lowered.push(HirArgument::Raw(resolve_static_target(
            raw_arguments,
            index_resolver,
        )));
    }
    if static_target && lowered.len() > 1 && matches!(lowered.last(), Some(HirArgument::Omitted)) {
        // Emuera treats a final comma after a static CALL/JUMP target as the end
        // of its argument list, not as an extra omitted user-function argument.
        lowered.pop();
    }
    if matches!(key.as_str(), "IF" | "ELSEIF" | "SIF" | "WHILE" | "REPEAT")
        && matches!(lowered.last(), Some(HirArgument::Omitted))
    {
        // The reference condition builders consume their first term and tolerate
        // a dangling comma left by translated scripts.
        lowered.pop();
    }
    if let Some(signature) = signature {
        let expression_arguments: Vec<_> = lowered
            .iter()
            .map(|argument| match argument {
                HirArgument::Expression(expression)
                | HirArgument::MixedExpression { expression, .. } => Some(expression.clone()),
                HirArgument::Place(place) => Some(erabasic_hir::HirExpr {
                    kind: HirExprKind::Variable {
                        place: place.clone(),
                    },
                    value_type: place.value_type,
                    constant: None,
                    location: place.location,
                }),
                HirArgument::Formatted(value) => Some(HirExpr {
                    kind: HirExprKind::Formatted {
                        value: value.clone(),
                    },
                    value_type: SemanticType::String,
                    constant: None,
                    location: value.location,
                }),
                HirArgument::Omitted | HirArgument::Raw(_) => None,
            })
            .collect();
        if !matches!(
            signature.argument_style,
            erabasic_parser::ArgumentStyle::Formatted
                | erabasic_parser::ArgumentStyle::Raw
                | erabasic_parser::ArgumentStyle::Times
                | erabasic_parser::ArgumentStyle::DynamicCall
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
    } else if let Some(signature) = method_signature {
        let expression_arguments: Vec<_> = lowered
            .iter()
            .map(|argument| match argument {
                HirArgument::Expression(expression)
                | HirArgument::MixedExpression { expression, .. } => Some(expression.clone()),
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
        if key.contains("FORM") && !key.contains("FORMS") {
            if lowered.len() < signature.minimum_arguments {
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
        } else {
            analyzer.check_arguments(
                &expression_arguments,
                &signature.arguments,
                signature.minimum_arguments,
                signature.variadic,
                signature.allow_omitted,
                SourceLocation::new(source.source.id, statement.span),
            );
        }
    }
    let target = if let Some(method_signature) = method_signature {
        InstructionTarget::BuiltinMethod {
            name: key,
            return_type: method_signature.return_type,
        }
    } else if signature.is_none() {
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

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn analyze_scoped_declaration_statement(
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
fn analyze_case_arguments(
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

fn static_target_source(raw: &str) -> &str {
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

fn resolve_static_target(raw: &str, index_resolver: &IndexResolver) -> String {
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

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn register_private_variables(
    function_id: FunctionId,
    source: &ParsedProjectSource,
    function: &AstFunction,
    symbols: &mut Symbols,
    context: &AnalysisParserContext,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) {
    let private_directives: Vec<_> = function
        .attributes
        .iter()
        .filter(|directive| matches!(directive.name.as_str(), "DIM" | "DIMS"))
        .collect();
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
    if private_directives.is_empty() && scoped_statements.clone().next().is_none() {
        return;
    }

    // Building the constant lookup clones every constant name and value. Most
    // functions have no private declarations, so defer that work until the
    // declaration parser can actually consume it.
    let mut constants = symbols.constant_values();
    let mut variable_dimensions = symbols.variable_dimensions(function_id);
    for directive in private_directives {
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
                let target = static_target_source(raw_arguments).trim().trim_matches('"');
                if !target.is_empty() {
                    calls.push(target.to_owned());
                }
            }
            for argument in arguments {
                if let Argument::Expression(expression)
                | Argument::MixedExpression { expression, .. } = argument
                {
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
    // Emuera parses every function once a runtime-resolved call target is reachable.
    // Keep this list aligned with the cross-function dynamic lowering paths so the
    // IgnoreUncalledFunction optimization cannot discard a possible target body.
    matches!(
        &statement.kind,
        StatementKind::Instruction { name, .. }
            if matches!(
                name.as_str(),
                "CALLFORM"
                    | "CALLFORMF"
                    | "JUMPFORM"
                    | "TRYCALLFORM"
                    | "TRYCALLFORMF"
                    | "TRYJUMPFORM"
                    | "TRYCCALL"
                    | "TRYCCALLFORM"
                    | "TRYCJUMP"
                    | "TRYCJUMPFORM"
            )
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

fn confine_formatted_spans(formatted: &mut FormattedString, container: Span) {
    formatted.span = container;
    for part in &mut formatted.parts {
        match part {
            FormPart::Text(_) => {}
            FormPart::StringInterpolation {
                expression,
                width,
                span,
                ..
            }
            | FormPart::IntegerInterpolation {
                expression,
                width,
                span,
                ..
            } => {
                confine_expr_spans(expression, container);
                if let Some(width) = width {
                    confine_expr_spans(width, container);
                }
                confine_span(span, container);
            }
            FormPart::Conditional {
                condition,
                then_value,
                else_value,
                span,
            } => {
                confine_expr_spans(condition, container);
                confine_formatted_spans(then_value, container);
                if let Some(else_value) = else_value {
                    confine_formatted_spans(else_value, container);
                }
                confine_span(span, container);
            }
            FormPart::Triple { span, .. } => confine_span(span, container),
        }
    }
}

fn confine_expr_spans(expression: &mut Expr, container: Span) {
    confine_span(&mut expression.span, container);
    match &mut expression.kind {
        ExprKind::Variable { indices, .. } => {
            for index in indices {
                confine_expr_spans(index, container);
            }
        }
        ExprKind::Call { args, .. } => {
            for argument in args.iter_mut().flatten() {
                confine_expr_spans(argument, container);
            }
        }
        ExprKind::Unary { operand, .. }
        | ExprKind::Postfix { operand, .. }
        | ExprKind::Group(operand) => confine_expr_spans(operand, container),
        ExprKind::Binary { left, right, .. } => {
            confine_expr_spans(left, container);
            confine_expr_spans(right, container);
        }
        ExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            confine_expr_spans(condition, container);
            confine_expr_spans(then_expr, container);
            confine_expr_spans(else_expr, container);
        }
        ExprKind::Formatted(formatted) => confine_formatted_spans(formatted, container),
        ExprKind::Integer(_) | ExprKind::String(_) | ExprKind::Identifier(_) | ExprKind::Error => {}
    }
}

fn confine_span(span: &mut Span, container: Span) {
    let start = span.start.clamp(container.start, container.end);
    let end = span.end.clamp(start, container.end);
    *span = Span::new(start, end);
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
