//! Parser-derived appearances; unparsed/preprocessed candidates remain explicitly uncertain.

use std::collections::{BTreeMap, BTreeSet};

use erabasic_analyzer::AnalyzerOptions;
use erabasic_ast::{
    Argument, Directive, Expr, ExprKind, FormPart, FormattedString, Script, Span, Statement,
    StatementKind,
};
use erabasic_parser::{DefaultParserContext, parse_erb, parse_erh, parse_expression, parse_line};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize)]
pub(super) struct Appearance {
    pub path: String,
    pub api: String,
    pub form: String,
    pub arity: Option<usize>,
    pub omitted_arguments: usize,
    pub span: Span,
    pub span_status: String,
    pub activity: String,
    pub raw: String,
    pub dynamic_target: Option<String>,
    pub owning_function: Option<usize>,
    pub ownership_status: String,
    pub target: Option<super::targets::Target>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ParsedFunction {
    pub id: usize,
    pub name: String,
    pub path: String,
    pub decoded_utf8_blake3: String,
    pub span: Span,
    pub span_status: String,
    pub attributes: Vec<String>,
    pub parameters: Vec<Value>,
}

struct Walker<'a> {
    path: &'a str,
    source: &'a str,
    activity: &'a str,
    offset: usize,
    context: &'a DefaultParserContext,
    rows: &'a mut Vec<Appearance>,
    owner: Option<usize>,
    ownership_status: &'a str,
}

impl Walker<'_> {
    fn add(
        &mut self,
        api: &str,
        form: &str,
        arity: Option<usize>,
        omitted: usize,
        span: Span,
        dynamic: Option<String>,
    ) {
        let shifted = span
            .start
            .checked_add(self.offset)
            .zip(span.end.checked_add(self.offset))
            .map(|(start, end)| Span::new(start, end));
        let raw = shifted.and_then(|span| self.source.get(span.start..span.end));
        let span = shifted.unwrap_or(span);
        self.rows.push(Appearance {
            path: self.path.into(),
            api: api.to_ascii_uppercase(),
            form: form.into(),
            arity,
            omitted_arguments: omitted,
            span,
            span_status: if raw.is_some() {
                "valid_decoded_utf8"
            } else {
                "invalid_parser_span"
            }
            .into(),
            activity: if raw.is_some() {
                self.activity
            } else {
                "unverified_invalid_parser_span"
            }
            .into(),
            raw: raw.unwrap_or_default().into(),
            dynamic_target: dynamic,
            owning_function: self.owner,
            ownership_status: self.ownership_status.into(),
            target: None,
        });
    }

    fn arguments(&mut self, arguments: &[Argument]) {
        for argument in arguments {
            match argument {
                Argument::Expression(expression) | Argument::MixedExpression { expression, .. } => {
                    self.expression(expression)
                }
                Argument::Formatted(form) => self.formatted(form),
                Argument::Raw(_) | Argument::Omitted(_) => {}
            }
        }
    }

    fn directive(&mut self, directive: &Directive) {
        self.add(
            &directive.name,
            "declaration",
            Some(directive.arguments.len()),
            0,
            directive.span,
            None,
        );
        self.arguments(&directive.arguments);
    }

    fn statement(&mut self, statement: &Statement) {
        match &statement.kind {
            StatementKind::Instruction {
                name,
                arguments,
                raw_arguments,
            } => {
                let upper = name.to_ascii_uppercase();
                let dynamic = (upper.contains("FORM")
                    && ["CALL", "JUMP", "GOTO"]
                        .iter()
                        .any(|prefix| upper.contains(prefix)))
                .then(|| raw_arguments.clone());
                self.add(
                    name,
                    "instruction",
                    Some(arguments.len()),
                    arguments
                        .iter()
                        .filter(|argument| matches!(argument, Argument::Omitted(_)))
                        .count(),
                    statement.span,
                    dynamic,
                );
                if let Some(row) = self.rows.last_mut() {
                    row.target = super::targets::instruction(&upper, arguments);
                }
                self.arguments(arguments);
            }
            StatementKind::Assignment {
                target,
                value,
                additional_values,
                op,
                raw_value,
                ..
            } => {
                self.add(
                    &target.name,
                    "variable_write",
                    Some(target.indices.len()),
                    0,
                    target.span,
                    None,
                );
                if !matches!(
                    op,
                    erabasic_ast::AssignOp::Assign | erabasic_ast::AssignOp::StringAssign
                ) {
                    self.add(
                        &format!("OPERATOR_{op:?}"),
                        "compound_assignment",
                        Some(2),
                        0,
                        statement.span,
                        None,
                    );
                }
                for index in &target.indices {
                    self.expression(index);
                }
                self.expression(value);
                if matches!(op, erabasic_ast::AssignOp::Assign) {
                    self.assignment_candidate(raw_value, value.span.start);
                }
                for value in additional_values {
                    self.expression(value);
                }
            }
            StatementKind::Directive(directive) => self.directive(directive),
            StatementKind::Invalid => {
                self.add("<invalid>", "syntax", None, 0, statement.span, None)
            }
            StatementKind::GotoLabel { .. } => {}
        }
    }

    fn assignment_candidate(&mut self, raw: &str, start: usize) {
        // Plain `=` retains FORM syntax until the target's semantic type is known.
        // Preserve the numeric interpretation as a candidate, not an executable API.
        let parsed = parse_expression(raw, self.context);
        if parsed.diagnostics.is_empty()
            && let Some(expression) = parsed.value
        {
            Walker {
                path: self.path,
                source: self.source,
                activity: "unverified_type_directed_rhs",
                offset: self.offset + start,
                context: self.context,
                rows: self.rows,
                owner: self.owner,
                ownership_status: self.ownership_status,
            }
            .expression(&expression);
        }
    }

    fn expression(&mut self, expression: &Expr) {
        match &expression.kind {
            ExprKind::Call { name, args } => {
                self.add(
                    name,
                    "expression",
                    Some(args.len()),
                    args.iter().filter(|argument| argument.is_none()).count(),
                    expression.span,
                    None,
                );
                if let Some(row) = self.rows.last_mut() {
                    row.target = Some(super::targets::expression(name, args));
                }
                for argument in args.iter().flatten() {
                    self.expression(argument);
                }
            }
            ExprKind::Variable { name, indices } => {
                self.add(
                    name,
                    "variable_read",
                    Some(indices.len()),
                    0,
                    expression.span,
                    None,
                );
                for index in indices {
                    self.expression(index);
                }
            }
            ExprKind::Unary { operand, .. }
            | ExprKind::Postfix { operand, .. }
            | ExprKind::Group(operand) => self.expression(operand),
            ExprKind::Binary { op, left, right } => {
                self.add(
                    &format!("OPERATOR_{op:?}"),
                    "operator",
                    Some(2),
                    0,
                    expression.span,
                    None,
                );
                self.expression(left);
                self.expression(right);
            }
            ExprKind::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expression(condition);
                self.expression(then_expr);
                self.expression(else_expr);
            }
            ExprKind::Formatted(form) => self.formatted(form),
            ExprKind::Error => self.add(
                "<expression_error>",
                "syntax",
                None,
                0,
                expression.span,
                None,
            ),
            ExprKind::Identifier(name) => self.add(
                name,
                "variable_or_identifier",
                None,
                0,
                expression.span,
                None,
            ),
            ExprKind::Integer(_) | ExprKind::String(_) => {}
        }
    }

    fn formatted(&mut self, formatted: &FormattedString) {
        for part in &formatted.parts {
            match part {
                FormPart::StringInterpolation {
                    expression, width, ..
                }
                | FormPart::IntegerInterpolation {
                    expression, width, ..
                } => {
                    self.expression(expression);
                    if let Some(width) = width {
                        self.expression(width);
                    }
                }
                FormPart::Conditional {
                    condition,
                    then_value,
                    else_value,
                    ..
                } => {
                    self.expression(condition);
                    self.formatted(then_value);
                    if let Some(value) = else_value {
                        self.formatted(value);
                    }
                }
                FormPart::Text(_) | FormPart::Triple { .. } => {}
            }
        }
    }

    fn script(
        &mut self,
        script: &Script,
        covered: &mut Vec<Span>,
        functions: &mut Vec<ParsedFunction>,
    ) {
        let source_hash = blake3::hash(self.source.as_bytes()).to_hex().to_string();
        for directive in &script.declarations {
            covered.push(directive.span);
            self.directive(directive);
        }
        for statement in &script.top_level {
            covered.push(statement.span);
            self.statement(statement);
        }
        for function in &script.functions {
            let id = functions.len();
            self.owner = Some(id);
            self.ownership_status = "parser_function_membership_not_execution";
            functions.push(ParsedFunction { id, name: function.name.clone(), path: self.path.into(),
                decoded_utf8_blake3: source_hash.clone(), span: function.span,
                span_status: if self.source.get(function.span.start..function.span.end).is_some() { "valid_decoded_utf8" } else { "invalid_parser_span" }.into(),
                attributes: function.attributes.iter().map(|attribute| attribute.name.clone()).collect(),
                parameters: function.parameters.iter().map(|parameter| json!({"name": parameter.name, "reference": parameter.is_reference,
                    "optional": parameter.default.is_some(), "span": parameter.span, "phase": "parser_not_type_resolved"})).collect() });
            for attribute in &function.attributes {
                covered.push(attribute.span);
                self.directive(attribute);
            }
            for parameter in &function.parameters {
                if let Some(default) = &parameter.default {
                    self.expression(default);
                }
            }
            for statement in &function.body {
                covered.push(statement.span);
                self.statement(statement);
            }
        }
        self.owner = None;
    }
}

pub(super) struct Scan {
    pub rows: Vec<Appearance>,
    pub diagnostics: Vec<Value>,
    pub user_functions: BTreeSet<String>,
    pub functions: Vec<ParsedFunction>,
}

pub(super) fn scan(sources: &[(String, String)], options: &AnalyzerOptions) -> Scan {
    let mut context = DefaultParserContext::default();
    context.set_compatibility(options.compatibility.clone());
    context.set_lexer_compatibility(
        options.allow_full_width_space,
        options.debug_semicolon,
        options.ignore_triple_symbols,
    );
    context.define_preprocessor_symbol("__DEBUG__", i64::from(options.debug_mode));
    let mut result = Scan {
        rows: Vec::new(),
        diagnostics: Vec::new(),
        user_functions: BTreeSet::new(),
        functions: Vec::new(),
    };
    for (path, text) in sources {
        let first_function = result.functions.len();
        crate::watchdog::publish_or_exit(
            json!({"phase": "appearance_parse", "case": path, "pending": "parse_source", "source_bytes": text.len(), "appearances_completed": result.rows.len(), "diagnostics": result.diagnostics, "lastFullResponse": null}),
        );
        let parsed = if path.to_ascii_lowercase().ends_with(".erh") {
            parse_erh(text, &mut context)
        } else {
            parse_erb(text, &mut context)
        };
        result.diagnostics.extend(
            parsed
                .diagnostics
                .iter()
                .map(|diagnostic| json!({"path": path, "diagnostic": diagnostic})),
        );
        let mut covered = Vec::new();
        if let Some(script) = parsed.value {
            result.user_functions.extend(
                script
                    .functions
                    .iter()
                    .map(|function| function.name.to_ascii_uppercase()),
            );
            Walker {
                path,
                source: text,
                activity: "active_ast",
                offset: 0,
                context: &context,
                rows: &mut result.rows,
                owner: None,
                ownership_status: "top_level",
            }
            .script(&script, &mut covered, &mut result.functions);
        }
        // Recover appearances from lines excluded by preprocessing or parser recovery,
        // but never label those lexical candidates as active executable code.
        covered.sort_by_key(|span| span.start);
        let mut coverage_cursor = 0;
        let mut offset = 0;
        let mut lexical_owner = None;
        let header_owners = result.functions[first_function..]
            .iter()
            .filter(|function| function.span_status == "valid_decoded_utf8")
            .map(|function| (function.span.start, function.id))
            .collect::<BTreeMap<_, _>>();
        for line in text.split_inclusive('\n') {
            let trimmed = line.trim_start();
            let start = offset + line.len() - trimmed.len();
            if trimmed.starts_with('@') {
                // An excluded/invalid header breaks ownership inference; do not
                // attribute its body to the preceding active function.
                lexical_owner = header_owners
                    .range(offset..offset + line.len())
                    .next()
                    .map(|(_, &id)| id);
            }
            while coverage_cursor < covered.len() && covered[coverage_cursor].end <= start {
                coverage_cursor += 1;
            }
            let included = covered
                .get(coverage_cursor)
                .is_some_and(|span| span.start <= start && start < span.end);
            if !included
                && !trimmed.is_empty()
                && !trimmed.starts_with([';', '@', '#', '[', '{', '}'])
                && let Some(statement) = parse_line(line, &context).value
            {
                Walker {
                    path,
                    source: text,
                    activity: "unverified_not_in_active_ast",
                    offset,
                    context: &context,
                    rows: &mut result.rows,
                    owner: lexical_owner,
                    ownership_status: "lexical_header_candidate_not_ast_membership",
                }
                .statement(&statement);
            }
            offset += line.len();
        }
    }
    result.rows.sort_by(|left, right| {
        (&left.path, left.span.start, &left.api, &left.form).cmp(&(
            &right.path,
            right.span.start,
            &right.api,
            &right.form,
        ))
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_parser_span_remains_visible_but_never_claims_active_evidence() {
        let mut rows = Vec::new();
        Walker {
            path: "a.erb",
            source: "PRINTL text",
            activity: "active_ast",
            offset: 0,
            context: &Default::default(),
            rows: &mut rows,
            owner: Some(3),
            ownership_status: "parser_function_membership_not_execution",
        }
        .add("PRINTL", "instruction", Some(1), 0, Span::new(9, 2), None);
        assert_eq!(rows[0].span, Span::new(9, 2));
        assert_eq!(rows[0].span_status, "invalid_parser_span");
        assert_eq!(rows[0].activity, "unverified_invalid_parser_span");
        assert!(rows[0].raw.is_empty());
        assert_eq!(
            rows[0].owning_function,
            Some(3),
            "structural membership is retained without validating the span"
        );
    }

    #[test]
    fn scans_uncalled_functions_and_preserves_unknown_and_preprocessed_apis() {
        let sources = vec![("ERB/a.erb".into(), "@SYSTEM_TITLE\nRETURN\n@UNUSED\nSQL_CONNECT \"x\"\nRESULT = DT_COLUMN_OPTIONS(\"x\")\n[SKIPSTART]\n@REMOVED_FUNCTION\nREMOVED_API 1\n[SKIPEND]\nRETURN\n".into())];
        let report = scan(&sources, &AnalyzerOptions::analysis_mode());
        assert!(
            report
                .rows
                .iter()
                .any(|row| row.api == "SQL_CONNECT" && row.activity == "active_ast")
        );
        let call = report
            .rows
            .iter()
            .find(|row| row.api == "DT_COLUMN_OPTIONS")
            .unwrap();
        assert_eq!(call.activity, "unverified_type_directed_rhs");
        assert_eq!(call.raw, "DT_COLUMN_OPTIONS(\"x\")");
        assert_eq!(call.arity, Some(1));
        assert!(
            report
                .rows
                .iter()
                .any(|row| row.api == "REMOVED_API"
                    && row.activity == "unverified_not_in_active_ast")
        );
        assert!(
            report
                .rows
                .iter()
                .find(|row| row.api == "REMOVED_API")
                .unwrap()
                .owning_function
                .is_none()
        );
    }

    #[test]
    fn appearance_spans_are_utf8_bytes_and_dynamic_targets_are_retained() {
        let source = "@SYSTEM_TITLE\nPRINTL 日本語\nCALLFORM TARGET_{1}(2)\nRETURN\n";
        let report = scan(
            &[("x.erb".into(), source.into())],
            &AnalyzerOptions::analysis_mode(),
        );
        let call = report
            .rows
            .iter()
            .find(|row| row.api == "CALLFORM")
            .unwrap();
        assert_eq!(
            source.get(call.span.start..call.span.end),
            Some(call.raw.as_str())
        );
        assert!(
            call.dynamic_target
                .as_deref()
                .is_some_and(|target| target.contains("TARGET_"))
        );
    }

    #[test]
    fn dynamic_methods_inside_form_defaults_and_numeric_rhs_keep_ownership_and_patterns() {
        let source = "@SYSTEM_TITLE\nCALLFORM CAN_MOVE_{ARG}(1)\nPRINTFORML %GETMETHS(\"ODEKAKEMAP_SETTING_\" + ARGS, \"\")%\nPRINTFORML %STRFORM(ARGS)%\nRESULT = GETMETH(\"CAN_MOVE_\" + \"A\", 0)\nRETURN\n@UNUSED(ARG = GETMETH(\"FALLBACK\", 1))\nRETURN\n";
        let report = scan(
            &[("a.erb".into(), source.into())],
            &AnalyzerOptions::analysis_mode(),
        );
        assert_eq!(report.functions.len(), 2);
        for name in ["CALLFORM", "GETMETHS", "GETMETH"] {
            assert!(report.rows.iter().any(|row| row.api == name
                && row.owning_function == Some(0)
                && row.target.is_some()));
        }
        let call = report
            .rows
            .iter()
            .find(|row| row.api == "CALLFORM")
            .unwrap();
        let pattern = &call.target.as_ref().unwrap().pattern;
        assert!(pattern.matches("CAN_MOVE_11", true));
        assert!(!pattern.matches("OTHER_11", true));
        assert!(
            report
                .rows
                .iter()
                .filter(|row| row.api == "GETMETH")
                .any(
                    |row| row.target.as_ref().unwrap().pattern.exact().as_deref()
                        == Some("CAN_MOVE_A")
                )
        );
        assert!(
            report
                .rows
                .iter()
                .any(|row| row.api == "GETMETH" && row.owning_function == Some(1))
        );
        let generated = report
            .rows
            .iter()
            .find(|row| row.api == "STRFORM")
            .unwrap()
            .target
            .as_ref()
            .unwrap();
        assert_eq!(generated.dispatch, "dynamic_form_evaluation");
        assert!(generated.pattern.matches("ANY_USER_FUNCTION", true));
    }

    #[test]
    fn unknown_name_segments_preserve_suffix_anchors_and_literal_glob_characters() {
        use super::super::targets::{Pattern, Segment};
        let pattern = Pattern {
            segments: vec![
                Segment::Unknown("runtime".into()),
                Segment::Literal("AB".into()),
            ],
        };
        assert!(pattern.matches("ABAB", true));
        assert!(!pattern.matches("ABA", true));
        let pattern = Pattern {
            segments: vec![
                Segment::Literal("CAN_*_".into()),
                Segment::Unknown("runtime".into()),
                Segment::Literal("_END".into()),
            ],
        };
        assert!(pattern.matches("CAN_*_A_END", true));
        assert!(!pattern.matches("CAN_X_A_END", true));
        assert!(
            Pattern {
                segments: vec![Segment::Unknown("runtime".into())]
            }
            .matches("ANY_METHOD", false)
        );
    }
}
