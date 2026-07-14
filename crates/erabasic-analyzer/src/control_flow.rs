use std::collections::BTreeMap;

use erabasic_hir::{
    ControlFlowEdge, ControlFlowKind, HirArgument, HirStatement, HirStatementKind, LabelId, LineId,
    SourceId,
};

use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, symbols::Symbols,
};

struct OpenBlock {
    name: String,
    line: LineId,
    alternatives: Vec<LineId>,
}

#[allow(clippy::too_many_lines)]
pub(crate) fn build_control_flow(
    lines: &[HirStatement],
    symbols: &Symbols,
    source: SourceId,
    path: &str,
    text: &str,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> (Vec<(LabelId, String, LineId)>, Vec<ControlFlowEdge>) {
    let mut edges = Vec::new();
    let mut labels = Vec::new();
    let mut label_by_name = BTreeMap::new();
    for line in lines {
        if let HirStatementKind::Label { label, name } = &line.kind {
            labels.push((*label, name.clone(), line.id));
            label_by_name.insert(name.to_ascii_uppercase(), (*label, line.id));
        }
    }

    for pair in lines.windows(2) {
        if falls_through(&pair[0]) {
            edges.push(ControlFlowEdge {
                kind: ControlFlowKind::Next,
                from: pair[0].id,
                to: Some(pair[1].id),
                function: None,
                label: None,
            });
        }
    }

    let mut blocks: Vec<OpenBlock> = Vec::new();
    for line in lines {
        let HirStatementKind::Instruction { name, arguments } = &line.kind else {
            continue;
        };
        match name.as_str() {
            "IF" | "SELECTCASE" | "REPEAT" | "FOR" | "WHILE" | "DO" | "TRYC" | "PRINTDATA"
            | "STRDATA" | "TRYLIST" | "NOSKIP" => blocks.push(OpenBlock {
                name: name.clone(),
                line: line.id,
                alternatives: Vec::new(),
            }),
            "ELSE" | "ELSEIF" => match blocks.last_mut() {
                Some(block) if block.name == "IF" => block.alternatives.push(line.id),
                _ => invalid_flow(
                    line,
                    source,
                    path,
                    text,
                    diagnostics,
                    format!("{name} is outside an IF block"),
                ),
            },
            "CASE" | "CASEELSE" => match blocks.last_mut() {
                Some(block) if block.name == "SELECTCASE" => block.alternatives.push(line.id),
                _ => invalid_flow(
                    line,
                    source,
                    path,
                    text,
                    diagnostics,
                    format!("{name} is outside a SELECTCASE block"),
                ),
            },
            "ENDIF" => close_block(
                "IF",
                line,
                &mut blocks,
                &mut edges,
                source,
                path,
                text,
                diagnostics,
            ),
            "ENDSELECT" => close_block(
                "SELECTCASE",
                line,
                &mut blocks,
                &mut edges,
                source,
                path,
                text,
                diagnostics,
            ),
            "REND" => close_loop(
                "REPEAT",
                line,
                &mut blocks,
                &mut edges,
                source,
                path,
                text,
                diagnostics,
            ),
            "NEXT" => close_loop(
                "FOR",
                line,
                &mut blocks,
                &mut edges,
                source,
                path,
                text,
                diagnostics,
            ),
            "WEND" => close_loop(
                "WHILE",
                line,
                &mut blocks,
                &mut edges,
                source,
                path,
                text,
                diagnostics,
            ),
            "LOOP" => close_loop(
                "DO",
                line,
                &mut blocks,
                &mut edges,
                source,
                path,
                text,
                diagnostics,
            ),
            "ENDCATCH" => close_block(
                "TRYC",
                line,
                &mut blocks,
                &mut edges,
                source,
                path,
                text,
                diagnostics,
            ),
            "ENDDATA" => close_block(
                "PRINTDATA",
                line,
                &mut blocks,
                &mut edges,
                source,
                path,
                text,
                diagnostics,
            ),
            "ENDLIST" => close_block(
                "TRYLIST",
                line,
                &mut blocks,
                &mut edges,
                source,
                path,
                text,
                diagnostics,
            ),
            "ENDNOSKIP" => close_block(
                "NOSKIP",
                line,
                &mut blocks,
                &mut edges,
                source,
                path,
                text,
                diagnostics,
            ),
            "BREAK" | "CONTINUE" => {
                if let Some(block) = blocks
                    .iter()
                    .rev()
                    .find(|block| matches!(block.name.as_str(), "REPEAT" | "FOR" | "WHILE" | "DO"))
                {
                    edges.push(ControlFlowEdge {
                        kind: if name == "BREAK" {
                            ControlFlowKind::Break
                        } else {
                            ControlFlowKind::Continue
                        },
                        from: line.id,
                        to: Some(block.line),
                        function: None,
                        label: None,
                    });
                } else {
                    invalid_flow(
                        line,
                        source,
                        path,
                        text,
                        diagnostics,
                        format!("{name} is outside a loop"),
                    );
                }
            }
            "GOTO" | "TRYGOTO" => {
                if let Some(target) = raw_target(arguments) {
                    if let Some((label, target_line)) =
                        label_by_name.get(&target.to_ascii_uppercase())
                    {
                        edges.push(ControlFlowEdge {
                            kind: ControlFlowKind::Goto,
                            from: line.id,
                            to: Some(*target_line),
                            function: None,
                            label: Some(*label),
                        });
                    } else if name == "GOTO" {
                        diagnostics.push(AnalyzerDiagnostic::at(
                            AnalyzerDiagnosticCode::UndefinedLabel,
                            AnalyzerDiagnosticSeverity::Error,
                            2,
                            source,
                            path,
                            text,
                            line.location.span,
                            format!("undefined local label {target}"),
                        ));
                    }
                }
            }
            "CALL" | "CALLF" | "TRYCALL" | "JUMP" | "TRYJUMP" | "BEGIN" => {
                if let Some(target) = raw_target(arguments)
                    && let Some(function) = symbols.function(target)
                {
                    edges.push(ControlFlowEdge {
                        kind: if matches!(name.as_str(), "JUMP" | "TRYJUMP" | "BEGIN") {
                            ControlFlowKind::Jump
                        } else {
                            ControlFlowKind::Call
                        },
                        from: line.id,
                        to: None,
                        function: Some(function.id),
                        label: None,
                    });
                }
            }
            "RETURN" | "RETURNF" | "RETURNFORM" => edges.push(ControlFlowEdge {
                kind: ControlFlowKind::Return,
                from: line.id,
                to: None,
                function: None,
                label: None,
            }),
            _ => {}
        }
    }
    for block in blocks {
        if let Some(line) = lines.get(block.line.0 as usize) {
            invalid_flow(
                line,
                source,
                path,
                text,
                diagnostics,
                format!("unclosed {} block", block.name),
            );
        }
    }
    (labels, edges)
}

#[allow(clippy::too_many_arguments)]
fn close_block(
    expected: &str,
    line: &HirStatement,
    blocks: &mut Vec<OpenBlock>,
    edges: &mut Vec<ControlFlowEdge>,
    source: SourceId,
    path: &str,
    text: &str,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) {
    let Some(block) = blocks.pop() else {
        invalid_flow(
            line,
            source,
            path,
            text,
            diagnostics,
            format!("unexpected closing instruction for {expected}"),
        );
        return;
    };
    if block.name != expected {
        invalid_flow(
            line,
            source,
            path,
            text,
            diagnostics,
            format!(
                "expected the end of {}, found the end of {expected}",
                block.name
            ),
        );
        return;
    }
    for from in std::iter::once(block.line).chain(block.alternatives) {
        edges.push(ControlFlowEdge {
            kind: ControlFlowKind::Branch,
            from,
            to: Some(line.id),
            function: None,
            label: None,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn close_loop(
    expected: &str,
    line: &HirStatement,
    blocks: &mut Vec<OpenBlock>,
    edges: &mut Vec<ControlFlowEdge>,
    source: SourceId,
    path: &str,
    text: &str,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) {
    let before = edges.len();
    let opener = blocks.last().map(|block| block.line);
    close_block(
        expected,
        line,
        blocks,
        edges,
        source,
        path,
        text,
        diagnostics,
    );
    if edges.len() > before
        && let Some(opener) = opener
    {
        edges.push(ControlFlowEdge {
            kind: ControlFlowKind::LoopBack,
            from: line.id,
            to: Some(opener),
            function: None,
            label: None,
        });
    }
}

fn raw_target(arguments: &[HirArgument]) -> Option<&str> {
    match arguments.first()? {
        HirArgument::Raw(value) => Some(value.trim().trim_matches('"')),
        _ => None,
    }
}

fn falls_through(line: &HirStatement) -> bool {
    !matches!(
        &line.kind,
        HirStatementKind::Instruction { name, .. }
            if matches!(
                name.as_str(),
                "RETURN" | "RETURNF" | "RETURNFORM" | "JUMP" | "BEGIN" | "GOTO" | "QUIT"
                    | "BREAK" | "CONTINUE"
            )
    )
}

fn invalid_flow(
    line: &HirStatement,
    source: SourceId,
    path: &str,
    text: &str,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
    message: impl Into<String>,
) {
    diagnostics.push(AnalyzerDiagnostic::at(
        AnalyzerDiagnosticCode::InvalidControlFlow,
        AnalyzerDiagnosticSeverity::Error,
        2,
        source,
        path,
        text,
        line.location.span,
        message,
    ));
}
