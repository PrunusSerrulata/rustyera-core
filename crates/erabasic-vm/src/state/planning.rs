#[allow(clippy::wildcard_imports)]
use super::*;

pub(super) fn structured_scope_ranges(function: &BytecodeFunction) -> Vec<StructuredScopeRange> {
    let mut open = Vec::<(StructuredScopeKind, usize)>::new();
    let mut ranges = Vec::new();
    for (instruction, encoded) in function.code.iter().enumerate() {
        let Ok(opcode) = Opcode::try_from(encoded.opcode) else {
            continue;
        };
        let kind = match opcode {
            Opcode::ForStart => {
                open.push((StructuredScopeKind::Loop, instruction));
                continue;
            }
            Opcode::SelectStart => {
                open.push((StructuredScopeKind::Select, instruction));
                continue;
            }
            Opcode::ForNext => StructuredScopeKind::Loop,
            Opcode::SelectEnd => StructuredScopeKind::Select,
            _ => continue,
        };
        let Some(position) = open.iter().rposition(|(candidate, _)| *candidate == kind) else {
            continue;
        };
        let (_, opener) = open.remove(position);
        let end = if kind == StructuredScopeKind::Loop
            && function
                .code
                .get(instruction + 1)
                .and_then(|encoded| Opcode::try_from(encoded.opcode).ok())
                == Some(Opcode::Unary)
            && function
                .code
                .get(instruction + 2)
                .and_then(|encoded| Opcode::try_from(encoded.opcode).ok())
                == Some(Opcode::JumpIfFalse)
        {
            instruction + 2
        } else {
            instruction
        };
        ranges.push(StructuredScopeRange {
            kind,
            opener,
            start: opener + 1,
            end,
        });
    }
    ranges.sort_by_key(|range| (range.start, std::cmp::Reverse(range.end)));
    ranges
}

pub(super) fn simple_bulk_fill_loop(
    artifact: &BytecodeArtifact,
    function_index: usize,
    instruction: usize,
    variable_global_indices: &[Vec<u32>],
) -> Option<BulkFillLoopPlan> {
    let function = artifact.functions.get(function_index)?;
    let code = function
        .code
        .get(instruction..instruction.checked_add(9)?)?;
    let opcodes = code
        .iter()
        .map(|encoded| Opcode::try_from(encoded.opcode).ok())
        .collect::<Option<Vec<_>>>()?;
    if opcodes
        != [
            Opcode::ForStart,
            Opcode::JumpIfFalse,
            Opcode::LoadVariable,
            Opcode::LoadVariable,
            Opcode::PushInteger,
            Opcode::StoreVariable,
            Opcode::ForNext,
            Opcode::Unary,
            Opcode::JumpIfFalse,
        ]
        || read_payload_u32(&code[1].payload, 0)? as usize != instruction + 9
        || read_payload_u32(&code[8].payload, 0)? as usize != instruction + 2
        || code[7].payload.as_ref() != [2]
        || read_payload_u16(&code[2].payload, 16)? != 0
        || read_payload_u16(&code[3].payload, 16)? != 0
        || read_payload_u16(&code[5].payload, 16)? != 2
        || code[5].payload.get(18).copied()? != 0
    {
        return None;
    }
    let globals = variable_global_indices.get(function_index)?;
    let prefix_index = compact_global_index(globals, instruction + 2)?;
    let counter_index = compact_global_index(globals, instruction + 3)?;
    let target_index = compact_global_index(globals, instruction + 5)?;
    let prefix = artifact.globals.get(prefix_index)?;
    let counter = artifact.globals.get(counter_index)?;
    let target = artifact.globals.get(target_index)?;
    if prefix.value_type != BytecodeType::Integer
        || prefix.storage == BytecodeStorage::Character
        || counter.value_type != BytecodeType::Integer
        || target.storage != BytecodeStorage::Project
        || target.value_type != BytecodeType::Integer
        || target.dimensions.len() != 2
        || !target.mutable
    {
        return None;
    }
    let value = i64::from_le_bytes(code[4].payload.as_ref().try_into().ok()?);
    Some(BulkFillLoopPlan {
        prefix: prefix.key,
        counter: counter.key,
        target: target.key,
        value: VmValue::Integer(value),
        after_loop: instruction + 9,
    })
}

pub(super) fn literal_group_match(
    artifact: &BytecodeArtifact,
    function: &BytecodeFunction,
    instruction: usize,
    native_import_indices: &SymbolMap<usize>,
    normalized_native_names: &[Arc<str>],
) -> Option<LiteralGroupMatchPlan> {
    let mut candidates = Vec::new();
    let mut cursor = instruction;
    while let Some(encoded) = function.code.get(cursor)
        && Opcode::try_from(encoded.opcode).ok()? == Opcode::PushString
    {
        let length = read_payload_u32(&encoded.payload, 0)? as usize;
        let bytes = encoded.payload.get(4..4 + length)?;
        if encoded.payload.len() != 4 + length {
            return None;
        }
        candidates.push(Arc::<str>::from(std::str::from_utf8(bytes).ok()?));
        cursor += 1;
    }
    if candidates.is_empty() {
        return None;
    }
    let call = function.code.get(cursor)?;
    if Opcode::try_from(call.opcode).ok()? != Opcode::CallNative
        || usize::from(read_payload_u16(&call.payload, 4)?) != candidates.len() + 1
    {
        return None;
    }
    let import_index = read_payload_u32(&call.payload, 0)? as usize;
    let import = function.imports.get(import_index)?;
    if import.kind != ImportKind::Native {
        return None;
    }
    let native_index = *native_import_indices.get(&import.key)?;
    if normalized_native_names.get(native_index)?.as_ref() != "groupmatch"
        || artifact.native_imports.get(native_index)?.import.result != Some(BytecodeType::Integer)
    {
        return None;
    }
    Some(LiteralGroupMatchPlan {
        candidates,
        after_call: cursor + 1,
    })
}

pub(super) fn memoized_indexed_read(
    artifact: &BytecodeArtifact,
    function_index: usize,
    function: &BytecodeFunction,
    variable_global_indices: &[Vec<u32>],
    function_indices: &SymbolMap<usize>,
    function_memo_plans: &[Option<FunctionMemoPlan>],
) -> Option<MemoizedIndexedReadPlan> {
    let code = &function.code;
    let tail = code.len().checked_sub(4)?;
    let variables = variable_global_indices.get(function_index)?;
    let index_load = code.get(tail)?;
    let selector_load = code.get(tail + 1)?;
    let target_load = code.get(tail + 2)?;
    let return_instruction = code.get(tail + 3)?;
    if Opcode::try_from(index_load.opcode).ok()? != Opcode::LoadVariable
        || read_payload_u16(&index_load.payload, 16)? != 0
        || Opcode::try_from(selector_load.opcode).ok()? != Opcode::LoadVariable
        || read_payload_u16(&selector_load.payload, 16)? != 0
        || Opcode::try_from(target_load.opcode).ok()? != Opcode::LoadVariable
        || read_payload_u16(&target_load.payload, 16)? != 2
        || Opcode::try_from(return_instruction.opcode).ok()? != Opcode::Return
        || return_instruction.payload.first().copied() != Some(1)
    {
        return None;
    }
    let index_global = compact_global_index(variables, tail)?;
    let scratch_global = compact_global_index(variables, tail + 1)?;
    let target_global = compact_global_index(variables, tail + 2)?;
    let index_parameter = function
        .parameters
        .iter()
        .position(|parameter| parameter.key == artifact.globals[index_global].key)?;
    let scratch = &artifact.globals[scratch_global];
    let target = &artifact.globals[target_global];
    if scratch.value_type != BytecodeType::Integer
        || !scratch.mutable
        || !matches!(
            scratch.storage,
            BytecodeStorage::FunctionPersistent | BytecodeStorage::FunctionStatic
        )
        || scratch.owner != Some(function.key)
        || target.storage != BytecodeStorage::Project
        || target.dimensions.len() != 2
        || function.result != Some(target.value_type)
    {
        return None;
    }
    for (call_index, call) in code[..tail].iter().enumerate() {
        if Opcode::try_from(call.opcode).ok()? != Opcode::Call
            || read_payload_u16(&call.payload, 4)? != 2
            || call_index < 2
        {
            continue;
        }
        let prefix = &code[call_index - 2];
        let selector = &code[call_index - 1];
        if Opcode::try_from(prefix.opcode).ok()? != Opcode::PushString
            || Opcode::try_from(selector.opcode).ok()? != Opcode::LoadVariable
            || read_payload_u16(&selector.payload, 16)? != 0
        {
            continue;
        }
        let selector_global = compact_global_index(variables, call_index - 1)?;
        let Some(selector_parameter) = function
            .parameters
            .iter()
            .position(|parameter| parameter.key == artifact.globals[selector_global].key)
        else {
            continue;
        };
        let import_index = read_payload_u32(&call.payload, 0)? as usize;
        let import = function.imports.get(import_index)?;
        if import.kind != ImportKind::Function {
            continue;
        }
        let selector_function_index = *function_indices.get(&import.key)?;
        if artifact.functions.get(selector_function_index)?.result != Some(BytecodeType::Integer)
            || function_memo_plans.get(selector_function_index)?.is_none()
        {
            continue;
        }
        if artifact.manifest.compatibility.integer_arithmetic_policy()
            == erabasic_compat::IntegerArithmeticPolicy::SnakeSaturatingV1
            && !has_exact_indexed_getter_prefix(code, variables, tail, call_index, scratch_global)
        {
            // A warm selector does not prove that skipped getter instructions are safe.
            // Unknown prefixes must execute so their diagnostics and faults remain visible.
            continue;
        }
        let length = read_payload_u32(&prefix.payload, 0)? as usize;
        let bytes = prefix.payload.get(4..4 + length)?;
        if prefix.payload.len() != 4 + length {
            continue;
        }
        return Some(MemoizedIndexedReadPlan {
            index_parameter,
            selector_parameter,
            selector_function: import.key,
            selector_prefix: std::str::from_utf8(bytes).ok()?.to_owned(),
            scratch: scratch.key,
            target: target.key,
        });
    }
    None
}

fn has_exact_indexed_getter_prefix(
    code: &[erabasic_bytecode::EncodedInstruction],
    variables: &[u32],
    tail: usize,
    call_index: usize,
    scratch_global: usize,
) -> bool {
    let mut prefix = code[..tail]
        .iter()
        .enumerate()
        .filter(|(_, instruction)| Opcode::try_from(instruction.opcode).ok() != Some(Opcode::Nop));
    let Some((_, text)) = prefix.next() else {
        return false;
    };
    let Some((_, selector)) = prefix.next() else {
        return false;
    };
    let Some((call, _)) = prefix.next() else {
        return false;
    };
    let Some((store_index, store)) = prefix.next() else {
        return false;
    };
    // The entire prefix is: literal prefix, parameter, memoized selector call,
    // plain scratch assignment. The tail's loads and return were checked above.
    prefix.next().is_none()
        && Opcode::try_from(text.opcode).ok() == Some(Opcode::PushString)
        && Opcode::try_from(selector.opcode).ok() == Some(Opcode::LoadVariable)
        && call == call_index
        && Opcode::try_from(store.opcode).ok() == Some(Opcode::StoreVariable)
        && read_payload_u16(&store.payload, 16) == Some(0)
        && store.payload.get(18).copied() == Some(0)
        && compact_global_index(variables, store_index) == Some(scratch_global)
}

pub(super) fn path_memo_result_reads(
    artifact: &BytecodeArtifact,
    function_index: usize,
    function: &BytecodeFunction,
    variable_global_indices: &[Vec<u32>],
) -> Vec<PathMemoResultReadPlan> {
    // A value return may live inside a branch rather than at the lexical end of the function.
    // Runtime tracing later confirms that a candidate pair was actually executed contiguously.
    function
        .code
        .windows(2)
        .enumerate()
        .filter_map(|(instruction, pair)| {
            let [load, returned] = pair else {
                return None;
            };
            if Opcode::try_from(load.opcode).ok()? != Opcode::LoadVariable
                || Opcode::try_from(returned.opcode).ok()? != Opcode::Return
                || returned.payload.first().copied() != Some(1)
            {
                return None;
            }
            let global_index =
                compact_global_index(variable_global_indices.get(function_index)?, instruction)?;
            let variable = artifact.globals.get(global_index)?;
            if !matches!(
                variable.storage,
                BytecodeStorage::Project
                    | BytecodeStorage::Constant
                    | BytecodeStorage::FunctionStatic
                    | BytecodeStorage::FunctionPersistent
            ) || function.result != Some(variable.value_type)
            {
                return None;
            }
            Some(PathMemoResultReadPlan {
                instruction,
                variable: variable.key,
            })
        })
        .collect()
}

pub(super) fn build_function_memo_plans(
    artifact: &BytecodeArtifact,
    variable_global_indices: &[Vec<u32>],
    native_import_indices: &SymbolMap<usize>,
    host_import_indices: &SymbolMap<usize>,
    normalized_native_names: &[Arc<str>],
    mut advance: impl FnMut(),
) -> Vec<Option<FunctionMemoPlan>> {
    artifact
        .functions
        .iter()
        .enumerate()
        .map(|(function_index, function)| {
            analyze_function_memo(
                artifact,
                variable_global_indices,
                native_import_indices,
                host_import_indices,
                normalized_native_names,
                function_index,
                function,
            )
        })
        .map(|candidate| {
            advance();
            candidate.map(|candidate| FunctionMemoPlan {
                dependency_indices: candidate.dependencies.into_iter().collect(),
                scratch_indices: candidate.replay.into_iter().collect(),
            })
        })
        .collect()
}

#[derive(Clone, Debug)]
struct FunctionMemoAnalysis {
    dependencies: BTreeSet<usize>,
    replay: BTreeSet<usize>,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn analyze_function_memo(
    artifact: &BytecodeArtifact,
    variable_global_indices: &[Vec<u32>],
    native_import_indices: &SymbolMap<usize>,
    host_import_indices: &SymbolMap<usize>,
    normalized_native_names: &[Arc<str>],
    function_index: usize,
    function: &BytecodeFunction,
) -> Option<FunctionMemoAnalysis> {
    if !matches!(
        function.result,
        Some(BytecodeType::Integer | BytecodeType::String)
    ) || function.parameters.iter().any(|parameter| {
        parameter.by_reference
            || matches!(
                parameter.value_type,
                BytecodeType::IntegerPlace | BytecodeType::StringPlace
            )
    }) {
        return None;
    }
    let variables = variable_global_indices.get(function_index)?;
    let scratch = function
        .code
        .iter()
        .enumerate()
        .filter_map(|(instruction, encoded)| {
            (Opcode::try_from(encoded.opcode).ok()? == Opcode::StoreVariable)
                .then(|| compact_global_index(variables, instruction))
                .flatten()
        })
        .filter(|index| {
            let definition = &artifact.globals[*index];
            definition.storage != BytecodeStorage::FunctionLocal
                && !function
                    .parameters
                    .iter()
                    .any(|parameter| parameter.key == definition.key)
        })
        .collect::<BTreeSet<_>>();
    if scratch.iter().any(|index| {
        let definition = &artifact.globals[*index];
        !matches!(
            definition.storage,
            BytecodeStorage::FunctionPersistent | BytecodeStorage::FunctionStatic
        ) || definition.owner != Some(function.key)
            || !definition.mutable
    }) || !scratch_is_definitely_initialized(function, variables, &scratch)
    {
        return None;
    }
    let replay = scratch.clone();
    let mut dependencies = BTreeSet::new();
    for (instruction_index, instruction) in function.code.iter().enumerate() {
        let opcode = Opcode::try_from(instruction.opcode).ok()?;
        match opcode {
            Opcode::LoadVariable | Opcode::MakePlace => {
                let index = compact_global_index(variables, instruction_index)?;
                let definition = &artifact.globals[index];
                if function
                    .parameters
                    .iter()
                    .any(|parameter| parameter.key == definition.key)
                    || scratch.contains(&index)
                {
                    continue;
                }
                match definition.storage {
                    BytecodeStorage::FunctionLocal => {}
                    BytecodeStorage::Character => return None,
                    _ => {
                        dependencies.insert(index);
                    }
                }
            }
            Opcode::StoreVariable => {
                let index = compact_global_index(variables, instruction_index)?;
                if scratch.contains(&index) {
                    if read_payload_u16(&instruction.payload, 16)? != 0 {
                        return None;
                    }
                } else if artifact.globals[index].storage != BytecodeStorage::FunctionLocal {
                    return None;
                }
            }
            Opcode::CallNative => {
                let import = function
                    .imports
                    .get(read_payload_u32(&instruction.payload, 0)? as usize)?;
                let name = normalized_native_names.get(*native_import_indices.get(&import.key)?)?;
                if !matches!(
                    name.as_ref(),
                    "escape"
                        | "findelement"
                        | "findlastelement"
                        | "format_integer"
                        | "format_string"
                        | "isnumeric"
                        | "toint"
                ) {
                    return None;
                }
            }
            Opcode::CallHost => {
                let import = function
                    .imports
                    .get(read_payload_u32(&instruction.payload, 0)? as usize)?;
                let host_index = *host_import_indices.get(&import.key)?;
                if !artifact.host_imports[host_index]
                    .import
                    .name
                    .eq_ignore_ascii_case("THROW")
                {
                    return None;
                }
            }
            Opcode::ForStart | Opcode::ForNext | Opcode::ForBreak if !scratch.is_empty() => {
                return None;
            }
            Opcode::StorePlace
            | Opcode::Call
            | Opcode::ResolveUserCall
            | Opcode::SelectUserArgument
            | Opcode::CaptureUserArgument
            | Opcode::InvokeUserCall
            | Opcode::GuardUserArgument
            | Opcode::AdvanceUserArgument
            | Opcode::AbandonUserCall
            | Opcode::JumpDynamicLabel
            | Opcode::InvokeEvent
            | Opcode::Yield
            | Opcode::AwaitResume
            | Opcode::Trap => return None,
            Opcode::Nop
            | Opcode::PushInteger
            | Opcode::PushString
            | Opcode::Unary
            | Opcode::Binary
            | Opcode::ToString
            | Opcode::Concat
            | Opcode::Pop
            | Opcode::Dup
            | Opcode::Jump
            | Opcode::JumpIfFalse
            | Opcode::ForStart
            | Opcode::ForNext
            | Opcode::SelectStart
            | Opcode::SelectCompare
            | Opcode::SelectEnd
            | Opcode::ForBreak
            | Opcode::Return => {}
        }
    }
    Some(FunctionMemoAnalysis {
        dependencies,
        replay,
    })
}

fn scratch_is_definitely_initialized(
    function: &BytecodeFunction,
    variables: &[u32],
    scratch: &BTreeSet<usize>,
) -> bool {
    scratch.iter().all(|scratch| {
        let mut pending = vec![(0_usize, false)];
        let mut visited = BTreeSet::new();
        while let Some((instruction_index, mut initialized)) = pending.pop() {
            if !visited.insert((instruction_index, initialized)) {
                continue;
            }
            let Some(instruction) = function.code.get(instruction_index) else {
                return false;
            };
            let Ok(opcode) = Opcode::try_from(instruction.opcode) else {
                return false;
            };
            let variable = compact_global_index(variables, instruction_index);
            if variable == Some(*scratch) {
                if opcode == Opcode::LoadVariable && !initialized || opcode == Opcode::MakePlace {
                    return false;
                }
                if opcode == Opcode::StoreVariable {
                    initialized = true;
                }
            }
            match opcode {
                Opcode::Return => {
                    if !initialized {
                        return false;
                    }
                }
                Opcode::Jump => {
                    let Some(target) = read_payload_u32(&instruction.payload, 0) else {
                        return false;
                    };
                    pending.push((target as usize, initialized));
                }
                Opcode::JumpIfFalse => {
                    let Some(target) = read_payload_u32(&instruction.payload, 0) else {
                        return false;
                    };
                    pending.push((target as usize, initialized));
                    pending.push((instruction_index + 1, initialized));
                }
                _ => pending.push((instruction_index + 1, initialized)),
            }
        }
        true
    })
}

fn read_payload_u16(payload: &[u8], offset: usize) -> Option<u16> {
    payload
        .get(offset..offset + 2)?
        .try_into()
        .ok()
        .map(u16::from_le_bytes)
}

fn read_payload_u32(payload: &[u8], offset: usize) -> Option<u32> {
    payload
        .get(offset..offset + 4)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
}

fn compact_global_index(indices: &[u32], instruction: usize) -> Option<usize> {
    indices
        .get(instruction)
        .copied()
        .filter(|index| *index != NO_GLOBAL_INDEX)
        .map(|index| index as usize)
}

pub(super) fn index_source_entries<'a>(
    offsets: &[u64],
    entries: impl IntoIterator<Item = (u32, &'a SourceMapEntry)>,
) -> Vec<u32> {
    let mut indices = vec![NO_SOURCE_MAP_ENTRY; offsets.len()];
    for (index, entry) in entries {
        let start = offsets.partition_point(|offset| *offset < entry.code_start);
        let end = offsets.partition_point(|offset| *offset < entry.code_end);
        for slot in &mut indices[start..end] {
            if *slot == NO_SOURCE_MAP_ENTRY {
                *slot = index;
            }
        }
    }
    indices
}

pub(super) fn case_insensitive_index<'a>(
    indices: &'a HashMap<String, usize>,
    name: &str,
) -> Option<&'a usize> {
    if name.bytes().any(|byte| byte.is_ascii_lowercase()) {
        indices.get(&name.to_ascii_uppercase())
    } else {
        indices.get(name)
    }
}

#[cfg(test)]
mod program_index_tests {
    use super::*;

    #[test]
    fn name_index_preserves_ascii_case_insensitive_lookup() {
        let indices = HashMap::from([("MIXED_NAME".to_owned(), 7)]);
        assert_eq!(case_insensitive_index(&indices, "MIXED_NAME"), Some(&7));
        assert_eq!(case_insensitive_index(&indices, "mixed_name"), Some(&7));
        assert_eq!(case_insensitive_index(&indices, "Mixed_Name"), Some(&7));
        assert_eq!(case_insensitive_index(&indices, "missing"), None);
    }

    #[test]
    fn compact_global_index_rejects_missing_and_sentinel_slots() {
        assert_eq!(compact_global_index(&[4, NO_GLOBAL_INDEX, 9], 0), Some(4));
        assert_eq!(compact_global_index(&[4, NO_GLOBAL_INDEX, 9], 1), None);
        assert_eq!(compact_global_index(&[4, NO_GLOBAL_INDEX, 9], 3), None);
    }

    #[test]
    fn compact_source_index_preserves_first_matching_entry() {
        let function = SymbolKey::derive("source-index-test", b"function");
        let broad = SourceMapEntry {
            function,
            code_start: 2,
            code_end: 10,
            byte_start: 1,
            byte_end: 2,
            statement_fingerprint: 0,
            origin_chain: None,
            source_index: 0,
        };
        let overlapping = SourceMapEntry {
            code_start: 5,
            code_end: 7,
            ..broad.clone()
        };
        let indices = index_source_entries(&[0, 2, 5, 7, 10], [(3, &broad), (4, &overlapping)]);

        assert_eq!(indices, [NO_SOURCE_MAP_ENTRY, 3, 3, 3, NO_SOURCE_MAP_ENTRY]);
        assert_eq!(std::mem::size_of_val(&indices[0]), 4);
    }
}
