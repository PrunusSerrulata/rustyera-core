//! Continuation roots are justified by the bytecode that created them.

use crate::interpreter::dynamic_form::RuntimeFormContinuation;
use crate::state::Frame;
use erabasic_bytecode::{
    BytecodeArtifact, BytecodeFunction, BytecodeType, ImportKind, Opcode,
};

pub(super) fn valid_origin(
    frame: &Frame,
    function: &BytecodeFunction,
    artifact: &BytecodeArtifact,
    continuation: &RuntimeFormContinuation,
) -> bool {
    let (generation, function_key, origin) = continuation.origin();
    if generation != frame.generation
        || function_key != frame.function
        || frame.instruction != origin.saturating_add(1)
    {
        return false;
    }
    let Some(instruction) = function.code.get(origin) else {
        return false;
    };
    if Opcode::try_from(instruction.opcode) != Ok(Opcode::CallNative) {
        return false;
    }
    let Some(encoded_index) = instruction.payload.get(..4) else {
        return false;
    };
    let mut bytes = [0; 4];
    bytes.copy_from_slice(encoded_index);
    let Some(import) = function
        .imports
        .get(u32::from_le_bytes(bytes) as usize)
        .filter(|import| import.kind == ImportKind::Native)
    else {
        return false;
    };
    artifact.native_imports.iter().any(|native| {
        native.import.key == import.key
            && native.import.name.eq_ignore_ascii_case("STRFORM")
            && native.import.parameters == [BytecodeType::String]
            && native.import.result == Some(BytecodeType::String)
    })
}
