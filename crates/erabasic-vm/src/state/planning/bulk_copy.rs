use super::bulk_fill::{constant_indexed_read, supports_fill};
#[allow(clippy::wildcard_imports)]
use super::*;

/// Match a contiguous copy between distinct integer arrays, with one invariant row/character.
/// Reference aliases, side effects and other loop shapes retain ordinary execution.
pub(in crate::state) fn simple_bulk_copy_loop(
    artifact: &BytecodeArtifact,
    function_index: usize,
    instruction: usize,
    variable_global_indices: &[Vec<u32>],
    reference_keys: &HashSet<SymbolKey, BuildHasherDefault<SymbolKeyHasher>>,
) -> Option<BulkFillLoopPlan> {
    let function = artifact.functions.get(function_index)?;
    let code = &function.code;
    if code.get(instruction)?.opcode != Opcode::ForStart as u16
        || code.get(instruction.checked_add(1)?)?.opcode != Opcode::JumpIfFalse as u16
    {
        return None;
    }
    let globals = variable_global_indices.get(function_index)?;
    let body_start = instruction.checked_add(2)?;
    let mut cursor = body_start;
    let (prefix_index, prefix_indices) = constant_indexed_read(function, globals, &mut cursor)?;
    // Restrict the first version to literal + counter, as emitted by the existing compiler.
    let offset = code.get(cursor)?;
    if offset.opcode != Opcode::PushInteger as u16 {
        return None;
    }
    let target_offset = i64::from_le_bytes(offset.payload.as_ref().try_into().ok()?);
    cursor += 1;
    let (counter_index, counter_indices) = constant_indexed_read(function, globals, &mut cursor)?;
    if code.get(cursor)?.opcode != Opcode::Binary as u16 || code[cursor].payload.as_ref() != [3] {
        return None;
    }
    cursor += 1;
    let source_prefix = constant_indexed_read(function, globals, &mut cursor)?;
    let source_counter = constant_indexed_read(function, globals, &mut cursor)?;
    if source_prefix.0 != prefix_index
        || source_prefix.1.as_slice() != prefix_indices.as_slice()
        || source_counter.0 != counter_index
        || source_counter.1.as_slice() != counter_indices.as_slice()
    {
        return None;
    }
    let load = code.get(cursor)?;
    let source = artifact
        .globals
        .get(compact_global_index(globals, cursor)?)?;
    if load.opcode != Opcode::LoadVariable as u16 || read_payload_u16(&load.payload, 16)? != 2 {
        return None;
    }
    cursor += 1;
    let store = code.get(cursor)?;
    let target = artifact
        .globals
        .get(compact_global_index(globals, cursor)?)?;
    if store.opcode != Opcode::StoreVariable as u16
        || read_payload_u16(&store.payload, 16)? != 2
        || store.payload.get(18).copied()? != 0
    {
        return None;
    }
    let next = cursor.checked_add(1)?;
    let after_loop = next.checked_add(3)?;
    if code.get(next)?.opcode != Opcode::ForNext as u16
        || code.get(next + 1)?.opcode != Opcode::Unary as u16
        || code.get(next + 1)?.payload.as_ref() != [2]
        || code.get(next + 2)?.opcode != Opcode::JumpIfFalse as u16
        || read_payload_u32(&code[instruction + 1].payload, 0)? as usize != after_loop
        || read_payload_u32(&code[next + 2].payload, 0)? as usize != body_start
    {
        return None;
    }
    let prefix = artifact.globals.get(prefix_index)?;
    let counter = artifact.globals.get(counter_index)?;
    if !supports_fill(
        prefix,
        counter,
        target,
        &prefix_indices,
        &counter_indices,
        reference_keys,
        0,
    ) || source.value_type != BytecodeType::Integer
        || !matches!(
            (source.storage, source.dimensions.len()),
            (BytecodeStorage::Project, 2) | (BytecodeStorage::Character, 1)
        )
        || reference_keys.contains(&source.key)
        || [target.key, counter.key, prefix.key].contains(&source.key)
    {
        return None;
    }
    let stack_peak = 4
        .max(2 + prefix_indices.len())
        .max(3 + counter_indices.len());
    Some(BulkFillLoopPlan {
        prefix: prefix.key,
        prefix_indices,
        counter: counter.key,
        counter_indices,
        target: target.key,
        operation: BulkArrayOperation::Copy {
            source: source.key,
            target_offset,
        },
        after_loop,
        iteration_instructions: u64::try_from(after_loop.checked_sub(body_start)?).ok()?,
        stack_peak,
    })
}
