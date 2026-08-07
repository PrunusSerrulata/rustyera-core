//! Deterministic source identities and structured control-flow precomputation.

use std::{cell::RefCell, collections::HashMap, sync::OnceLock};

use erabasic_hir::{ConstantValue, HirPlace};

use super::{
    AssignOp, BinaryOp, Builder, ControlFlowKind, DenseIdIndex, Digest, Function, HirArgument,
    HirExpr, HirExprKind, HirFormPart, HirFormattedString, HirStatementKind, InstructionTarget,
    LineId, Opcode, SemanticType, SourceLocation, opcode,
};

pub(super) fn statement_fingerprint(kind: &HirStatementKind) -> Digest {
    if let Some(fingerprint) = simple_instruction_fingerprint(kind) {
        return fingerprint;
    }
    match kind {
        HirStatementKind::Instruction { target, arguments } => {
            if let Some(fingerprint) =
                cached_binary_integer_instruction_fingerprint(kind, target, arguments)
                    .or_else(|| cached_variable_instruction_fingerprint(kind, target, arguments))
            {
                return fingerprint;
            }
            if let Some(value) = canonical_integer_argument(arguments) {
                let (cache, name) = match target {
                    InstructionTarget::Builtin(name) => (IntegerInstructionCache::Builtin, name),
                    InstructionTarget::Extension(name) => {
                        (IntegerInstructionCache::Extension, name)
                    }
                    InstructionTarget::Unresolved(name) => {
                        (IntegerInstructionCache::Unresolved, name)
                    }
                    InstructionTarget::BuiltinMethod { .. } => {
                        return uncached_statement_fingerprint(kind);
                    }
                };
                return FINGERPRINT_CACHE.with_borrow_mut(|fingerprints| {
                    let values = match cache {
                        IntegerInstructionCache::Builtin => &mut fingerprints.integer_builtin,
                        IntegerInstructionCache::Extension => &mut fingerprints.integer_extension,
                        IntegerInstructionCache::Unresolved => &mut fingerprints.integer_unresolved,
                    };
                    if let Some(fingerprint) =
                        values.get(name).and_then(|by_value| by_value.get(&value))
                    {
                        return *fingerprint;
                    }
                    let fingerprint = uncached_statement_fingerprint(kind);
                    values
                        .entry(name.clone())
                        .or_default()
                        .insert(value, fingerprint);
                    fingerprint
                });
            }
            if arguments.is_empty() {
                let (cache, name) = match target {
                    InstructionTarget::Builtin(name) => (EmptyInstructionCache::Builtin, name),
                    InstructionTarget::Extension(name) => (EmptyInstructionCache::Extension, name),
                    InstructionTarget::Unresolved(name) => {
                        (EmptyInstructionCache::Unresolved, name)
                    }
                    InstructionTarget::BuiltinMethod { .. } => {
                        return uncached_statement_fingerprint(kind);
                    }
                };
                return FINGERPRINT_CACHE.with_borrow_mut(|fingerprints| {
                    let values = match cache {
                        EmptyInstructionCache::Builtin => &mut fingerprints.empty_builtin,
                        EmptyInstructionCache::Extension => &mut fingerprints.empty_extension,
                        EmptyInstructionCache::Unresolved => &mut fingerprints.empty_unresolved,
                    };
                    if let Some(fingerprint) = values.get(name) {
                        return *fingerprint;
                    }
                    let fingerprint = uncached_statement_fingerprint(kind);
                    values.insert(name.clone(), fingerprint);
                    fingerprint
                });
            }
        }
        HirStatementKind::Assignment { .. } => {
            if let Some(key) = canonical_integer_assignment(kind) {
                return FINGERPRINT_CACHE.with_borrow_mut(|fingerprints| {
                    if let Some(fingerprint) = fingerprints.integer_assignments.get(&key) {
                        return *fingerprint;
                    }
                    let fingerprint = uncached_statement_fingerprint(kind);
                    fingerprints.integer_assignments.insert(key, fingerprint);
                    fingerprint
                });
            }
        }
        HirStatementKind::Label { .. } => {}
        HirStatementKind::Error => {
            static ERROR_FINGERPRINT: OnceLock<Digest> = OnceLock::new();
            return *ERROR_FINGERPRINT.get_or_init(|| uncached_statement_fingerprint(kind));
        }
    }
    uncached_statement_fingerprint(kind)
}

fn simple_instruction_fingerprint(kind: &HirStatementKind) -> Option<Digest> {
    if let Some((target_kind, target_name, formatted)) = simple_formatted(kind) {
        return Some(simple_formatted_fingerprint(
            target_kind,
            target_name,
            formatted,
        ));
    }
    let (target_kind, target_name, raw) = simple_raw_argument(kind)?;
    Some(simple_raw_argument_fingerprint(
        target_kind,
        target_name,
        raw,
    ))
}

fn cached_binary_integer_instruction_fingerprint(
    kind: &HirStatementKind,
    target: &InstructionTarget,
    arguments: &[HirArgument],
) -> Option<Digest> {
    let key = canonical_binary_integer_expression(arguments)?;
    let (cache, name) = match target {
        InstructionTarget::Builtin(name) => (VariableInstructionCache::Builtin, name),
        InstructionTarget::Extension(name) => (VariableInstructionCache::Extension, name),
        InstructionTarget::Unresolved(name) => (VariableInstructionCache::Unresolved, name),
        InstructionTarget::BuiltinMethod { .. } => {
            return Some(uncached_statement_fingerprint(kind));
        }
    };
    Some(FINGERPRINT_CACHE.with_borrow_mut(|fingerprints| {
        let values = match cache {
            VariableInstructionCache::Builtin => &mut fingerprints.binary_integer_builtin,
            VariableInstructionCache::Extension => &mut fingerprints.binary_integer_extension,
            VariableInstructionCache::Unresolved => &mut fingerprints.binary_integer_unresolved,
        };
        if let Some(fingerprint) = values
            .get(name)
            .and_then(|by_expression| by_expression.get(&key))
        {
            return *fingerprint;
        }
        let fingerprint = uncached_statement_fingerprint(kind);
        values
            .entry(name.clone())
            .or_default()
            .insert(key, fingerprint);
        fingerprint
    }))
}

fn cached_variable_instruction_fingerprint(
    kind: &HirStatementKind,
    target: &InstructionTarget,
    arguments: &[HirArgument],
) -> Option<Digest> {
    let key = canonical_integer_variable(arguments)?;
    let (cache, name) = match target {
        InstructionTarget::Builtin(name) => (VariableInstructionCache::Builtin, name),
        InstructionTarget::Extension(name) => (VariableInstructionCache::Extension, name),
        InstructionTarget::Unresolved(name) => (VariableInstructionCache::Unresolved, name),
        InstructionTarget::BuiltinMethod { .. } => {
            return Some(uncached_statement_fingerprint(kind));
        }
    };
    Some(FINGERPRINT_CACHE.with_borrow_mut(|fingerprints| {
        let values = match cache {
            VariableInstructionCache::Builtin => &mut fingerprints.integer_variable_builtin,
            VariableInstructionCache::Extension => &mut fingerprints.integer_variable_extension,
            VariableInstructionCache::Unresolved => &mut fingerprints.integer_variable_unresolved,
        };
        if let Some(fingerprint) = values
            .get(name)
            .and_then(|by_variable| by_variable.get(&key))
        {
            return *fingerprint;
        }
        let fingerprint = uncached_statement_fingerprint(kind);
        values
            .entry(name.clone())
            .or_default()
            .insert(key, fingerprint);
        fingerprint
    }))
}

fn simple_formatted(kind: &HirStatementKind) -> Option<(&'static str, &str, &HirFormattedString)> {
    let HirStatementKind::Instruction { target, arguments } = kind else {
        return None;
    };
    let [HirArgument::Formatted(formatted)] = arguments.as_slice() else {
        return None;
    };
    if !formatted.parts.iter().all(|part| match part {
        HirFormPart::Text { .. } | HirFormPart::Triple { .. } => true,
        HirFormPart::Interpolation {
            expression, width, ..
        } => {
            simple_fingerprint_expression(expression)
                && width.as_deref().is_none_or(simple_fingerprint_expression)
        }
        HirFormPart::Conditional { .. } => false,
    }) {
        return None;
    }
    let (target_kind, target_name) = match target {
        InstructionTarget::Builtin(name) => ("builtin", name.as_str()),
        InstructionTarget::Extension(name) => ("extension", name.as_str()),
        InstructionTarget::Unresolved(name) => ("unresolved", name.as_str()),
        InstructionTarget::BuiltinMethod { .. } => return None,
    };
    Some((target_kind, target_name, formatted))
}

fn simple_fingerprint_expression(expression: &HirExpr) -> bool {
    canonical_integer_expression(expression).is_some()
        || simple_variable_expression(expression).is_some()
}

fn simple_variable_expression(expression: &HirExpr) -> Option<&HirPlace> {
    let HirExpr {
        kind: HirExprKind::Variable { place },
        value_type,
        constant: None,
        ..
    } = expression
    else {
        return None;
    };
    if place.value_type != *value_type
        || !matches!(value_type, SemanticType::Integer | SemanticType::String)
        || !matches!(place.indices.as_slice(), [] | [_])
        || !place
            .indices
            .iter()
            .all(|index| canonical_integer_expression(index).is_some())
    {
        return None;
    }
    Some(place)
}

fn simple_formatted_fingerprint(
    target_kind: &str,
    target_name: &str,
    formatted: &HirFormattedString,
) -> Digest {
    FINGERPRINT_JSON.with_borrow_mut(|bytes| {
        bytes.clear();
        bytes.extend_from_slice(br#"{"arguments":[{"kind":"formatted","value":{"parts":["#);
        for (index, part) in formatted.parts.iter().enumerate() {
            if index != 0 {
                bytes.push(b',');
            }
            match part {
                HirFormPart::Text { value } => {
                    bytes.extend_from_slice(br#"{"kind":"text","value":"#);
                    serde_json::to_writer(&mut *bytes, value)
                        .expect("formatted text is serializable");
                    bytes.push(b'}');
                }
                HirFormPart::Interpolation {
                    expression,
                    width,
                    alignment,
                    integer,
                    ..
                } => {
                    bytes.extend_from_slice(br#"{"alignment":"#);
                    serde_json::to_writer(&mut *bytes, alignment)
                        .expect("formatted alignment is serializable");
                    bytes.extend_from_slice(br#","expression":"#);
                    append_simple_expression(bytes, expression);
                    bytes.extend_from_slice(br#","integer":"#);
                    bytes.extend_from_slice(if *integer { b"true" } else { b"false" });
                    bytes.extend_from_slice(br#","kind":"interpolation","width":"#);
                    if let Some(width) = width {
                        append_simple_expression(bytes, width);
                    } else {
                        bytes.extend_from_slice(b"null");
                    }
                    bytes.push(b'}');
                }
                HirFormPart::Triple { symbol, .. } => {
                    bytes.extend_from_slice(br#"{"kind":"triple","symbol":"#);
                    serde_json::to_writer(&mut *bytes, symbol)
                        .expect("formatted triple symbol is serializable");
                    bytes.push(b'}');
                }
                HirFormPart::Conditional { .. } => {
                    unreachable!("simple formatted strings exclude conditional parts")
                }
            }
        }
        bytes.extend_from_slice(br#"]}}],"kind":"instruction","target":{"kind":"#);
        append_simple_target(bytes, target_kind, target_name);
        fingerprint_digest(bytes)
    })
}

fn append_simple_expression(bytes: &mut Vec<u8>, expression: &HirExpr) {
    if let Some(value) = canonical_integer_expression(expression) {
        bytes.extend_from_slice(br#"{"constant":{"type":"integer","value":"#);
        serde_json::to_writer(&mut *bytes, &value).expect("integer constant is serializable");
        bytes.extend_from_slice(br#"},"kind":{"kind":"integer","value":"#);
        serde_json::to_writer(&mut *bytes, &value).expect("integer expression is serializable");
        bytes.extend_from_slice(br#"},"value_type":"integer"}"#);
        return;
    }
    let place = simple_variable_expression(expression)
        .expect("simple formatted expression shape was prevalidated");
    bytes.extend_from_slice(br#"{"constant":null,"kind":{"kind":"variable","place":{"indices":["#);
    for (index, expression) in place.indices.iter().enumerate() {
        if index != 0 {
            bytes.push(b',');
        }
        append_simple_expression(bytes, expression);
    }
    bytes.extend_from_slice(br#"],"mutable":"#);
    bytes.extend_from_slice(if place.mutable { b"true" } else { b"false" });
    bytes.extend_from_slice(br#","value_type":"#);
    serde_json::to_writer(&mut *bytes, &place.value_type).expect("variable type is serializable");
    bytes.extend_from_slice(br#","variable":"#);
    serde_json::to_writer(&mut *bytes, &place.variable.0).expect("variable ID is serializable");
    bytes.extend_from_slice(br#"}},"value_type":"#);
    serde_json::to_writer(&mut *bytes, &place.value_type).expect("expression type is serializable");
    bytes.push(b'}');
}

fn simple_raw_argument(kind: &HirStatementKind) -> Option<(&'static str, &str, &str)> {
    let HirStatementKind::Instruction { target, arguments } = kind else {
        return None;
    };
    let [HirArgument::Raw(raw)] = arguments.as_slice() else {
        return None;
    };
    let (target_kind, target_name) = match target {
        InstructionTarget::Builtin(name) => ("builtin", name.as_str()),
        InstructionTarget::Extension(name) => ("extension", name.as_str()),
        InstructionTarget::Unresolved(name) => ("unresolved", name.as_str()),
        InstructionTarget::BuiltinMethod { .. } => return None,
    };
    Some((target_kind, target_name, raw))
}

fn simple_raw_argument_fingerprint(target_kind: &str, target_name: &str, raw: &str) -> Digest {
    FINGERPRINT_JSON.with_borrow_mut(|bytes| {
        bytes.clear();
        bytes.extend_from_slice(br#"{"arguments":[{"kind":"raw","value":"#);
        serde_json::to_writer(&mut *bytes, raw).expect("raw argument is serializable");
        bytes.extend_from_slice(br#"}],"kind":"instruction","target":{"kind":"#);
        append_simple_target(bytes, target_kind, target_name);
        fingerprint_digest(bytes)
    })
}

fn append_simple_target(bytes: &mut Vec<u8>, target_kind: &str, target_name: &str) {
    serde_json::to_writer(&mut *bytes, target_kind)
        .expect("instruction target kind is serializable");
    bytes.extend_from_slice(br#","name":"#);
    serde_json::to_writer(&mut *bytes, target_name)
        .expect("instruction target name is serializable");
    bytes.extend_from_slice(br"}}");
}

fn canonical_integer_argument(arguments: &[HirArgument]) -> Option<i64> {
    let [HirArgument::Expression(expression)] = arguments else {
        return None;
    };
    canonical_integer_expression(expression)
}

fn canonical_integer_variable(arguments: &[HirArgument]) -> Option<IntegerVariableKey> {
    let [HirArgument::Expression(expression)] = arguments else {
        return None;
    };
    canonical_integer_variable_expression(expression)
}

fn canonical_integer_variable_expression(expression: &HirExpr) -> Option<IntegerVariableKey> {
    let HirExpr {
        kind:
            HirExprKind::Variable {
                place:
                    HirPlace {
                        variable,
                        indices,
                        value_type: SemanticType::Integer,
                        mutable,
                        ..
                    },
            },
        value_type: SemanticType::Integer,
        constant: None,
        ..
    } = expression
    else {
        return None;
    };
    match indices.as_slice() {
        [] => Some(IntegerVariableKey::Scalar {
            variable: variable.0,
            mutable: *mutable,
        }),
        [index] => Some(IntegerVariableKey::Indexed {
            variable: variable.0,
            mutable: *mutable,
            index: canonical_integer_expression(index)?,
        }),
        _ => None,
    }
}

fn canonical_binary_integer_expression(arguments: &[HirArgument]) -> Option<BinaryIntegerKey> {
    let [
        HirArgument::Expression(HirExpr {
            kind: HirExprKind::Binary { op, left, right },
            value_type: SemanticType::Integer,
            constant: None,
            ..
        }),
    ] = arguments
    else {
        return None;
    };
    if let (Some(variable), Some(integer)) = (
        canonical_integer_variable_expression(left),
        canonical_integer_expression(right),
    ) {
        return Some(BinaryIntegerKey {
            op: *op,
            variable,
            integer,
            variable_on_left: true,
        });
    }
    Some(BinaryIntegerKey {
        op: *op,
        variable: canonical_integer_variable_expression(right)?,
        integer: canonical_integer_expression(left)?,
        variable_on_left: false,
    })
}

fn canonical_integer_expression(expression: &HirExpr) -> Option<i64> {
    let HirExpr {
        kind: HirExprKind::Integer { value },
        value_type: SemanticType::Integer,
        constant: Some(ConstantValue::Integer(constant)),
        ..
    } = expression
    else {
        return None;
    };
    (value == constant).then_some(*value)
}

fn canonical_integer_assignment(kind: &HirStatementKind) -> Option<IntegerAssignmentKey> {
    let HirStatementKind::Assignment {
        target:
            HirPlace {
                variable,
                indices,
                value_type: SemanticType::Integer,
                mutable,
                ..
            },
        op: AssignOp::Assign,
        value,
    } = kind
    else {
        return None;
    };
    let value = canonical_integer_expression(value)?;
    match indices.as_slice() {
        [] => Some(IntegerAssignmentKey::Scalar {
            variable: variable.0,
            mutable: *mutable,
            value,
        }),
        [index] => Some(IntegerAssignmentKey::Indexed {
            variable: variable.0,
            mutable: *mutable,
            index: canonical_integer_expression(index)?,
            value,
        }),
        _ => None,
    }
}

enum EmptyInstructionCache {
    Builtin,
    Extension,
    Unresolved,
}

enum IntegerInstructionCache {
    Builtin,
    Extension,
    Unresolved,
}

enum VariableInstructionCache {
    Builtin,
    Extension,
    Unresolved,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum IntegerVariableKey {
    Scalar {
        variable: u32,
        mutable: bool,
    },
    Indexed {
        variable: u32,
        mutable: bool,
        index: i64,
    },
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct BinaryIntegerKey {
    op: BinaryOp,
    variable: IntegerVariableKey,
    integer: i64,
    variable_on_left: bool,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum IntegerAssignmentKey {
    Scalar {
        variable: u32,
        mutable: bool,
        value: i64,
    },
    Indexed {
        variable: u32,
        mutable: bool,
        index: i64,
        value: i64,
    },
}

#[derive(Default)]
struct FingerprintCache {
    empty_builtin: HashMap<String, Digest>,
    empty_extension: HashMap<String, Digest>,
    empty_unresolved: HashMap<String, Digest>,
    integer_builtin: HashMap<String, HashMap<i64, Digest>>,
    integer_extension: HashMap<String, HashMap<i64, Digest>>,
    integer_unresolved: HashMap<String, HashMap<i64, Digest>>,
    integer_variable_builtin: HashMap<String, HashMap<IntegerVariableKey, Digest>>,
    integer_variable_extension: HashMap<String, HashMap<IntegerVariableKey, Digest>>,
    integer_variable_unresolved: HashMap<String, HashMap<IntegerVariableKey, Digest>>,
    binary_integer_builtin: HashMap<String, HashMap<BinaryIntegerKey, Digest>>,
    binary_integer_extension: HashMap<String, HashMap<BinaryIntegerKey, Digest>>,
    binary_integer_unresolved: HashMap<String, HashMap<BinaryIntegerKey, Digest>>,
    integer_assignments: HashMap<IntegerAssignmentKey, Digest>,
}

fn uncached_statement_fingerprint(kind: &HirStatementKind) -> Digest {
    let mut value = serde_json::to_value(kind).expect("typed statements are serializable");
    // Source locations are deliberately excluded: inserting unrelated lines must
    // not break a breakpoint anchor for an otherwise identical typed statement.
    strip_source_locations(&mut value);
    FINGERPRINT_JSON.with_borrow_mut(|bytes| {
        bytes.clear();
        serde_json::to_writer(&mut *bytes, &value).expect("normalized statements are serializable");
        fingerprint_digest(bytes)
    })
}

fn fingerprint_digest(bytes: &[u8]) -> Digest {
    let mut digest = Digest::hash("rustyera.bytecode.source-statement.v1", &[bytes]);
    // Breakpoint relocation only needs a stable statement identity, not the
    // collision resistance of an artifact identity. Keeping the value in the
    // existing Digest type preserves all public interfaces while allowing the
    // project-file representation to store the meaningful 128 bits only.
    digest.0[16..].fill(0);
    digest
}

thread_local! {
    static FINGERPRINT_JSON: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static FINGERPRINT_CACHE: RefCell<FingerprintCache> = RefCell::new(FingerprintCache::default());
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
