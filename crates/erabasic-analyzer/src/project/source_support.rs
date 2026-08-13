use std::collections::BTreeSet;

use erabasic_ast::{
    Diagnostic, Expr, ExprKind, FormPart, FormattedString, Function as AstFunction, ParseOutput,
    Severity, SourceKind, Span,
};
use erabasic_csv::CsvDiagnosticSeverity;
use erabasic_hir::SourceId;

use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, AnalyzerOptions,
    ExtensionRegistry, SourceIoErrorKind, SourcePayload, catalog::Catalog,
};

use super::{ParsedProjectSource, compare_reference_file_paths};

pub(super) struct IndexedSource {
    pub(super) id: SourceId,
    pub(super) path: String,
    pub(super) text: String,
    pub(super) kind: SourceKind,
    input_order: usize,
    priority: bool,
}

pub(super) fn index_sources(
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
                compare_reference_file_paths(&left.path, &right.path)
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

pub(super) fn validate_extensions(
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

pub(super) fn append_parser_diagnostics<T>(
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

pub(super) fn confine_formatted_spans(formatted: &mut FormattedString, container: Span) {
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

pub(super) fn confine_span(span: &mut Span, container: Span) {
    let start = span.start.clamp(container.start, container.end);
    let end = span.end.clamp(start, container.end);
    *span = Span::new(start, end);
}

pub(super) fn map_parser_diagnostic(
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

pub(super) fn map_csv_diagnostic(diagnostic: erabasic_csv::CsvDiagnostic) -> AnalyzerDiagnostic {
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

pub(super) fn at_function(
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

pub(super) fn key(name: &str, ignore_case: bool) -> String {
    crate::identifiers::identifier_key(name, ignore_case)
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
