use erabasic_ast::{
    Argument, Diagnostic, Directive, Function, ParseOutput, Span, Statement, StatementKind,
    VariableRef,
};

use crate::util::{map_expression_spans, map_formatted_spans};

#[derive(Clone, Copy)]
enum ContinuationSegmentKind {
    Source,
    Replacement,
}

struct ContinuationSegment {
    logical_start: usize,
    logical_end: usize,
    source_start: usize,
    source_end: usize,
    kind: ContinuationSegmentKind,
}

#[derive(Default)]
pub(crate) struct ContinuationSourceMap {
    segments: Vec<ContinuationSegment>,
}

impl ContinuationSourceMap {
    pub(crate) fn push_source(&mut self, logical_start: usize, length: usize, source_start: usize) {
        self.segments.push(ContinuationSegment {
            logical_start,
            logical_end: logical_start + length,
            source_start,
            source_end: source_start + length,
            kind: ContinuationSegmentKind::Source,
        });
    }

    pub(crate) fn push_replacement(
        &mut self,
        logical_start: usize,
        length: usize,
        source_start: usize,
        source_end: usize,
    ) {
        self.segments.push(ContinuationSegment {
            logical_start,
            logical_end: logical_start + length,
            source_start,
            source_end,
            kind: ContinuationSegmentKind::Replacement,
        });
    }

    fn map_span(&self, span: Span, logical_start: usize) -> Span {
        let start = logical_start.saturating_add(span.start);
        let end = logical_start.saturating_add(span.end);
        if start == end {
            let at = self.map_start(start);
            return Span::empty(at);
        }
        Span::new(self.map_start(start), self.map_end(end))
    }

    fn map_start(&self, offset: usize) -> usize {
        self.segments
            .iter()
            .find(|segment| segment.logical_start <= offset && offset < segment.logical_end)
            .map_or_else(
                || {
                    self.segments
                        .last()
                        .map_or(offset, |segment| segment.source_end)
                },
                |segment| segment.map(offset),
            )
    }

    fn map_end(&self, offset: usize) -> usize {
        self.segments
            .iter()
            .find(|segment| segment.logical_start < offset && offset <= segment.logical_end)
            .map_or_else(
                || {
                    self.segments
                        .first()
                        .map_or(offset, |segment| segment.source_start)
                },
                |segment| segment.map(offset),
            )
    }
}

impl ContinuationSegment {
    fn map(&self, offset: usize) -> usize {
        match self.kind {
            ContinuationSegmentKind::Source => self
                .source_start
                .saturating_add(offset.saturating_sub(self.logical_start)),
            ContinuationSegmentKind::Replacement => {
                let logical_length = self.logical_end.saturating_sub(self.logical_start);
                if logical_length == 0 {
                    return self.source_start;
                }
                let source_length = self.source_end.saturating_sub(self.source_start);
                self.source_start.saturating_add(
                    offset
                        .saturating_sub(self.logical_start)
                        .saturating_mul(source_length)
                        / logical_length,
                )
            }
        }
    }
}

pub(crate) fn remap_statement_output(
    output: &mut ParseOutput<Statement>,
    source_map: &ContinuationSourceMap,
    logical_start: usize,
) {
    remap_diagnostics(&mut output.diagnostics, source_map, logical_start);
    if let Some(statement) = &mut output.value {
        remap_statement(statement, source_map, logical_start);
    }
}

pub(crate) fn remap_directive_output(
    output: &mut ParseOutput<Directive>,
    source_map: &ContinuationSourceMap,
    logical_start: usize,
) {
    remap_diagnostics(&mut output.diagnostics, source_map, logical_start);
    if let Some(directive) = &mut output.value {
        remap_directive(directive, source_map, logical_start);
    }
}

pub(crate) fn remap_function_output(
    output: &mut ParseOutput<Function>,
    source_map: &ContinuationSourceMap,
    logical_start: usize,
) {
    remap_diagnostics(&mut output.diagnostics, source_map, logical_start);
    if let Some(function) = &mut output.value {
        function.span = source_map.map_span(function.span, logical_start);
        for parameter in &mut function.parameters {
            parameter.span = source_map.map_span(parameter.span, logical_start);
            if let Some(target) = &mut parameter.target {
                remap_variable(target, source_map, logical_start);
            }
            if let Some(default) = &mut parameter.default {
                map_expression_spans(default, &|span| source_map.map_span(span, logical_start));
            }
        }
    }
}

fn remap_diagnostics(
    diagnostics: &mut [Diagnostic],
    source_map: &ContinuationSourceMap,
    logical_start: usize,
) {
    for diagnostic in diagnostics {
        diagnostic.span = source_map.map_span(diagnostic.span, logical_start);
    }
}

fn remap_statement(
    statement: &mut Statement,
    source_map: &ContinuationSourceMap,
    logical_start: usize,
) {
    statement.span = source_map.map_span(statement.span, logical_start);
    match &mut statement.kind {
        StatementKind::Instruction { arguments, .. } => {
            for argument in arguments {
                remap_argument(argument, source_map, logical_start);
            }
        }
        StatementKind::Assignment {
            target,
            value,
            additional_values,
            ..
        } => {
            remap_variable(target, source_map, logical_start);
            map_expression_spans(value, &|span| source_map.map_span(span, logical_start));
            for value in additional_values {
                map_expression_spans(value, &|span| source_map.map_span(span, logical_start));
            }
        }
        StatementKind::Directive(directive) => {
            remap_directive(directive, source_map, logical_start);
        }
        StatementKind::GotoLabel { .. } | StatementKind::Invalid => {}
    }
}

fn remap_directive(
    directive: &mut Directive,
    source_map: &ContinuationSourceMap,
    logical_start: usize,
) {
    directive.span = source_map.map_span(directive.span, logical_start);
    for argument in &mut directive.arguments {
        remap_argument(argument, source_map, logical_start);
    }
}

fn remap_argument(
    argument: &mut Argument,
    source_map: &ContinuationSourceMap,
    logical_start: usize,
) {
    match argument {
        Argument::Expression(expression) | Argument::MixedExpression { expression, .. } => {
            map_expression_spans(expression, &|span| source_map.map_span(span, logical_start));
        }
        Argument::Formatted(formatted) => {
            map_formatted_spans(formatted, &|span| source_map.map_span(span, logical_start));
        }
        Argument::Omitted(span) => {
            *span = source_map.map_span(*span, logical_start);
        }
        Argument::Raw(_) => {}
    }
}

fn remap_variable(
    variable: &mut VariableRef,
    source_map: &ContinuationSourceMap,
    logical_start: usize,
) {
    variable.span = source_map.map_span(variable.span, logical_start);
    for index in &mut variable.indices {
        map_expression_spans(index, &|span| source_map.map_span(span, logical_start));
    }
}
