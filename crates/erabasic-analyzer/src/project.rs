use std::borrow::Cow;

use erabasic_ast::{Script, SourceKind};
use erabasic_csv::{CsvLoadOptions, resolve_deferred_indices};
use erabasic_data::ProjectData;
use erabasic_hir::{FunctionKind, Program, SourceFile};
use erabasic_parser::{parse_erb, parse_erh};
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    AnalysisInput, AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity,
    AnalyzerOptions, ExtensionRegistry,
    catalog::Catalog,
    context::AnalysisParserContext,
    declarations::{DeclarationInput, analyze_global_declarations},
    expression::IndexResolver,
    identifiers::{identifier_key, is_reserved},
    symbols::Symbols,
};

mod lowering_support;
mod path_order;
mod progress;
mod reachability;
mod source_support;
mod statement_analysis;

use lowering_support::{register_function_declarations, source_file};
pub use path_order::compare_reference_file_paths;
pub(crate) use progress::ProgressCounter;
pub use progress::{AnalysisProgress, AnalysisProgressCallback, AnalysisProgressStage};
use reachability::{function_semantics, reachable_functions, report_uncalled, uncalled_function};
use source_support::{
    append_parser_diagnostics, at_function, index_sources, map_csv_diagnostic, validate_extensions,
};
use statement_analysis::{FunctionDefinition, analyze_function, should_analyze_function};

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
    analyze_project_inner_with_source_lifecycle(
        input,
        options,
        extensions,
        progress,
        cfg!(target_arch = "wasm32"),
    )
}

fn analyze_project_inner_with_source_lifecycle(
    input: AnalysisInput,
    options: &AnalyzerOptions,
    extensions: &ExtensionRegistry,
    progress: Option<&dyn AnalysisProgressCallback>,
    consume_owned_ast: bool,
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
    let parsing_progress = ProgressCounter::new(
        AnalysisProgressStage::Parsing,
        source_count.saturating_mul(2),
        progress,
    );
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
        parsing_progress.advance();
    }
    // ERH parsing above establishes the shared macro and variable environment.
    // ERB parsing never mutates it, so each worker receives a cheap copy-on-write
    // context and indexed collection preserves the source/diagnostic order.
    let parse_erb_source = |source: source_support::IndexedSource| {
        let mut local_context = context.clone();
        let output = parse_erb(&source.text, &mut local_context);
        parsing_progress.advance();
        (source, output)
    };
    #[cfg(not(target_arch = "wasm32"))]
    let erb_outputs = erb_sources
        .into_par_iter()
        .map(parse_erb_source)
        .collect::<Vec<_>>();
    #[cfg(target_arch = "wasm32")]
    let erb_outputs = erb_sources
        .into_iter()
        .map(parse_erb_source)
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
        parsing_progress.advance();
    }
    analyze_with_context(
        input.project_data,
        Cow::Owned(parsed),
        options,
        &catalog,
        &context,
        diagnostics,
        progress,
        consume_owned_ast,
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
        Cow::Borrowed(sources),
        options,
        &catalog,
        &context,
        diagnostics,
        None,
        false,
    )
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn analyze_with_context(
    mut project_data: ProjectData,
    mut sources: Cow<'_, [ParsedProjectSource]>,
    options: &AnalyzerOptions,
    catalog: &Catalog,
    context: &AnalysisParserContext,
    mut diagnostics: Vec<AnalyzerDiagnostic>,
    progress: Option<&dyn AnalysisProgressCallback>,
    consume_owned_ast: bool,
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
        progress,
    );
    if options.use_erd {
        let csv_options = CsvLoadOptions {
            compatibility: options.compatibility.clone(),
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
    drop(declarations);

    let index_resolver = IndexResolver::new(&project_data);
    let mut symbols = Symbols::new(&project_data, &declaration_output.variables, options);
    drop(declaration_output);
    let mut definitions = Vec::new();
    let indexing_progress = ProgressCounter::new(
        AnalysisProgressStage::IndexingFunctions,
        sources
            .iter()
            .filter(|source| source.source.kind == SourceKind::Erb)
            .map(|source| source.script.functions.len())
            .sum(),
        progress,
    );
    for (source_index, source) in sources.iter().enumerate() {
        if source.source.kind != SourceKind::Erb {
            continue;
        }
        for (function_index, function) in source.script.functions.iter().enumerate() {
            let (kind, return_type) = function_semantics(function, &options.compatibility);
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
                .contains_key(&identifier_key(&function.name, options.ignore_case))
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
            match symbols.register_function(
                &function.name,
                kind,
                return_type,
                function.parameters.len(),
            ) {
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
            indexing_progress.advance();
        }
    }

    let reachable = reachable_functions(
        sources.as_ref(),
        &definitions,
        &symbols,
        options,
        context,
        &index_resolver,
    );
    let declaration_progress = ProgressCounter::new(
        AnalysisProgressStage::DeclaringLocals,
        definitions.len(),
        progress,
    );
    for definition in &definitions {
        if !should_analyze_function(definition, &reachable, options) {
            declaration_progress.advance();
            continue;
        }
        let source = &sources[definition.source_index];
        let function = &source.script.functions[definition.function_index];
        symbols.prepare_function_locals(definition.id, &function.name);
        register_function_declarations(
            definition.id,
            definition.kind,
            source,
            function,
            &mut symbols,
            context,
            &index_resolver,
            options,
            &mut diagnostics,
        );
        declaration_progress.advance();
    }
    // Function bodies only read the completed symbol table. Indexed Rayon collection keeps
    // HIR ordering deterministic while large projects analyze independent bodies in parallel.
    let analyzing_progress = ProgressCounter::new(
        AnalysisProgressStage::Analyzing,
        definitions.len().saturating_mul(3),
        progress,
    );
    let analyze_definition = |definition: &FunctionDefinition,
                              source: &ParsedProjectSource,
                              function: &erabasic_ast::Function| {
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

    let mut functions = Vec::with_capacity(definitions.len());
    if consume_owned_ast {
        match &mut sources {
            Cow::Owned(sources) => {
                let mut definitions = definitions.into_iter().peekable();
                for (source_index, source) in sources.iter_mut().enumerate() {
                    if source.source.kind != SourceKind::Erb {
                        source.script.declarations = Vec::new();
                        source.script.top_level = Vec::new();
                        continue;
                    }
                    let ast_functions = std::mem::take(&mut source.script.functions);
                    for (function_index, function) in ast_functions.into_iter().enumerate() {
                        let Some(definition) = definitions.next_if(|definition| {
                            definition.source_index == source_index
                                && definition.function_index == function_index
                        }) else {
                            continue;
                        };
                        let (function, function_diagnostics) =
                            analyze_definition(&definition, source, &function);
                        functions.push(function);
                        diagnostics.extend(function_diagnostics);
                    }
                    // The source text and compact line index remain available for
                    // diagnostics, while the much larger AST allocation is returned
                    // before the next source contributes HIR.
                    source.script.declarations = Vec::new();
                    source.script.top_level = Vec::new();
                }
                debug_assert!(definitions.next().is_none());
            }
            Cow::Borrowed(_) => unreachable!("only owned parser output can be consumed"),
        }
    } else {
        #[cfg(not(target_arch = "wasm32"))]
        let analyzed_functions = definitions
            .par_iter()
            .map(|definition| {
                let source = &sources[definition.source_index];
                let function = &source.script.functions[definition.function_index];
                analyze_definition(definition, source, function)
            })
            .collect::<Vec<_>>();
        #[cfg(target_arch = "wasm32")]
        let analyzed_functions = definitions
            .iter()
            .map(|definition| {
                let source = &sources[definition.source_index];
                let function = &source.script.functions[definition.function_index];
                analyze_definition(definition, source, function)
            })
            .collect::<Vec<_>>();
        for (function, function_diagnostics) in analyzed_functions {
            functions.push(function);
            diagnostics.extend(function_diagnostics);
        }
    }

    let mut program = Program::new(sources.iter().map(|source| source.source.clone()).collect());
    program.compatibility = options.compatibility.clone();
    program.call_compatibility = erabasic_hir::CallCompatibility {
        user_argument_policy: options
            .compatibility
            .user_call_argument_policy(options.strict_user_call_arguments),
        allow_event_as_normal: options.compatible_call_event,
        allow_omitted_arguments: options.compatible_function_argument_optional,
        auto_convert_integer_to_string: options.compatible_function_argument_auto_convert,
        allow_full_width_space: options.allow_full_width_space,
        debug_semicolon: options.debug_semicolon,
        ignore_triple_symbols: options.ignore_triple_symbols,
        compatible_rand: options.compatible_rand,
        system_no_target: options.system_no_target,
        ignore_case: options.ignore_case,
        before_error_throw_hooks: options.compatibility.supports_fault_hooks()
            && !options.disable_before_error_throw,
    };
    program.variables = symbols.variables;
    program.functions = functions;
    let diagnostic_sources = sources
        .iter()
        .map(|source| crate::portability::DiagnosticSource {
            source: &source.source,
            text: &source.text,
        })
        .collect::<Vec<_>>();
    crate::portability::analyze(&program, &diagnostic_sources, &mut diagnostics, || {
        analyzing_progress.advance();
    });
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

#[cfg(test)]
mod source_lifecycle_tests;
