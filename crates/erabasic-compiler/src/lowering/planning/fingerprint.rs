//! Deterministic source identities and structured control-flow precomputation.

use std::{cell::RefCell, collections::HashMap, sync::OnceLock};

use erabasic_hir::{ConstantValue, HirPlace};

use super::super::{
    AssignOp, BinaryOp, Digest, HirArgument, HirExpr, HirExprKind, HirFormPart, HirFormattedString,
    HirStatementKind, InstructionTarget, SemanticType,
};
use super::strip_source_locations;

pub(in crate::lowering) fn statement_fingerprint(kind: &HirStatementKind) -> Digest {
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
