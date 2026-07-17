//! Deterministic source identities and structured control-flow precomputation.

use super::{
    BTreeMap, BTreeSet, Builder, ControlFlowKind, Digest, Function, HirArgument, HirExpr,
    HirExprKind, HirFormPart, HirFormattedString, HirStatementKind, LineId, Opcode, SourceLocation,
    opcode,
};

pub(super) fn statement_fingerprint(kind: &HirStatementKind) -> Digest {
    let mut value = serde_json::to_value(kind).expect("typed statements are serializable");
    // Source locations are deliberately excluded: inserting unrelated lines must
    // not break a breakpoint anchor for an otherwise identical typed statement.
    strip_source_locations(&mut value);
    let bytes = serde_json::to_vec(&value).expect("normalized statements are serializable");
    Digest::hash("rustyera.bytecode.source-statement.v1", &[&bytes])
}

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

pub(super) fn collect_try_lists(
    function: &Function,
) -> (BTreeMap<LineId, TryListBlock<'_>>, BTreeSet<LineId>) {
    let mut blocks = BTreeMap::new();
    let mut body = BTreeSet::new();
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
            body.insert(candidate.id);
            if instruction_name(candidate) == Some("ENDFUNC") {
                cursor += 1;
                break;
            }
            if instruction_name(candidate) == Some("FUNC") {
                candidates.push(candidate);
            }
            cursor += 1;
        }
        blocks.insert(opener.id, TryListBlock { opener, candidates });
        index = cursor;
    }
    (blocks, body)
}

pub(super) fn collect_data_blocks(
    function: &Function,
) -> (BTreeMap<LineId, DataBlock<'_>>, BTreeSet<LineId>) {
    let mut blocks = BTreeMap::new();
    let mut body = BTreeSet::new();
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
            body.insert(candidate.id);
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
                        body.insert(member.id);
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
        blocks.insert(
            line.id,
            DataBlock {
                opener: line,
                choices,
            },
        );
        index = cursor;
    }
    (blocks, body)
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
        HirArgument::Expression(_)
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
    function: &Function,
    line: LineId,
    location: SourceLocation,
    builder: &mut Builder<'_>,
    structured: &StructuredFlow,
    pending: &mut Vec<(usize, LineId, bool)>,
) {
    if let Some(target) = structured.false_targets.get(&line) {
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
    let outgoing: Vec<_> = function
        .control_flow
        .iter()
        .filter(|edge| edge.from == line)
        .collect();
    if !structured.alternative_ends.contains_key(&line)
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

#[derive(Default)]
pub(super) struct StructuredFlow {
    pub(super) false_targets: BTreeMap<LineId, LineId>,
    pub(super) alternative_ends: BTreeMap<LineId, LineId>,
}

struct OpenIf {
    opener: LineId,
    alternatives: Vec<(LineId, bool)>,
}

pub(super) fn structured_if_flow(function: &Function) -> StructuredFlow {
    let mut result = StructuredFlow::default();
    let mut open = Vec::<OpenIf>::new();
    for line in &function.lines {
        let HirStatementKind::Instruction { target, .. } = &line.kind else {
            continue;
        };
        match target.name() {
            "IF" | "TRYCCALL" | "TRYCCALLFORM" | "TRYCJUMP" | "TRYCJUMPFORM" | "TRYCGOTO"
            | "TRYCGOTOFORM" => open.push(OpenIf {
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
                        result.false_targets.insert(condition, alternative);
                    }
                    result.alternative_ends.insert(alternative, line.id);
                    previous_condition = is_condition.then_some(alternative);
                }
                if let Some(condition) = previous_condition {
                    result.false_targets.insert(condition, line.id);
                }
            }
            _ => {}
        }
    }
    result
}
