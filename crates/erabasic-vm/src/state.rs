use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use crate::debug::DebugState;
use crate::regex_compat::RegexCache;
use crate::{
    FiberId, FiberStatus, FrameId, GenerationId, HostReady, HostRequestId, Memory, PlaceDescriptor,
    VariableCell, VariableMap, VmConfig, VmError, VmValue,
};
use crate::{
    PreparedRuntimeState, VmRuntimeFill, VmRuntimeRead, VmRuntimeStatePort,
    VmRuntimeStateTransaction,
};
use erabasic_bytecode::{
    BytecodeArtifact, BytecodeConstant, BytecodeFunction, BytecodeFunctionKind, BytecodeStorage,
    BytecodeType, Digest, ImportKind, Opcode, SourceMapEntry, SymbolKey,
};
use erabasic_validator::ValidatedArtifact;

#[derive(Clone, Debug)]
pub(crate) struct ProgramGeneration {
    pub artifact: Arc<BytecodeArtifact>,
    function_indices: HashMap<SymbolKey, usize>,
    function_name_indices: HashMap<String, usize>,
    // This map is lookup-only; authoritative globals remain canonically ordered
    // in the artifact, so hash iteration can never affect serialized output.
    global_indices: HashMap<SymbolKey, usize>,
    variable_global_indices: Vec<Vec<Option<usize>>>,
    bulk_fill_loop_plans: Vec<Vec<Option<BulkFillLoopPlan>>>,
    literal_group_match_plans: Vec<Vec<Option<LiteralGroupMatchPlan>>>,
    function_memo_plans: Vec<Option<FunctionMemoPlan>>,
    memoized_indexed_read_plans: Vec<Option<MemoizedIndexedReadPlan>>,
    // Canonical owner-free definitions always win system-name lookup.
    global_name_indices: HashMap<String, usize>,
    // Runtime inspection historically exposes otherwise unique function variables.
    runtime_name_fallback_indices: HashMap<String, usize>,
    target_global_index: Option<usize>,
    native_import_indices: HashMap<SymbolKey, usize>,
    host_import_indices: HashMap<SymbolKey, usize>,
    normalized_native_names: Vec<Arc<str>>,
    function_static_indices: BTreeMap<SymbolKey, Vec<usize>>,
    function_local_indices: BTreeMap<SymbolKey, Vec<usize>>,
    instruction_source_indices: BTreeMap<SymbolKey, Vec<u32>>,
}

const NO_SOURCE_MAP_ENTRY: u32 = u32::MAX;

impl ProgramGeneration {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn new(artifact: Arc<BytecodeArtifact>) -> Self {
        // Era projects commonly contain tens of thousands of functions. Resolving the
        // active function with a linear scan for every instruction makes otherwise
        // lightweight EraBasic execution quadratic in the project size.
        let function_indices = artifact
            .functions
            .iter()
            .enumerate()
            .map(|(index, function)| (function.key, index))
            .collect();
        let mut function_name_indices = HashMap::new();
        for (index, function) in artifact.functions.iter().enumerate() {
            // Dynamic lookup follows the artifact order when duplicate declarations
            // are permitted by the selected compatibility mode.
            function_name_indices
                .entry(function.name.to_ascii_uppercase())
                .or_insert(index);
        }
        let global_indices: HashMap<SymbolKey, usize> = artifact
            .globals
            .iter()
            .enumerate()
            .map(|(index, global)| (global.key, index))
            .collect();
        let variable_global_indices: Vec<Vec<Option<usize>>> = artifact
            .functions
            .iter()
            .map(|function| {
                function
                    .code
                    .iter()
                    .map(|instruction| {
                        matches!(
                            Opcode::try_from(instruction.opcode),
                            Ok(Opcode::LoadVariable | Opcode::StoreVariable | Opcode::MakePlace)
                        )
                        .then(|| {
                            instruction
                                .payload
                                .get(..16)?
                                .try_into()
                                .ok()
                                .map(SymbolKey)
                                .and_then(|key| global_indices.get(&key).copied())
                        })
                        .flatten()
                    })
                    .collect()
            })
            .collect();
        let mut runtime_name_fallback_indices = HashMap::new();
        for (index, global) in artifact.globals.iter().enumerate() {
            runtime_name_fallback_indices
                .entry(global.name.to_ascii_uppercase())
                .or_insert(index);
        }
        let mut global_name_indices = HashMap::new();
        for (index, global) in artifact.globals.iter().enumerate() {
            if global.owner.is_none() {
                global_name_indices.insert(global.name.to_ascii_uppercase(), index);
            }
        }
        let target_global_index = global_name_indices.get("TARGET").copied();
        let mut native_import_indices = HashMap::new();
        for (index, import) in artifact.native_imports.iter().enumerate() {
            native_import_indices
                .entry(import.import.key)
                .or_insert(index);
        }
        let normalized_native_names: Vec<Arc<str>> = artifact
            .native_imports
            .iter()
            .map(|import| Arc::<str>::from(import.import.name.to_ascii_lowercase()))
            .collect();
        let mut host_import_indices = HashMap::new();
        for (index, import) in artifact.host_imports.iter().enumerate() {
            host_import_indices
                .entry(import.import.key)
                .or_insert(index);
        }
        let bulk_fill_loop_plans = artifact
            .functions
            .iter()
            .enumerate()
            .map(|(function_index, function)| {
                (0..function.code.len())
                    .map(|instruction| {
                        simple_bulk_fill_loop(
                            &artifact,
                            function_index,
                            instruction,
                            &variable_global_indices,
                        )
                    })
                    .collect()
            })
            .collect();
        let literal_group_match_plans = artifact
            .functions
            .iter()
            .map(|function| {
                (0..function.code.len())
                    .map(|instruction| {
                        literal_group_match(
                            &artifact,
                            function,
                            instruction,
                            &native_import_indices,
                            &normalized_native_names,
                        )
                    })
                    .collect()
            })
            .collect();
        let function_memo_plans = build_function_memo_plans(
            &artifact,
            &variable_global_indices,
            &native_import_indices,
            &host_import_indices,
            &normalized_native_names,
        );
        let memoized_indexed_read_plans = artifact
            .functions
            .iter()
            .enumerate()
            .map(|(function_index, function)| {
                memoized_indexed_read(
                    &artifact,
                    function_index,
                    function,
                    &variable_global_indices,
                    &function_indices,
                    &function_memo_plans,
                )
            })
            .collect();
        let mut function_static_indices = BTreeMap::<SymbolKey, Vec<usize>>::new();
        let mut function_local_indices = BTreeMap::<SymbolKey, Vec<usize>>::new();
        let mut function_names_by_key = BTreeMap::<SymbolKey, String>::new();
        let mut function_keys_by_name = BTreeMap::<String, Vec<SymbolKey>>::new();
        for function in &artifact.functions {
            let normalized = function.name.to_ascii_uppercase();
            function_names_by_key.insert(function.key, normalized.clone());
            function_keys_by_name
                .entry(normalized)
                .or_default()
                .push(function.key);
        }
        for (index, global) in artifact.globals.iter().enumerate() {
            if global.storage == BytecodeStorage::FunctionStatic
                && let Some(owner) = global.owner
            {
                function_static_indices
                    .entry(owner)
                    .or_default()
                    .push(index);
            } else if global.storage == BytecodeStorage::FunctionPersistent
                && let Some(owner) = global.owner
                && let Some(owner_name) = function_names_by_key.get(&owner)
                && let Some(function_keys) = function_keys_by_name.get(owner_name)
            {
                // LOCAL/LOCALS/ARG/ARGS persist per normalized Era function name.
                // Duplicate event handlers therefore share these cells even though a
                // serialized global can name only one function key as its owner.
                for function in function_keys {
                    function_static_indices
                        .entry(*function)
                        .or_default()
                        .push(index);
                }
            } else if global.storage == BytecodeStorage::FunctionLocal
                && let Some(owner) = global.owner
            {
                function_local_indices.entry(owner).or_default().push(index);
            }
        }
        // Resolve serialized source-map precedence once per generation. Filling only empty
        // instruction slots preserves `SourceMap::resolve`'s first-matching-entry behavior.
        // A u32 sentinel is sufficient for the validator's source-map limit and is one quarter
        // the size of `Option<usize>` on 64-bit targets. Offsets are built one function at a time
        // so startup does not retain another project-wide index beside the permanent projection.
        let mut source_entries = BTreeMap::<SymbolKey, Vec<(u32, &SourceMapEntry)>>::new();
        for (index, entry) in artifact.source_map.entries.iter().enumerate() {
            source_entries.entry(entry.function).or_default().push((
                u32::try_from(index).expect("validated source-map index fits u32"),
                entry,
            ));
        }
        let instruction_source_indices = artifact
            .functions
            .iter()
            .map(|function| {
                let mut offset = 0_u64;
                let offsets = function
                    .code
                    .iter()
                    .map(|instruction| {
                        let current = offset;
                        offset = offset.saturating_add(instruction.encoded_len());
                        current
                    })
                    .collect::<Vec<_>>();
                let indices = index_source_entries(
                    &offsets,
                    source_entries.get(&function.key).map_or(&[], Vec::as_slice),
                );
                (function.key, indices)
            })
            .collect();
        Self {
            artifact,
            function_indices,
            function_name_indices,
            global_indices,
            variable_global_indices,
            bulk_fill_loop_plans,
            literal_group_match_plans,
            function_memo_plans,
            memoized_indexed_read_plans,
            global_name_indices,
            runtime_name_fallback_indices,
            target_global_index,
            native_import_indices,
            host_import_indices,
            normalized_native_names,
            function_static_indices,
            function_local_indices,
            instruction_source_indices,
        }
    }

    pub(crate) fn function(&self, key: SymbolKey) -> Option<&BytecodeFunction> {
        self.function_index(key)
            .and_then(|index| self.artifact.functions.get(*index))
    }

    pub(crate) fn function_index(&self, key: SymbolKey) -> Option<&usize> {
        self.function_indices.get(&key)
    }

    pub(crate) fn function_by_name(&self, name: &str) -> Option<&BytecodeFunction> {
        case_insensitive_index(&self.function_name_indices, name)
            .and_then(|index| self.artifact.functions.get(*index))
    }

    pub(crate) fn global(&self, key: SymbolKey) -> Option<&erabasic_bytecode::BytecodeGlobal> {
        self.global_index(key)
            .and_then(|index| self.artifact.globals.get(*index))
    }

    pub(crate) fn global_index(&self, key: SymbolKey) -> Option<&usize> {
        self.global_indices.get(&key)
    }

    pub(crate) fn instruction_global(
        &self,
        function_index: usize,
        instruction: usize,
    ) -> Option<&erabasic_bytecode::BytecodeGlobal> {
        self.variable_global_indices
            .get(function_index)?
            .get(instruction)
            .copied()
            .flatten()
            .and_then(|index| self.artifact.globals.get(index))
    }

    pub(crate) fn function_memo_plan(&self, function: SymbolKey) -> Option<&FunctionMemoPlan> {
        let index = *self.function_index(function)?;
        self.function_memo_plans.get(index)?.as_ref()
    }

    pub(crate) fn memoized_indexed_read_plan(
        &self,
        function: SymbolKey,
    ) -> Option<&MemoizedIndexedReadPlan> {
        let index = *self.function_index(function)?;
        self.memoized_indexed_read_plans.get(index)?.as_ref()
    }

    pub(crate) fn bulk_fill_loop_plan(
        &self,
        function: SymbolKey,
        instruction: usize,
    ) -> Option<&BulkFillLoopPlan> {
        let index = *self.function_index(function)?;
        self.bulk_fill_loop_plans
            .get(index)?
            .get(instruction)?
            .as_ref()
    }

    pub(crate) fn literal_group_match_plan(
        &self,
        function: SymbolKey,
        instruction: usize,
    ) -> Option<&LiteralGroupMatchPlan> {
        let index = *self.function_index(function)?;
        self.literal_group_match_plans
            .get(index)?
            .get(instruction)?
            .as_ref()
    }

    pub(crate) fn global_by_name(&self, name: &str) -> Option<&erabasic_bytecode::BytecodeGlobal> {
        case_insensitive_index(&self.global_name_indices, name)
            .or_else(|| case_insensitive_index(&self.runtime_name_fallback_indices, name))
            .and_then(|index| self.artifact.globals.get(*index))
    }

    pub(crate) fn scoped_variable(
        &self,
        function: SymbolKey,
        name: &str,
    ) -> Option<&erabasic_bytecode::BytecodeGlobal> {
        self.function_locals(function)
            .chain(self.function_statics(function))
            .chain(
                self.artifact
                    .globals
                    .iter()
                    .filter(|global| global.owner.is_none()),
            )
            .find(|global| global.name.eq_ignore_ascii_case(name))
    }

    pub(crate) fn target_global(&self) -> Option<&erabasic_bytecode::BytecodeGlobal> {
        self.target_global_index
            .and_then(|index| self.artifact.globals.get(index))
    }

    pub(crate) fn native_import_index(&self, key: SymbolKey) -> Option<usize> {
        self.native_import_indices.get(&key).copied()
    }

    pub(crate) fn host_import_index(&self, key: SymbolKey) -> Option<usize> {
        self.host_import_indices.get(&key).copied()
    }

    pub(crate) fn normalized_native_name(&self, index: usize) -> Option<Arc<str>> {
        self.normalized_native_names.get(index).cloned()
    }

    pub(crate) fn function_statics(
        &self,
        function: SymbolKey,
    ) -> impl Iterator<Item = &erabasic_bytecode::BytecodeGlobal> {
        self.function_static_indices
            .get(&function)
            .into_iter()
            .flatten()
            .filter_map(|index| self.artifact.globals.get(*index))
    }

    pub(crate) fn function_locals(
        &self,
        function: SymbolKey,
    ) -> impl Iterator<Item = &erabasic_bytecode::BytecodeGlobal> {
        self.function_local_indices
            .get(&function)
            .into_iter()
            .flatten()
            .filter_map(|index| self.artifact.globals.get(*index))
    }

    pub(crate) fn source_location(
        &self,
        function: SymbolKey,
        instruction: usize,
    ) -> Option<erabasic_bytecode::ResolvedSourceLocation> {
        let entry = self
            .instruction_source_indices
            .get(&function)?
            .get(instruction)
            .copied()
            .filter(|index| *index != NO_SOURCE_MAP_ENTRY)
            .and_then(|index| self.artifact.source_map.entries.get(index as usize))?;
        self.artifact.source_map.resolve_entry(entry)
    }
}

fn simple_bulk_fill_loop(
    artifact: &BytecodeArtifact,
    function_index: usize,
    instruction: usize,
    variable_global_indices: &[Vec<Option<usize>>],
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
    let prefix_index = globals.get(instruction + 2).copied().flatten()?;
    let counter_index = globals.get(instruction + 3).copied().flatten()?;
    let target_index = globals.get(instruction + 5).copied().flatten()?;
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

fn literal_group_match(
    artifact: &BytecodeArtifact,
    function: &BytecodeFunction,
    instruction: usize,
    native_import_indices: &HashMap<SymbolKey, usize>,
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

fn memoized_indexed_read(
    artifact: &BytecodeArtifact,
    function_index: usize,
    function: &BytecodeFunction,
    variable_global_indices: &[Vec<Option<usize>>],
    function_indices: &HashMap<SymbolKey, usize>,
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
    let index_global = variables[tail]?;
    let scratch_global = variables[tail + 1]?;
    let target_global = variables[tail + 2]?;
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
        let selector_global = variables[call_index - 1]?;
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

fn build_function_memo_plans(
    artifact: &BytecodeArtifact,
    variable_global_indices: &[Vec<Option<usize>>],
    native_import_indices: &HashMap<SymbolKey, usize>,
    host_import_indices: &HashMap<SymbolKey, usize>,
    normalized_native_names: &[Arc<str>],
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
    variable_global_indices: &[Vec<Option<usize>>],
    native_import_indices: &HashMap<SymbolKey, usize>,
    host_import_indices: &HashMap<SymbolKey, usize>,
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
                .then_some(variables[instruction]?)
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
                let index = variables[instruction_index]?;
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
                let index = variables[instruction_index]?;
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
            | Opcode::ResolveFunction
            | Opcode::InvokeDynamic
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
    variables: &[Option<usize>],
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
            let variable = variables.get(instruction_index).copied().flatten();
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

fn index_source_entries(offsets: &[u64], entries: &[(u32, &SourceMapEntry)]) -> Vec<u32> {
    let mut indices = vec![NO_SOURCE_MAP_ENTRY; offsets.len()];
    for (index, entry) in entries {
        let start = offsets.partition_point(|offset| *offset < entry.code_start);
        let end = offsets.partition_point(|offset| *offset < entry.code_end);
        for slot in &mut indices[start..end] {
            if *slot == NO_SOURCE_MAP_ENTRY {
                *slot = *index;
            }
        }
    }
    indices
}

fn case_insensitive_index<'a>(
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
        let indices = index_source_entries(&[0, 2, 5, 7, 10], &[(3, &broad), (4, &overlapping)]);

        assert_eq!(indices, [NO_SOURCE_MAP_ENTRY, 3, 3, 3, NO_SOURCE_MAP_ENTRY]);
        assert_eq!(std::mem::size_of_val(&indices[0]), 4);
    }
}

mod frames;
mod lifecycle;
mod places;
mod runtime;
mod runtime_types;

pub use runtime_types::Vm;
pub(crate) use runtime_types::{
    BulkFillLoopPlan, EventDispatch, EventDispatchEntry, Fiber, FiberState, FindElementCacheKey,
    FindElementNeedle, ForLoopState, Frame, FunctionMemoEntry, FunctionMemoKey, FunctionMemoPlan,
    LiteralGroupMatchPlan, MemoValue, MemoizedIndexedReadPlan, WaitingHost,
};

pub(crate) use frames::{
    bind_persistent_arguments, make_frame, prepare_dynamic_arguments, validate_arguments,
};
use frames::{find_frame, find_frame_mut, find_global};
use runtime::replace_cell_values;
