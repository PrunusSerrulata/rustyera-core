#[allow(clippy::wildcard_imports)]
use super::*;

/// Preflight every read and bound before committing a bounded, nonaliasing copy.
/// Active path traces use ordinary reads/writes so their dependencies remain complete.
pub(super) fn copy_range(
    vm: &mut Vm,
    fiber: &Fiber,
    generation: crate::GenerationId,
    program: &crate::state::ProgramGeneration,
    target: &erabasic_bytecode::BytecodeGlobal,
    copy: (SymbolKey, i64),
    range: (i64, i64, i64),
) -> bool {
    if vm.path_memo_is_active_for(fiber.id) {
        return false;
    }
    let (prefix, start, end) = range;
    let Some(length) = end.checked_sub(start).and_then(|n| usize::try_from(n).ok()) else {
        return false;
    };
    // Bound temporary storage without heap allocation or materializing sparse arrays.
    let mut values = [0_i64; 256];
    if length == 0 || length > values.len() {
        return false;
    }
    let Some(source) = program.global(copy.0) else {
        return false;
    };
    let Some((source_character, source_start, source_end)) =
        bulk_fill_target_range(source, prefix, start, end)
    else {
        return false;
    };
    let Some((target_start, target_end)) = start.checked_add(copy.1).zip(end.checked_add(copy.1))
    else {
        return false;
    };
    let Some((target_character, target_start, target_end)) =
        bulk_fill_target_range(target, prefix, target_start, target_end)
    else {
        return false;
    };
    let source_character = source_character.unwrap_or(0);
    let target_character = target_character.unwrap_or(0);
    if vm
        .validate_script_character(source.storage, source_character)
        .is_err()
        || vm
            .validate_script_character(target.storage, target_character)
            .is_err()
    {
        return false;
    }
    let Some(source_cell) = vm.memory.cell(generation, source, source_character) else {
        return false;
    };
    if source_cell.dimensions != source.dimensions
        || source_end > source_cell.len()
        || source_cell.value_type != BytecodeType::Integer
    {
        return false;
    }
    for (offset, value) in values[..length].iter_mut().enumerate() {
        let Some(VmValue::Integer(integer)) = source_cell.get(source_start + offset) else {
            return false;
        };
        *value = integer;
    }
    let Some(target_cell) = vm.memory.cell(generation, target, target_character) else {
        return false;
    };
    if target_cell.dimensions != target.dimensions
        || target_end > target_cell.len()
        || target_cell.value_type != BytecodeType::Integer
    {
        return false;
    }
    let target_cell = vm
        .memory
        .cell_mut(generation, target.key, target.storage, target_character)
        .expect("preflighted target remains available");
    for (offset, value) in values[..length].iter().enumerate() {
        // Same per-cell revision behavior as scalar StoreVariable, including sparse zeros.
        target_cell
            .set(target_start + offset, VmValue::Integer(*value))
            .expect("preflighted integer target accepts each copied element");
    }
    true
}
