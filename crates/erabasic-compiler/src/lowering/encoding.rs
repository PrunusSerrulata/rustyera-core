//! Stable mappings from semantic operations to bytecode and Host ABI values.

use super::{
    AssignOp, BinaryOp, BytecodeType, PostfixOp, RuntimeImport, SemanticType, SymbolKey, UnaryOp,
};

pub(super) fn compiler_native_contract(pure: bool) -> erabasic_bytecode::OperationContract {
    use erabasic_bytecode::{
        CandidatePolicy, CapabilityFallback, OperationContract, OperationDebugPolicy,
        OperationHotReloadPolicy, OperationPersistence, OperationSnapshotPolicy, OperationState,
        OperationWaitPolicy, TransactionPolicy,
    };
    OperationContract {
        state: if pure {
            OperationState::Pure
        } else {
            OperationState::Vm
        },
        transaction: TransactionPolicy::ReadOnly,
        candidate: CandidatePolicy::ReadOnly,
        persistence: OperationPersistence::None,
        snapshot: OperationSnapshotPolicy::Included,
        hot_reload: OperationHotReloadPolicy::Preserve,
        wait: OperationWaitPolicy::Immediate,
        capability_fallback: CapabilityFallback::NotApplicable,
        debug: OperationDebugPolicy::Pure,
        portability: erabasic_bytecode::OperationPortability::Portable,
    }
}

pub(super) fn runtime_import(
    namespace: &str,
    name: &str,
    abi_version: u32,
    parameters: &[BytecodeType],
    result: Option<BytecodeType>,
) -> RuntimeImport {
    let identity = serde_json::to_vec(&(namespace, name, abi_version, parameters, result))
        .expect("runtime import identity is serializable");
    RuntimeImport {
        key: SymbolKey::derive("rustyera.bytecode.runtime-import.v1", &identity),
        namespace: namespace.into(),
        name: name.into(),
        abi_version,
        parameters: parameters.to_vec(),
        result,
    }
}

pub(crate) fn bytecode_type(value: SemanticType) -> Option<BytecodeType> {
    match value {
        SemanticType::Integer => Some(BytecodeType::Integer),
        SemanticType::String => Some(BytecodeType::String),
        SemanticType::Void | SemanticType::Error => None,
    }
}

pub(super) fn assign_tag(operation: AssignOp) -> u8 {
    match operation {
        AssignOp::Assign => 0,
        AssignOp::Add => 1,
        AssignOp::Subtract => 2,
        AssignOp::Multiply => 3,
        AssignOp::Divide => 4,
        AssignOp::Modulo => 5,
        AssignOp::BitAnd => 6,
        AssignOp::BitOr => 7,
        AssignOp::BitXor => 8,
        AssignOp::ShiftLeft => 9,
        AssignOp::ShiftRight => 10,
    }
}

pub(super) fn unary_tag(operation: UnaryOp) -> u8 {
    match operation {
        UnaryOp::Plus => 0,
        UnaryOp::Minus => 1,
        UnaryOp::LogicalNot => 2,
        UnaryOp::BitNot => 3,
        UnaryOp::PreIncrement => 4,
        UnaryOp::PreDecrement => 5,
    }
}

pub(super) fn postfix_tag(operation: PostfixOp) -> u8 {
    match operation {
        PostfixOp::Increment => 6,
        PostfixOp::Decrement => 7,
    }
}

pub(super) fn binary_tag(operation: BinaryOp) -> u8 {
    match operation {
        BinaryOp::Multiply => 0,
        BinaryOp::Divide => 1,
        BinaryOp::Modulo => 2,
        BinaryOp::Add => 3,
        BinaryOp::Subtract => 4,
        BinaryOp::ShiftLeft => 5,
        BinaryOp::ShiftRight => 6,
        BinaryOp::Less => 7,
        BinaryOp::LessEqual => 8,
        BinaryOp::Greater => 9,
        BinaryOp::GreaterEqual => 10,
        BinaryOp::Equal => 11,
        BinaryOp::NotEqual => 12,
        BinaryOp::BitAnd => 13,
        BinaryOp::BitXor => 14,
        BinaryOp::BitOr => 15,
        BinaryOp::LogicalAnd => 16,
        BinaryOp::LogicalXor => 17,
        BinaryOp::LogicalOr => 18,
        BinaryOp::Nand => 19,
        BinaryOp::Nor => 20,
    }
}
