#[allow(clippy::wildcard_imports)]
use super::*;

/// Recognize a pure constant fill with invariant prefix and literal-indexed counter reads.
/// Every unmatched form retains ordinary bytecode execution.
pub(in crate::state) fn simple_bulk_fill_loop(
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
    let (counter_index, counter_indices) = constant_indexed_read(function, globals, &mut cursor)?;
    let value_instruction = code.get(cursor)?;
    if value_instruction.opcode != Opcode::PushInteger as u16 {
        return None;
    }
    let value = i64::from_le_bytes(value_instruction.payload.as_ref().try_into().ok()?);
    let store_index = cursor.checked_add(1)?;
    let store = code.get(store_index)?;
    if store.opcode != Opcode::StoreVariable as u16
        || read_payload_u16(&store.payload, 16)? != 2
        || store.payload.get(18).copied()? != 0
    {
        return None;
    }
    let next = store_index.checked_add(1)?;
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
    let target = artifact
        .globals
        .get(compact_global_index(globals, store_index)?)?;
    if !supports_fill(
        prefix,
        counter,
        target,
        &prefix_indices,
        &counter_indices,
        reference_keys,
        value,
    ) {
        return None;
    }
    let stack_peak = 3.max(prefix_indices.len()).max(1 + counter_indices.len());
    Some(BulkFillLoopPlan {
        prefix: prefix.key,
        prefix_indices,
        counter: counter.key,
        counter_indices,
        target: target.key,
        operation: BulkArrayOperation::Fill(VmValue::Integer(value)),
        after_loop,
        iteration_instructions: u64::try_from(after_loop.checked_sub(body_start)?).ok()?,
        stack_peak,
    })
}

pub(super) fn supports_fill(
    prefix: &BytecodeGlobal,
    counter: &BytecodeGlobal,
    target: &BytecodeGlobal,
    prefix_indices: &[u64],
    counter_indices: &[u64],
    reference_keys: &HashSet<SymbolKey, BuildHasherDefault<SymbolKeyHasher>>,
    value: i64,
) -> bool {
    if prefix.value_type != BytecodeType::Integer
        || prefix.storage == BytecodeStorage::Character
        || counter.value_type != BytecodeType::Integer
        || counter.storage == BytecodeStorage::Character
        || !counter.mutable
        || target.value_type != BytecodeType::Integer
        || !target.mutable
        || !matches!(
            (target.storage, target.dimensions.len()),
            (BytecodeStorage::Project, 2) | (BytecodeStorage::Character, 1)
        )
        || [prefix.key, counter.key, target.key]
            .iter()
            .any(|key| reference_keys.contains(key))
        || target.key == prefix.key
        || target.key == counter.key
        || !valid_constant_indices(&prefix.dimensions, prefix_indices)
        || !valid_constant_indices(&counter.dimensions, counter_indices)
    {
        return false;
    }
    // Omitted indices denote zero. Distinct cells of LOCAL may be used as prefix/counter,
    // but the loop must not mutate its own prefix through another spelling of that cell.
    if prefix.key == counter.key
        && (0..prefix.dimensions.len()).all(|index| {
            prefix_indices.get(index).copied().unwrap_or(0)
                == counter_indices.get(index).copied().unwrap_or(0)
        })
    {
        return false;
    }
    // Keep the expanded character/indexed-counter path allocation-free while filling sparse
    // storage. The established scalar-counter project path also supports nonzero constants.
    if value != 0
        && (target.storage == BytecodeStorage::Character
            || !prefix_indices.is_empty()
            || !counter_indices.is_empty())
    {
        return false;
    }
    true
}

pub(super) fn constant_indexed_read(
    function: &BytecodeFunction,
    globals: &[u32],
    cursor: &mut usize,
) -> Option<(usize, Vec<u64>)> {
    let mut indices = Vec::new();
    while function.code.get(*cursor)?.opcode == Opcode::PushInteger as u16 {
        if indices.len() == 4 {
            return None;
        }
        let value = i64::from_le_bytes(function.code[*cursor].payload.as_ref().try_into().ok()?);
        indices.push(u64::try_from(value).ok()?);
        *cursor = cursor.checked_add(1)?;
    }
    let encoded = function.code.get(*cursor)?;
    if encoded.opcode != Opcode::LoadVariable as u16
        || usize::from(read_payload_u16(&encoded.payload, 16)?) != indices.len()
    {
        return None;
    }
    let global = compact_global_index(globals, *cursor)?;
    *cursor = cursor.checked_add(1)?;
    Some((global, indices))
}

fn valid_constant_indices(dimensions: &[u64], indices: &[u64]) -> bool {
    indices.len() <= dimensions.len()
        && dimensions
            .iter()
            .enumerate()
            .all(|(index, length)| indices.get(index).copied().unwrap_or(0) < *length)
}
