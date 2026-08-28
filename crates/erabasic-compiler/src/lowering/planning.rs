//! Deterministic source identities and structured control-flow precomputation.

use super::{
    Builder, ControlFlowKind, DenseIdIndex, Function, HirArgument, HirExpr, HirExprKind,
    HirFormPart, HirFormattedString, HirStatementKind, LineId, Opcode, SourceLocation, opcode,
};

#[cfg(test)]
use super::{AssignOp, BinaryOp, Digest, InstructionTarget, SemanticType};
#[cfg(test)]
use erabasic_hir::{ConstantValue, HirPlace};

mod fingerprint;

pub(super) use fingerprint::statement_fingerprint;

pub(super) fn strip_source_locations(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            fields.remove("location");
            for value in fields.values_mut() {
                strip_source_locations(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                strip_source_locations(value);
            }
        }
        _ => {}
    }
}

pub(super) struct DataBlock<'a> {
    pub(super) opener: &'a erabasic_hir::HirStatement,
    pub(super) choices: Vec<Vec<&'a erabasic_hir::HirStatement>>,
}

pub(super) struct TryListBlock<'a> {
    pub(super) opener: &'a erabasic_hir::HirStatement,
    pub(super) candidates: Vec<&'a erabasic_hir::HirStatement>,
}

pub(super) enum TryListLine<'a> {
    Opener(TryListBlock<'a>),
    Body,
}

pub(super) enum DataLine<'a> {
    Opener(DataBlock<'a>),
    Body,
}

pub(super) fn collect_try_lists(function: &Function) -> DenseIdIndex<TryListLine<'_>> {
    let mut lines = DenseIdIndex::new(function.lines.len());
    let mut index = 0;
    while index < function.lines.len() {
        let opener = &function.lines[index];
        if !matches!(
            instruction_name(opener),
            Some("TRYCALLLIST" | "TRYJUMPLIST" | "TRYGOTOLIST")
        ) {
            index += 1;
            continue;
        }
        let mut candidates = Vec::new();
        let mut cursor = index + 1;
        while cursor < function.lines.len() {
            let candidate = &function.lines[cursor];
            lines.insert(candidate.id.0, TryListLine::Body);
            if instruction_name(candidate) == Some("ENDFUNC") {
                cursor += 1;
                break;
            }
            if instruction_name(candidate) == Some("FUNC") {
                candidates.push(candidate);
            }
            cursor += 1;
        }
        lines.insert(
            opener.id.0,
            TryListLine::Opener(TryListBlock { opener, candidates }),
        );
        index = cursor;
    }
    lines
}

pub(super) fn collect_data_blocks(function: &Function) -> DenseIdIndex<DataLine<'_>> {
    let mut lines = DenseIdIndex::new(function.lines.len());
    let mut index = 0;
    while index < function.lines.len() {
        let line = &function.lines[index];
        let Some(name) = instruction_name(line) else {
            index += 1;
            continue;
        };
        if name != "STRDATA" && !name.starts_with("PRINTDATA") {
            index += 1;
            continue;
        }
        let mut choices = Vec::new();
        let mut cursor = index + 1;
        while cursor < function.lines.len() {
            let candidate = &function.lines[cursor];
            lines.insert(candidate.id.0, DataLine::Body);
            match instruction_name(candidate) {
                Some("ENDDATA") => {
                    cursor += 1;
                    break;
                }
                Some("DATALIST") => {
                    let mut group = Vec::new();
                    cursor += 1;
                    while cursor < function.lines.len() {
                        let member = &function.lines[cursor];
                        lines.insert(member.id.0, DataLine::Body);
                        if instruction_name(member) == Some("ENDLIST") {
                            break;
                        }
                        if matches!(instruction_name(member), Some("DATA" | "DATAFORM")) {
                            group.push(member);
                        }
                        cursor += 1;
                    }
                    choices.push(group);
                }
                Some("DATA" | "DATAFORM") => choices.push(vec![candidate]),
                _ => {}
            }
            cursor += 1;
        }
        lines.insert(
            line.id.0,
            DataLine::Opener(DataBlock {
                opener: line,
                choices,
            }),
        );
        index = cursor;
    }
    lines
}

pub(super) fn instruction_name(line: &erabasic_hir::HirStatement) -> Option<&str> {
    match &line.kind {
        HirStatementKind::Instruction { target, .. } => Some(target.name()),
        _ => None,
    }
}

pub(super) fn argument_place(argument: Option<&HirArgument>) -> Option<&erabasic_hir::HirPlace> {
    match argument? {
        HirArgument::Place(place)
        | HirArgument::Expression(HirExpr {
            kind: HirExprKind::Variable { place },
            ..
        }) => Some(place),
        HirArgument::MixedExpression { .. }
        | HirArgument::Expression(_)
        | HirArgument::Formatted(_)
        | HirArgument::Raw(_)
        | HirArgument::Omitted => None,
    }
}

pub(super) fn formatted_constant(value: &HirFormattedString) -> Option<String> {
    let mut result = String::new();
    for part in &value.parts {
        match part {
            HirFormPart::Text { value } => result.push_str(value),
            HirFormPart::Triple { symbol, .. } => result.push(*symbol),
            HirFormPart::Interpolation { .. } | HirFormPart::Conditional { .. } => return None,
        }
    }
    Some(result)
}

pub(super) fn add_control_flow(
    line: LineId,
    location: SourceLocation,
    builder: &mut Builder<'_>,
    structured: &StructuredFlow,
    outgoing: &[&erabasic_hir::ControlFlowEdge],
    pending: &mut Vec<(usize, LineId, bool)>,
) {
    if let Some(target) = structured.false_target(line) {
        if builder
            .code
            .last()
            .is_some_and(|instruction| instruction.opcode == Opcode::JumpDynamicLabel as u16)
        {
            pending.push((builder.code.len() - 1, *target, true));
            return;
        }
        let instruction = builder.code.len();
        builder.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
        pending.push((instruction, *target, true));
        return;
    }
    if structured.alternative_end(line).is_none()
        && let Some(branch) = outgoing
            .iter()
            .find(|edge| edge.kind == ControlFlowKind::Branch)
        && let Some(target) = branch.to
    {
        let instruction = builder.code.len();
        builder.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
        pending.push((instruction, target, false));
        return;
    }
    if let Some(edge) = outgoing.iter().find(|edge| {
        matches!(
            edge.kind,
            ControlFlowKind::Goto
                | ControlFlowKind::Jump
                | ControlFlowKind::LoopBack
                | ControlFlowKind::Break
                | ControlFlowKind::Continue
        )
    }) && let Some(target) = edge.to
    {
        let instruction = builder.code.len();
        builder.emit(opcode::jump(Opcode::Jump, 0), location);
        pending.push((instruction, target, false));
    }
}

pub(super) struct StructuredFlow {
    targets: DenseIdIndex<StructuredTargets>,
}

#[derive(Default)]
struct StructuredTargets {
    false_target: Option<LineId>,
    alternative_end: Option<LineId>,
}

impl StructuredFlow {
    pub(super) fn false_target(&self, line: LineId) -> Option<&LineId> {
        self.targets.get(line.0)?.false_target.as_ref()
    }

    pub(super) fn alternative_end(&self, line: LineId) -> Option<&LineId> {
        self.targets.get(line.0)?.alternative_end.as_ref()
    }

    fn set_false_target(&mut self, line: LineId, target: LineId) {
        self.targets
            .get_or_insert_with(line.0, StructuredTargets::default)
            .expect("validated structured-flow line IDs are in range")
            .false_target = Some(target);
    }

    fn set_alternative_end(&mut self, line: LineId, target: LineId) {
        self.targets
            .get_or_insert_with(line.0, StructuredTargets::default)
            .expect("validated structured-flow line IDs are in range")
            .alternative_end = Some(target);
    }
}

struct OpenIf {
    opener: LineId,
    alternatives: Vec<(LineId, bool)>,
}

pub(super) fn structured_if_flow(function: &Function) -> StructuredFlow {
    let mut result = StructuredFlow {
        targets: DenseIdIndex::new(function.lines.len()),
    };
    let mut open = Vec::<OpenIf>::new();
    let mut select_open = Vec::<(LineId, Vec<LineId>)>::new();
    for line in &function.lines {
        let HirStatementKind::Instruction { target, .. } = &line.kind else {
            continue;
        };
        match target.name() {
            "SELECTCASE" => select_open.push((line.id, Vec::new())),
            "CASE" | "CASEELSE" => {
                if let Some((_, cases)) = select_open.last_mut() {
                    cases.push(line.id);
                }
            }
            "ENDSELECT" => {
                let Some((_, cases)) = select_open.pop() else {
                    continue;
                };
                for pair in cases.windows(2) {
                    result.set_false_target(pair[0], pair[1]);
                    result.set_alternative_end(pair[1], line.id);
                }
                if let Some(last) = cases.last() {
                    result.set_false_target(*last, line.id);
                }
            }
            "IF" | "TRYCCALL" | "TRYCCALLFORM" | "TRYCCALLSTR" | "TRYCJUMP" | "TRYCJUMPFORM"
            | "TRYCJUMPSTR" | "TRYCGOTO" | "TRYCGOTOFORM" => open.push(OpenIf {
                opener: line.id,
                alternatives: Vec::new(),
            }),
            "ELSEIF" => {
                if let Some(frame) = open.last_mut() {
                    frame.alternatives.push((line.id, true));
                }
            }
            "ELSE" | "CATCH" => {
                if let Some(frame) = open.last_mut() {
                    frame.alternatives.push((line.id, false));
                }
            }
            "ENDIF" | "ENDCATCH" => {
                let Some(frame) = open.pop() else {
                    continue;
                };
                let mut previous_condition = Some(frame.opener);
                for (alternative, is_condition) in frame.alternatives {
                    if let Some(condition) = previous_condition {
                        result.set_false_target(condition, alternative);
                    }
                    result.set_alternative_end(alternative, line.id);
                    previous_condition = is_condition.then_some(alternative);
                }
                if let Some(condition) = previous_condition {
                    result.set_false_target(condition, line.id);
                }
            }
            _ => {}
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_fingerprint(kind: &HirStatementKind) -> Digest {
        let mut value = serde_json::to_value(kind).unwrap();
        strip_source_locations(&mut value);
        let bytes = serde_json::to_vec(&value).unwrap();
        let mut expected = Digest::hash("rustyera.bytecode.source-statement.v1", &[&bytes]);
        expected.0[16..].fill(0);
        expected
    }

    #[test]
    fn reused_fingerprint_buffer_preserves_the_canonical_digest() {
        let kind = HirStatementKind::Label {
            label: erabasic_hir::LabelId(7),
            name: "LABEL".into(),
        };

        assert_eq!(statement_fingerprint(&kind), legacy_fingerprint(&kind));
    }

    #[test]
    fn cached_empty_builtin_fingerprint_preserves_the_canonical_digest() {
        let kind = HirStatementKind::Instruction {
            target: InstructionTarget::Builtin("RETURN".into()),
            arguments: Vec::new(),
        };
        let expected = legacy_fingerprint(&kind);

        assert_eq!(statement_fingerprint(&kind), expected);
        assert_eq!(statement_fingerprint(&kind), expected);
    }

    #[test]
    fn cached_extension_and_label_fingerprints_preserve_canonical_digests() {
        let kinds = [
            HirStatementKind::Instruction {
                target: InstructionTarget::Extension("EXTENSION".into()),
                arguments: Vec::new(),
            },
            HirStatementKind::Label {
                label: erabasic_hir::LabelId(3),
                name: "REUSED".into(),
            },
        ];
        for kind in kinds {
            let expected = legacy_fingerprint(&kind);
            assert_eq!(statement_fingerprint(&kind), expected);
            assert_eq!(statement_fingerprint(&kind), expected);
        }
    }

    #[test]
    fn cached_integer_instruction_fingerprint_preserves_the_canonical_digest() {
        let kind = HirStatementKind::Instruction {
            target: InstructionTarget::Builtin("RETURN".into()),
            arguments: vec![HirArgument::Expression(HirExpr {
                kind: HirExprKind::Integer { value: -1 },
                value_type: SemanticType::Integer,
                constant: Some(ConstantValue::Integer(-1)),
                location: SourceLocation::default(),
            })],
        };
        let expected = legacy_fingerprint(&kind);

        assert_eq!(statement_fingerprint(&kind), expected);
        assert_eq!(statement_fingerprint(&kind), expected);
    }

    #[test]
    fn cached_integer_assignment_fingerprints_preserve_canonical_digests() {
        let expression = |value| HirExpr {
            kind: HirExprKind::Integer { value },
            value_type: SemanticType::Integer,
            constant: Some(ConstantValue::Integer(value)),
            location: SourceLocation::default(),
        };
        let kinds = [
            HirStatementKind::Assignment {
                target: HirPlace {
                    variable: erabasic_hir::VariableId(9),
                    indices: Vec::new(),
                    value_type: SemanticType::Integer,
                    mutable: true,
                    location: SourceLocation::default(),
                },
                op: AssignOp::Assign,
                value: expression(0),
            },
            HirStatementKind::Assignment {
                target: HirPlace {
                    variable: erabasic_hir::VariableId(9),
                    indices: vec![expression(3)],
                    value_type: SemanticType::Integer,
                    mutable: true,
                    location: SourceLocation::default(),
                },
                op: AssignOp::Assign,
                value: expression(-1),
            },
        ];
        for kind in kinds {
            let expected = legacy_fingerprint(&kind);
            assert_eq!(statement_fingerprint(&kind), expected);
            assert_eq!(statement_fingerprint(&kind), expected);
        }
    }

    #[test]
    fn simple_formatted_fast_path_preserves_the_canonical_digest() {
        for target in [
            InstructionTarget::Builtin("PRINTFORMW".into()),
            InstructionTarget::Extension("CUSTOM".into()),
            InstructionTarget::Unresolved("MISSING".into()),
        ] {
            let kind = HirStatementKind::Instruction {
                target,
                arguments: vec![HirArgument::Formatted(HirFormattedString {
                    parts: vec![
                        HirFormPart::Text {
                            value: "quoted \" text\n文字".into(),
                        },
                        HirFormPart::Interpolation {
                            expression: Box::new(HirExpr {
                                kind: HirExprKind::Variable {
                                    place: HirPlace {
                                        variable: erabasic_hir::VariableId(5),
                                        indices: vec![HirExpr {
                                            kind: HirExprKind::Integer { value: 2 },
                                            value_type: SemanticType::Integer,
                                            constant: Some(ConstantValue::Integer(2)),
                                            location: SourceLocation::default(),
                                        }],
                                        value_type: SemanticType::String,
                                        mutable: true,
                                        location: SourceLocation::default(),
                                    },
                                },
                                value_type: SemanticType::String,
                                constant: None,
                                location: SourceLocation::default(),
                            }),
                            width: Some(Box::new(HirExpr {
                                kind: HirExprKind::Integer { value: 12 },
                                value_type: SemanticType::Integer,
                                constant: Some(ConstantValue::Integer(12)),
                                location: SourceLocation::default(),
                            })),
                            alignment: Some(erabasic_ast::Alignment::Right),
                            integer: false,
                            location: SourceLocation::default(),
                        },
                        HirFormPart::Triple {
                            symbol: '*',
                            location: SourceLocation::default(),
                        },
                    ],
                    location: SourceLocation::default(),
                })],
            };

            assert_eq!(statement_fingerprint(&kind), legacy_fingerprint(&kind));
        }
    }

    #[test]
    fn simple_raw_argument_fast_path_preserves_the_canonical_digest() {
        for target in [
            InstructionTarget::Builtin("CALL".into()),
            InstructionTarget::Extension("CUSTOM".into()),
            InstructionTarget::Unresolved("MISSING".into()),
        ] {
            let kind = HirStatementKind::Instruction {
                target,
                arguments: vec![HirArgument::Raw("quoted \" target\n文字".into())],
            };

            assert_eq!(statement_fingerprint(&kind), legacy_fingerprint(&kind));
        }
    }

    #[test]
    fn cached_integer_variable_instructions_preserve_canonical_digests() {
        let integer = |value| HirExpr {
            kind: HirExprKind::Integer { value },
            value_type: SemanticType::Integer,
            constant: Some(ConstantValue::Integer(value)),
            location: SourceLocation::default(),
        };
        for indices in [Vec::new(), vec![integer(2)]] {
            let kind = HirStatementKind::Instruction {
                target: InstructionTarget::Builtin("IF".into()),
                arguments: vec![HirArgument::Expression(HirExpr {
                    kind: HirExprKind::Variable {
                        place: HirPlace {
                            variable: erabasic_hir::VariableId(5),
                            indices,
                            value_type: SemanticType::Integer,
                            mutable: true,
                            location: SourceLocation::default(),
                        },
                    },
                    value_type: SemanticType::Integer,
                    constant: None,
                    location: SourceLocation::default(),
                })],
            };
            let expected = legacy_fingerprint(&kind);

            assert_eq!(statement_fingerprint(&kind), expected);
            assert_eq!(statement_fingerprint(&kind), expected);
        }
    }

    #[test]
    fn cached_binary_integer_instructions_preserve_canonical_digests() {
        let integer = HirExpr {
            kind: HirExprKind::Integer { value: -1 },
            value_type: SemanticType::Integer,
            constant: Some(ConstantValue::Integer(-1)),
            location: SourceLocation::default(),
        };
        let variable = HirExpr {
            kind: HirExprKind::Variable {
                place: HirPlace {
                    variable: erabasic_hir::VariableId(5),
                    indices: vec![HirExpr {
                        kind: HirExprKind::Integer { value: 2 },
                        value_type: SemanticType::Integer,
                        constant: Some(ConstantValue::Integer(2)),
                        location: SourceLocation::default(),
                    }],
                    value_type: SemanticType::Integer,
                    mutable: true,
                    location: SourceLocation::default(),
                },
            },
            value_type: SemanticType::Integer,
            constant: None,
            location: SourceLocation::default(),
        };
        for (left, right) in [(variable.clone(), integer.clone()), (integer, variable)] {
            let kind = HirStatementKind::Instruction {
                target: InstructionTarget::Builtin("IF".into()),
                arguments: vec![HirArgument::Expression(HirExpr {
                    kind: HirExprKind::Binary {
                        op: BinaryOp::GreaterEqual,
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    value_type: SemanticType::Integer,
                    constant: None,
                    location: SourceLocation::default(),
                })],
            };
            let expected = legacy_fingerprint(&kind);

            assert_eq!(statement_fingerprint(&kind), expected);
            assert_eq!(statement_fingerprint(&kind), expected);
        }
    }
}
