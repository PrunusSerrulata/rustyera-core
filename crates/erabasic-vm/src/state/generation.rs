#[allow(clippy::wildcard_imports)]
use super::*;
impl ProgramGeneration {
    #[cfg(test)]
    pub(crate) fn disable_user_call_specs_for_test(&mut self) {
        self.decoded_user_call_specs =
            user_call_specs::DecodedUserCallSpecs::with_limits(&[], 0, 0, 0, || {});
    }

    #[cfg(test)]
    pub(crate) fn disable_bulk_fill_for_test(&mut self) {
        self.bulk_fill_loop_plans.clear();
    }

    #[cfg(test)]
    pub(crate) fn disable_literal_select_for_test(&mut self) {
        self.literal_select_plans.clear();
    }

    pub(crate) fn runtime_variable(
        &self,
        key: SymbolKey,
    ) -> Option<&erabasic_bytecode::RuntimeVariableSymbol> {
        self.artifact
            .runtime_variables
            .binary_search_by_key(&key, |symbol| symbol.key)
            .ok()
            .map(|index| &self.artifact.runtime_variables[index])
    }

    pub(crate) fn is_reference_variable(&self, key: SymbolKey) -> bool {
        self.reference_variable_keys.contains(&key)
    }

    pub(crate) fn effective_character_disposal(
        &self,
        key: SymbolKey,
    ) -> Option<erabasic_bytecode::CharacterArrayDisposal> {
        self.runtime_variable(key).map(|metadata| {
            if self.artifact.manifest.compatibility.profile
                == erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake
            {
                metadata.character_disposal
            } else {
                erabasic_bytecode::CharacterArrayDisposal::Preserve
            }
        })
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn new(artifact: Arc<BytecodeArtifact>) -> Self {
        Self::new_with_progress(artifact, None)
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn new_with_progress(
        artifact: Arc<BytecodeArtifact>,
        progress: Option<&mut dyn FnMut(VmPreparationProgress)>,
    ) -> Self {
        let function_work = u64::try_from(artifact.functions.len()).unwrap_or(u64::MAX);
        let global_work = u64::try_from(artifact.globals.len()).unwrap_or(u64::MAX);
        let total_work = function_work
            .saturating_mul(11)
            .saturating_add(global_work.saturating_mul(4))
            .max(1);
        let mut preparation = PreparationReporter::new(progress, total_work);
        // Era projects commonly contain tens of thousands of functions. Resolving the
        // active function with a linear scan for every instruction makes otherwise
        // lightweight EraBasic execution quadratic in the project size.
        let mut function_indices = SymbolMap::with_capacity_and_hasher(
            artifact.functions.len(),
            BuildHasherDefault::default(),
        );
        for (index, function) in artifact.functions.iter().enumerate() {
            function_indices.insert(function.key, index);
            preparation.advance();
        }
        let mut function_name_indices = HashMap::new();
        for (index, function) in artifact.functions.iter().enumerate() {
            // Dynamic lookup follows the artifact order when duplicate declarations
            // are permitted by the selected compatibility mode.
            function_name_indices
                .entry(function.name.to_ascii_uppercase())
                .or_insert(index);
            preparation.advance();
        }
        let mut global_indices = SymbolMap::with_capacity_and_hasher(
            artifact.globals.len(),
            BuildHasherDefault::default(),
        );
        for (index, global) in artifact.globals.iter().enumerate() {
            global_indices.insert(global.key, index);
            preparation.advance();
        }
        let mut variable_global_indices = Vec::with_capacity(artifact.functions.len());
        // Only REF definitions participate. Ordinary accesses no longer search the complete
        // runtime-symbol table, while the artifact remains the serialized source of truth.
        let reference_variable_keys = artifact
            .runtime_variables
            .iter()
            .filter(|symbol| symbol.reference)
            .map(|symbol| symbol.key)
            .collect();
        for function in &artifact.functions {
            variable_global_indices.push(
                function
                    .code
                    .iter()
                    .map(|instruction| {
                        if matches!(
                            Opcode::try_from(instruction.opcode),
                            Ok(Opcode::LoadVariable | Opcode::StoreVariable | Opcode::MakePlace)
                        ) {
                            instruction
                                .payload
                                .get(..16)
                                .and_then(|bytes| bytes.try_into().ok())
                                .map(SymbolKey)
                                .and_then(|key| global_indices.get(&key).copied())
                                .and_then(|index| u32::try_from(index).ok())
                                .unwrap_or(NO_GLOBAL_INDEX)
                        } else {
                            NO_GLOBAL_INDEX
                        }
                    })
                    .collect(),
            );
            preparation.advance();
        }
        let mut runtime_name_fallback_indices = HashMap::new();
        for (index, global) in artifact.globals.iter().enumerate() {
            runtime_name_fallback_indices
                .entry(global.name.to_ascii_uppercase())
                .or_insert(index);
            preparation.advance();
        }
        let mut global_name_indices = HashMap::new();
        let mut first_global_name_indices = HashMap::new();
        let mut owned_name_indices = SymbolMap::<HashMap<String, usize>>::default();
        for (index, global) in artifact.globals.iter().enumerate() {
            if let Some(owner) = global.owner {
                owned_name_indices
                    .entry(owner)
                    .or_default()
                    .entry(global.name.to_ascii_uppercase())
                    .or_insert(index);
            } else {
                let name = global.name.to_ascii_uppercase();
                first_global_name_indices
                    .entry(name.clone())
                    .or_insert(index);
                global_name_indices.insert(name, index);
            }
            preparation.advance();
        }
        let target_global_index = global_name_indices.get("TARGET").copied();
        let mut native_import_indices = SymbolMap::default();
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
        let mut host_import_indices = SymbolMap::default();
        for (index, import) in artifact.host_imports.iter().enumerate() {
            host_import_indices
                .entry(import.import.key)
                .or_insert(index);
        }
        let normalized_host_names = artifact
            .host_imports
            .iter()
            .map(|import| Arc::<str>::from(import.import.name.to_ascii_uppercase()))
            .collect();
        let mut bulk_fill_loop_plans = Vec::with_capacity(artifact.functions.len());
        for (function_index, function) in artifact.functions.iter().enumerate() {
            bulk_fill_loop_plans.push(
                (0..function.code.len())
                    .filter_map(|instruction| {
                        simple_bulk_fill_loop(
                            &artifact,
                            function_index,
                            instruction,
                            &variable_global_indices,
                            &reference_variable_keys,
                        )
                        .or_else(|| {
                            simple_bulk_copy_loop(
                                &artifact,
                                function_index,
                                instruction,
                                &variable_global_indices,
                                &reference_variable_keys,
                            )
                        })
                        .zip(u32::try_from(instruction).ok())
                        .map(|(plan, index)| (index, plan))
                    })
                    .collect(),
            );
            preparation.advance();
        }
        let mut literal_group_match_plans = Vec::with_capacity(artifact.functions.len());
        for function in &artifact.functions {
            literal_group_match_plans.push(
                (0..function.code.len())
                    .filter_map(|instruction| {
                        literal_group_match(
                            &artifact,
                            function,
                            instruction,
                            &native_import_indices,
                            &normalized_native_names,
                        )
                        .and_then(|(first, plan)| {
                            u32::try_from(first).ok().map(|index| (index, plan))
                        })
                    })
                    .collect(),
            );
            preparation.advance();
        }
        let mut structured_ranges = Vec::with_capacity(artifact.functions.len());
        let mut select_budget = planning::literal_select::SelectPlanBudget::default();
        let cache_selects = select_budget.reserve_directory(artifact.functions.len());
        let mut literal_select_plans = Vec::new();
        let mut remaining_jump_bytes = scope_transitions::MAXIMUM_STATIC_JUMP_BYTES;
        let mut remaining_jump_work = scope_transitions::MAXIMUM_STATIC_JUMP_WORK;
        let mut static_structured_jumps = scope_transitions::allocate_directory(
            artifact.functions.len(),
            &mut remaining_jump_bytes,
        );
        let cache_jumps = static_structured_jumps.capacity() >= artifact.functions.len();
        for function in &artifact.functions {
            let ranges = structured_scope_ranges(function);
            if cache_selects {
                literal_select_plans.push(planning::literal_select::plans(
                    function,
                    &ranges,
                    &mut select_budget,
                ));
            }
            if cache_jumps {
                static_structured_jumps.push(scope_transitions::plan_static_jumps(
                    function,
                    &ranges,
                    &mut remaining_jump_bytes,
                    &mut remaining_jump_work,
                ));
            }
            structured_ranges.push(ranges);
            preparation.advance();
        }
        let function_memo_plans = build_function_memo_plans(
            &artifact,
            &variable_global_indices,
            &native_import_indices,
            &host_import_indices,
            &normalized_native_names,
            || {
                preparation.advance();
            },
        );
        let mut memoized_indexed_read_plans = Vec::with_capacity(artifact.functions.len());
        let mut path_memo_result_read_plans = Vec::with_capacity(artifact.functions.len());
        for (function_index, function) in artifact.functions.iter().enumerate() {
            memoized_indexed_read_plans.push(memoized_indexed_read(
                &artifact,
                function_index,
                function,
                &variable_global_indices,
                &function_indices,
                &function_memo_plans,
            ));
            path_memo_result_read_plans.push(path_memo_result_reads(
                &artifact,
                function_index,
                function,
                &variable_global_indices,
            ));
            preparation.advance();
        }
        let mut function_static_indices = SymbolMap::<Vec<usize>>::default();
        let mut function_local_indices = SymbolMap::<Vec<usize>>::default();
        let mut function_names_by_key = BTreeMap::<SymbolKey, String>::new();
        let mut function_keys_by_name = BTreeMap::<String, Vec<SymbolKey>>::new();
        for function in &artifact.functions {
            let normalized = function.name.to_ascii_uppercase();
            function_names_by_key.insert(function.key, normalized.clone());
            function_keys_by_name
                .entry(normalized)
                .or_default()
                .push(function.key);
            preparation.advance();
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
            preparation.advance();
        }
        drop(function_names_by_key);
        drop(function_keys_by_name);
        // Validated artifacts store source entries in canonical function order. Consume each
        // contiguous function range directly so startup never retains a project-wide vector of
        // entry references beside the permanent projection. Filling only empty instruction slots
        // preserves `SourceMap::resolve`'s first match. A u32 sentinel is sufficient for the
        // validator's source-map limit and is one quarter the size of `Option<usize>` on 64-bit.
        let source_entries = &artifact.source_map.entries;
        let mut source_cursor = 0;
        let mut instruction_source_indices = Vec::with_capacity(artifact.functions.len());
        for function in &artifact.functions {
            let source_start = source_cursor;
            while source_cursor < source_entries.len()
                && source_entries[source_cursor].function == function.key
            {
                source_cursor += 1;
            }
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
            instruction_source_indices.push(index_source_entries(
                &offsets,
                source_entries[source_start..source_cursor]
                    .iter()
                    .enumerate()
                    .map(|(offset, entry)| {
                        (
                            u32::try_from(source_start + offset)
                                .expect("validated source-map index fits u32"),
                            entry,
                        )
                    }),
            ));
            preparation.advance();
        }
        debug_assert_eq!(source_cursor, source_entries.len());
        let decoded_user_call_specs =
            user_call_specs::DecodedUserCallSpecs::new(&artifact.functions, || {
                preparation.advance();
            });
        preparation.finish();
        Self {
            artifact,
            function_indices,
            function_name_indices,
            global_indices,
            reference_variable_keys,
            variable_global_indices,
            decoded_user_call_specs,
            bulk_fill_loop_plans,
            literal_group_match_plans,
            literal_select_plans,
            function_memo_plans,
            memoized_indexed_read_plans,
            path_memo_result_read_plans,
            global_name_indices,
            first_global_name_indices,
            owned_name_indices,
            runtime_name_fallback_indices,
            target_global_index,
            native_import_indices,
            host_import_indices,
            normalized_native_names,
            normalized_host_names,
            function_static_indices,
            function_local_indices,
            instruction_source_indices,
            structured_scope_ranges: structured_ranges,
            static_structured_jumps,
        }
    }

    pub(crate) fn function(&self, key: SymbolKey) -> Option<&BytecodeFunction> {
        self.function_index(key)
            .and_then(|index| self.artifact.functions.get(*index))
    }

    pub(crate) fn function_index(&self, key: SymbolKey) -> Option<&usize> {
        self.function_indices.get(&key)
    }

    pub(crate) fn cached_user_call_spec(
        &self,
        function: SymbolKey,
        instruction: usize,
    ) -> Option<&erabasic_bytecode::UserCallSpec> {
        self.decoded_user_call_specs
            .get(*self.function_index(function)?, instruction)
    }

    pub(crate) fn function_by_name(&self, name: &str) -> Option<&BytecodeFunction> {
        case_insensitive_index(&self.function_name_indices, name)
            .and_then(|index| self.artifact.functions.get(*index))
    }

    pub(crate) fn structured_jump_transition(
        &self,
        function: SymbolKey,
        source: usize,
        target: usize,
    ) -> Option<std::borrow::Cow<'_, StructuredJumpTransition>> {
        let index = *self.function_index(function)?;
        self.structured_jump_transition_at_index(index, source, target)
    }

    pub(crate) fn structured_jump_transition_at_index(
        &self,
        index: usize,
        source: usize,
        target: usize,
    ) -> Option<std::borrow::Cow<'_, StructuredJumpTransition>> {
        if let Some(plan) = self
            .static_structured_jumps
            .get(index)
            .and_then(|plans| sparse_instruction_plan(plans, source))
            && plan.target == target
        {
            return Some(std::borrow::Cow::Borrowed(&plan.transition));
        }
        Some(std::borrow::Cow::Owned(scope_transitions::transition(
            self.structured_scope_ranges.get(index)?,
            source,
            target,
        )))
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
            .filter(|index| *index != NO_GLOBAL_INDEX)
            .map(|index| index as usize)
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

    pub(crate) fn path_memo_result_read_plan(
        &self,
        function: SymbolKey,
        instruction: usize,
    ) -> Option<&PathMemoResultReadPlan> {
        let index = *self.function_index(function)?;
        self.path_memo_result_read_plans
            .get(index)?
            .iter()
            .find(|plan| plan.instruction == instruction)
    }

    pub(crate) fn bulk_fill_loop_plan(
        &self,
        function: SymbolKey,
        instruction: usize,
    ) -> Option<&BulkFillLoopPlan> {
        let index = *self.function_index(function)?;
        sparse_instruction_plan(self.bulk_fill_loop_plans.get(index)?, instruction)
    }

    pub(crate) fn literal_group_match_plan(
        &self,
        index: usize,
        instruction: usize,
    ) -> Option<&LiteralGroupMatchPlan> {
        sparse_instruction_plan(self.literal_group_match_plans.get(index)?, instruction)
    }

    pub(crate) fn literal_select_plan(
        &self,
        index: usize,
        instruction: usize,
    ) -> Option<&LiteralSelectPlan> {
        sparse_instruction_plan(self.literal_select_plans.get(index)?, instruction)
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
            .find(|global| global.name.eq_ignore_ascii_case(name))
            .or_else(|| {
                // Scoped resolution uses the first ownerless declaration, unlike the
                // last-wins runtime global lookup and its unrelated-scope fallback.
                case_insensitive_index(&self.first_global_name_indices, name)
                    .and_then(|index| self.artifact.globals.get(*index))
            })
    }

    /// Dynamic variable references use exact owner first, in artifact declaration order.
    /// Do not substitute `scoped_variable`: shared persistent event locals have different owners.
    pub(crate) fn dynamic_variable(
        &self,
        function: SymbolKey,
        name: &str,
    ) -> Option<&erabasic_bytecode::BytecodeGlobal> {
        self.owned_name_indices
            .get(&function)
            .and_then(|names| case_insensitive_index(names, name))
            .or_else(|| case_insensitive_index(&self.first_global_name_indices, name))
            .and_then(|index| self.artifact.globals.get(*index))
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

    pub(crate) fn normalized_native_name(&self, index: usize) -> Option<&str> {
        self.normalized_native_names.get(index).map(AsRef::as_ref)
    }

    pub(crate) fn normalized_host_name(&self, index: usize) -> Option<&str> {
        self.normalized_host_names.get(index).map(AsRef::as_ref)
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
        let function_index = *self.function_index(function)?;
        let entry = self
            .instruction_source_indices
            .get(function_index)?
            .get(instruction)
            .copied()
            .filter(|index| *index != NO_SOURCE_MAP_ENTRY)
            .and_then(|index| self.artifact.source_map.entries.get(index as usize))?;
        self.artifact.source_map.resolve_entry(entry)
    }
}

pub(super) fn report_vm_preparation(
    progress: &mut Option<&mut dyn FnMut(VmPreparationProgress)>,
    stage: VmPreparationStage,
    completed: u64,
    total: u64,
) {
    if let Some(progress) = progress {
        progress(VmPreparationProgress {
            stage,
            completed,
            total,
        });
    }
}

struct PreparationReporter<'a> {
    progress: Option<&'a mut dyn FnMut(VmPreparationProgress)>,
    completed: u64,
    total: u64,
    next_checkpoint: u64,
}

impl<'a> PreparationReporter<'a> {
    fn new(mut progress: Option<&'a mut dyn FnMut(VmPreparationProgress)>, total: u64) -> Self {
        report_vm_preparation(&mut progress, VmPreparationStage::IndexingProgram, 0, total);
        Self {
            progress,
            completed: 0,
            total,
            next_checkpoint: 1,
        }
    }

    fn advance(&mut self) {
        self.completed = self.completed.saturating_add(1).min(self.total);
        let checkpoint = self.completed.saturating_mul(100) / self.total.max(1);
        if checkpoint >= self.next_checkpoint || self.completed == self.total {
            report_vm_preparation(
                &mut self.progress,
                VmPreparationStage::IndexingProgram,
                self.completed,
                self.total,
            );
            self.next_checkpoint = checkpoint.saturating_add(1);
        }
    }

    fn finish(&mut self) {
        report_vm_preparation(
            &mut self.progress,
            VmPreparationStage::IndexingProgram,
            self.total,
            self.total,
        );
    }
}

fn sparse_instruction_plan<T>(plans: &[(u32, T)], instruction: usize) -> Option<&T> {
    let instruction = u32::try_from(instruction).ok()?;
    let index = plans
        .binary_search_by_key(&instruction, |(instruction, _)| *instruction)
        .ok()?;
    Some(&plans[index].1)
}

#[cfg(test)]
mod compact_generation_index_tests {
    use super::*;

    #[test]
    fn decoded_user_call_specs_preserve_origin_limits_and_generation_identity() {
        let artifact = Arc::new(compiled_generation_source(
            "@SYSTEM_TITLE\nTRYCALLFORM TARGET(1, 2)\nRETURN\n@TARGET(ARG, ARG:1)\nRETURN\n",
        ));
        let (function_index, function) = artifact
            .functions
            .iter()
            .enumerate()
            .find(|(_, function)| function.name == "SYSTEM_TITLE")
            .unwrap();
        let resolve = function
            .code
            .iter()
            .position(|instruction| instruction.opcode == Opcode::ResolveUserCall as u16)
            .unwrap();
        let expected =
            erabasic_bytecode::UserCallSpec::decode(&function.code[resolve].payload).unwrap();
        assert_eq!(expected.arguments.len(), 2);
        let generation = ProgramGeneration::new(Arc::clone(&artifact));
        let cached = generation
            .cached_user_call_spec(function.key, resolve)
            .unwrap();
        assert_eq!(cached, &expected);
        assert!(std::ptr::eq(
            cached,
            generation
                .cached_user_call_spec(function.key, resolve)
                .unwrap()
        ));
        assert!(
            generation
                .cached_user_call_spec(function.key, resolve + 1)
                .is_none()
        );
        assert!(
            generation
                .cached_user_call_spec(function.key, usize::MAX)
                .is_none()
        );
        assert!(
            generation
                .cached_user_call_spec(SymbolKey([0xff; 16]), resolve)
                .is_none()
        );

        let no_calls = user_call_specs::DecodedUserCallSpecs::with_limits(
            &artifact.functions,
            2,
            0,
            10,
            || {},
        );
        assert!(no_calls.get(function_index, resolve).is_none());
        let no_arguments = user_call_specs::DecodedUserCallSpecs::with_limits(
            &artifact.functions,
            2,
            10,
            1,
            || {},
        );
        assert!(no_arguments.get(function_index, resolve).is_none());
        let exact =
            user_call_specs::DecodedUserCallSpecs::with_limits(&artifact.functions, 2, 1, 2, || {});
        assert_eq!(exact.get(function_index, resolve), Some(&expected));

        let mut changed = (*artifact).clone();
        let mut changed_spec = expected.clone();
        changed_spec.arguments.clear();
        changed.functions[function_index].code[resolve] =
            erabasic_bytecode::opcode::resolve_user_call(&changed_spec);
        let next = ProgramGeneration::new(Arc::new(changed));
        assert_eq!(
            next.cached_user_call_spec(function.key, resolve),
            Some(&changed_spec)
        );
        assert_eq!(
            generation.cached_user_call_spec(function.key, resolve),
            Some(&expected)
        );

        let mut malformed = (*artifact).clone();
        malformed.functions[function_index].code[resolve].payload = vec![0xff].into();
        let malformed = ProgramGeneration::new(Arc::new(malformed));
        assert!(
            malformed
                .cached_user_call_spec(function.key, resolve)
                .is_none()
        );
    }

    #[test]
    fn literal_groupmatch_planning_uses_call_suffix_once_and_ignores_other_calls() {
        let literals = std::iter::repeat_n("7", 2048)
            .collect::<Vec<_>>()
            .join(", ");
        let artifact = Arc::new(compiled_generation_source(&format!(
            "@SYSTEM_TITLE\nRESULT = GROUPMATCH(7, {literals})\nRESULT:1 = MAX({literals})\nRETURN\n"
        )));
        let generation = ProgramGeneration::new(artifact);
        let plans = &generation.literal_group_match_plans[0];
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].1.candidates.len(), 2048);
        assert_eq!(
            plans[0].1.candidates.match_count(&VmValue::Integer(7)),
            Some(2048)
        );
        assert_eq!(plans[0].1.after_call - plans[0].0 as usize, 2049);
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "Shared cache accounting is checked across directory, entries, payloads and work exhaustion."
    )]
    fn static_structured_jumps_charge_all_capacity_and_bound_generation_work() {
        let artifact = Arc::new(compiled_generation_fixture());
        let generation = ProgramGeneration::new(Arc::clone(&artifact));
        let directory_bytes =
            artifact.functions.len() * std::mem::size_of::<Vec<(u32, StaticStructuredJump)>>();
        let entry_bytes = std::mem::size_of::<(u32, StaticStructuredJump)>();
        for limit in [
            0,
            directory_bytes - 1,
            directory_bytes,
            directory_bytes + entry_bytes,
            directory_bytes + 5 * entry_bytes,
        ] {
            let mut remaining = limit;
            let mut work = usize::MAX;
            let mut directory =
                scope_transitions::allocate_directory(artifact.functions.len(), &mut remaining);
            if directory.capacity() >= artifact.functions.len() {
                for (index, function) in artifact.functions.iter().enumerate() {
                    directory.push(scope_transitions::plan_static_jumps(
                        function,
                        &generation.structured_scope_ranges[index],
                        &mut remaining,
                        &mut work,
                    ));
                }
            }
            let retained = directory.capacity()
                * std::mem::size_of::<Vec<(u32, StaticStructuredJump)>>()
                + directory
                    .iter()
                    .map(|plans| {
                        plans.capacity() * entry_bytes
                            + plans
                                .iter()
                                .map(|(_, plan)| {
                                    plan.transition.entered.capacity()
                                        * std::mem::size_of::<StructuredScopeKind>()
                                })
                                .sum::<usize>()
                    })
                    .sum::<usize>();
            assert_eq!(retained + remaining, limit);
            let mut bounded = generation.clone();
            bounded.static_structured_jumps = directory;
            for (index, function) in artifact.functions.iter().enumerate() {
                for source in 0..function.code.len() {
                    for target in 0..=function.code.len() {
                        assert_eq!(
                            *bounded
                                .structured_jump_transition(function.key, source, target)
                                .unwrap(),
                            scope_transitions::transition(
                                &generation.structured_scope_ranges[index],
                                source,
                                target
                            )
                        );
                    }
                }
            }
        }
        let index = generation
            .structured_scope_ranges
            .iter()
            .position(|ranges| !ranges.is_empty())
            .unwrap();
        let ranges = &generation.structured_scope_ranges[index];
        let mut bytes = scope_transitions::MAXIMUM_STATIC_JUMP_BYTES;
        let mut work = ranges.len() * 5;
        let plans = scope_transitions::plan_static_jumps(
            &artifact.functions[index],
            ranges,
            &mut bytes,
            &mut work,
        );
        assert_eq!(plans.len(), 1);
        assert_eq!(work, 0);
        assert!(
            scope_transitions::plan_static_jumps(
                &artifact.functions[index],
                ranges,
                &mut bytes,
                &mut work
            )
            .is_empty()
        );
        let mut bounded = generation.clone();
        bounded.static_structured_jumps[index] = plans;
        let (source, plan) = &bounded.static_structured_jumps[index][0];
        assert!(matches!(
            bounded.structured_jump_transition(
                artifact.functions[index].key,
                *source as usize,
                plan.target
            ),
            Some(std::borrow::Cow::Borrowed(_))
        ));

        // A branch entering a scope needs an additional retained kind, unlike ordinary loop backedges.
        let mut function = artifact.functions[index].clone();
        function.code[0] = erabasic_bytecode::EncodedInstruction::new(
            Opcode::Jump,
            u32::try_from(ranges[0].start)
                .unwrap()
                .to_le_bytes()
                .to_vec(),
        );
        let entries = function
            .code
            .iter()
            .filter(|encoded| {
                matches!(
                    Opcode::try_from(encoded.opcode),
                    Ok(Opcode::Jump | Opcode::JumpIfFalse)
                )
            })
            .count();
        for extra in [0, std::mem::size_of::<StructuredScopeKind>()] {
            let mut bytes = entries * entry_bytes + extra;
            let mut work = usize::MAX;
            let plans =
                scope_transitions::plan_static_jumps(&function, ranges, &mut bytes, &mut work);
            assert_eq!(plans.iter().any(|(source, _)| *source == 0), extra != 0);
        }
    }

    #[test]
    fn static_structured_jumps_match_fallback_for_every_edge_and_respect_capacity() {
        let artifact = Arc::new(compiled_generation_fixture());
        let generation = ProgramGeneration::new(Arc::clone(&artifact));
        assert!(
            generation
                .literal_group_match_plans
                .iter()
                .flatten()
                .any(|(_, plan)| {
                    matches!(plan.candidates, LiteralGroupMatchCandidates::Integers(_))
                })
        );
        let mut cached = 0;
        for (index, function) in artifact.functions.iter().enumerate() {
            let ranges = &generation.structured_scope_ranges[index];
            let mut no_bytes = 0;
            let mut unlimited_work = usize::MAX;
            assert!(
                scope_transitions::plan_static_jumps(
                    function,
                    ranges,
                    &mut no_bytes,
                    &mut unlimited_work
                )
                .is_empty()
            );
            for source in 0..function.code.len() {
                for target in 0..=function.code.len() {
                    let actual = generation
                        .structured_jump_transition(function.key, source, target)
                        .unwrap();
                    assert_eq!(
                        *actual,
                        scope_transitions::transition(ranges, source, target)
                    );
                    cached += usize::from(matches!(actual, std::borrow::Cow::Borrowed(_)));
                }
            }
        }
        assert!(cached > 0, "fixture must exercise real cached branch edges");
    }
    use erabasic_analyzer::{
        AnalysisInput, AnalyzerOptions, ExtensionRegistry, ProjectSource, SourcePayload,
        analyze_project,
    };
    use erabasic_compiler::{CompilerOptions, compile_project, default_host_registry};
    use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};

    fn compiled_generation_fixture() -> BytecodeArtifact {
        compiled_generation_source(
            "@SYSTEM_TITLE\n#DIMS VALUE\nVALUE '= \"keep\"\n\
             RESULT = GROUPMATCH(VALUE, \"keep\", \"other\", \"keep\")\n\
             RESULT:1 = GROUPMATCH(RESULT, 1, 0, 1)\n\
             CALL CLEAR_ROW(2, 0)\nRETURN\n\
             @CLEAR_ROW(ARG, VALUE)\n#DIM VALUE\n#LOCALSIZE 1\n\
             FOR LOCAL, 0, 4\nDA:ARG:LOCAL = 0\nNEXT\nRETURN\n\
             @REF_VALUES(NUMBERS, TEXTS)\n#DIM REF NUMBERS\n#DIMS REF TEXTS\nRETURN\n",
        )
    }

    fn compiled_generation_source(source: &str) -> BytecodeArtifact {
        let project_data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
            .data
            .expect("default project data");
        let analysis = analyze_project(
            AnalysisInput {
                project_data,
                sources: vec![ProjectSource {
                    relative_path: "main.erb".into(),
                    payload: SourcePayload::Utf8(source.into()),
                }],
            },
            &AnalyzerOptions::analysis_mode(),
            &ExtensionRegistry::default(),
        );
        assert!(analysis.project.is_some(), "{:#?}", analysis.diagnostics);
        let compile = compile_project(
            analysis.project.as_ref().expect("analyzed project"),
            &CompilerOptions::default(),
            &default_host_registry(),
            None,
        );
        assert!(compile.artifact.is_some(), "{:#?}", compile.diagnostics);
        compile.artifact.expect("compiled artifact")
    }

    #[test]
    fn indexed_scoped_lookup_matches_ordered_scan_across_generations() {
        let mut artifact = compiled_generation_fixture();
        let mut duplicate = artifact
            .globals
            .iter()
            .find(|global| global.owner.is_none())
            .expect("root global")
            .clone();
        duplicate.key = SymbolKey([0xf1; 16]);
        artifact.globals.push(duplicate);
        let generation = ProgramGeneration::new(Arc::new(artifact));
        for program in [&generation, &generation.clone()] {
            for function in &program.artifact.functions {
                for name in program
                    .artifact
                    .globals
                    .iter()
                    .map(|global| global.name.to_ascii_lowercase())
                    .chain(["missing_variable_xyz".into()])
                {
                    let expected = program
                        .function_locals(function.key)
                        .chain(program.function_statics(function.key))
                        .chain(
                            program
                                .artifact
                                .globals
                                .iter()
                                .filter(|global| global.owner.is_none()),
                        )
                        .find(|global| global.name.eq_ignore_ascii_case(&name))
                        .map(|global| global.key);
                    assert_eq!(
                        program
                            .scoped_variable(function.key, &name)
                            .map(|global| global.key),
                        expected,
                        "{}:{name}",
                        function.name
                    );
                }
            }
        }
        let mut next = (*generation.artifact).clone();
        for global in &mut next.globals {
            if global.owner.is_none() {
                global.name = format!("RENAMED_{}", global.name);
            }
        }
        let next = ProgramGeneration::new(Arc::new(next));
        let missing_function = SymbolKey([0xf2; 16]);
        for global in generation
            .artifact
            .globals
            .iter()
            .filter(|global| global.owner.is_none())
        {
            assert!(
                generation
                    .scoped_variable(missing_function, &global.name)
                    .is_some()
            );
            assert!(
                next.scoped_variable(missing_function, &global.name)
                    .is_none()
            );
        }
    }

    #[test]
    fn indexed_dynamic_lookup_preserves_exact_owner_order_and_generations() {
        let mut artifact = compiled_generation_fixture();
        let owners = [artifact.functions[0].key, artifact.functions[1].key];
        artifact.functions[1].name = artifact.functions[0].name.clone();
        let template = artifact.globals[0].clone();
        for (index, owner) in [
            Some(owners[0]),
            None,
            Some(owners[1]),
            Some(owners[0]),
            None,
        ]
        .into_iter()
        .enumerate()
        {
            let mut definition = template.clone();
            definition.key = SymbolKey([u8::try_from(0xd0 + index).unwrap(); 16]);
            definition.name = "Lookupé".into();
            definition.owner = owner;
            definition.storage = BytecodeStorage::FunctionPersistent;
            artifact.globals.push(definition);
        }
        let first = ProgramGeneration::new(Arc::new(artifact));
        let mut next = (*first.artifact).clone();
        next.globals.reverse();
        let next = ProgramGeneration::new(Arc::new(next));
        for program in [&first, &first.clone(), &next] {
            for owner in owners.into_iter().chain([SymbolKey([0xfe; 16])]) {
                for name in program
                    .artifact
                    .globals
                    .iter()
                    .map(|global| global.name.to_ascii_lowercase())
                    .chain(["LOOKUPé".into(), "lookupÉ".into(), "not_present".into()])
                {
                    let expected = program
                        .artifact
                        .globals
                        .iter()
                        .find(|global| {
                            global.owner == Some(owner) && global.name.eq_ignore_ascii_case(&name)
                        })
                        .or_else(|| {
                            program.artifact.globals.iter().find(|global| {
                                global.owner.is_none() && global.name.eq_ignore_ascii_case(&name)
                            })
                        });
                    let actual = program.dynamic_variable(owner, &name);
                    assert_eq!(
                        actual.map(|global| global.key),
                        expected.map(|global| global.key),
                        "{name}"
                    );
                    if let (Some(actual), Some(expected)) = (actual, expected) {
                        assert!(std::ptr::eq(actual, expected));
                    }
                }
            }
        }
    }

    #[test]
    fn indexed_jump_transition_keeps_cached_and_uncached_scope_semantics() {
        let program = ProgramGeneration::new(Arc::new(compiled_generation_fixture()));
        let mut uncached = program.clone();
        uncached.static_structured_jumps.clear();
        for generation in [&program, &uncached] {
            for (index, function) in generation.artifact.functions.iter().enumerate() {
                for source in 0..function.code.len() {
                    for target in 0..=function.code.len() {
                        assert_eq!(
                            *generation
                                .structured_jump_transition_at_index(index, source, target)
                                .unwrap(),
                            scope_transitions::transition(
                                &generation.structured_scope_ranges[index],
                                source,
                                target
                            ),
                        );
                    }
                }
            }
            assert!(
                generation
                    .structured_jump_transition_at_index(usize::MAX, 0, 0)
                    .is_none()
            );
        }
    }

    #[test]
    fn indexed_function_names_keep_first_match_and_generation_identity() {
        let mut artifact = compiled_generation_fixture();
        let first = artifact.functions[0].clone();
        let mut duplicate = first.clone();
        duplicate.key = SymbolKey([0xf3; 16]);
        duplicate.name = first.name.to_ascii_lowercase();
        artifact.functions.push(duplicate);
        let generation = ProgramGeneration::new(Arc::new(artifact));
        assert_eq!(
            generation
                .function_by_name(&first.name.to_ascii_lowercase())
                .map(|function| function.key),
            Some(first.key)
        );
        assert!(
            generation
                .function_by_name("missing_function_xyz")
                .is_none()
        );
        let mut next = (*generation.artifact).clone();
        next.functions[0].name = "RENAMED_FIRST_FUNCTION".into();
        let next = ProgramGeneration::new(Arc::new(next));
        assert_eq!(
            next.function_by_name(&first.name)
                .map(|function| function.key),
            Some(SymbolKey([0xf3; 16]))
        );
        assert_eq!(
            generation
                .clone()
                .function_by_name(&first.name)
                .map(|function| function.key),
            Some(first.key)
        );
    }

    #[test]
    fn reference_membership_matches_metadata_and_is_generation_local() {
        let artifact = Arc::new(compiled_generation_fixture());
        let generation = ProgramGeneration::new(Arc::clone(&artifact));
        let mut references = 0;
        for symbol in &artifact.runtime_variables {
            assert_eq!(
                generation.is_reference_variable(symbol.key),
                symbol.reference
            );
            references += usize::from(symbol.reference);
        }
        assert_eq!(references, 2);
        let missing = SymbolKey([0xff; 16]);
        assert!(
            artifact
                .runtime_variables
                .iter()
                .all(|symbol| symbol.key != missing)
        );
        assert!(!generation.is_reference_variable(missing));

        let cloned = generation.clone();
        let mut next_artifact = (*artifact).clone();
        for symbol in &mut next_artifact.runtime_variables {
            symbol.reference = false;
        }
        let next = ProgramGeneration::new(Arc::new(next_artifact));
        for symbol in &artifact.runtime_variables {
            assert_eq!(cloned.is_reference_variable(symbol.key), symbol.reference);
            assert!(!next.is_reference_variable(symbol.key));
        }
    }

    #[test]
    fn sparse_instruction_lookup_preserves_exact_instruction_identity() {
        let plans = [(2, "first"), (19, "second")];
        assert_eq!(sparse_instruction_plan(&plans, 2), Some(&"first"));
        assert_eq!(sparse_instruction_plan(&plans, 19), Some(&"second"));
        assert_eq!(sparse_instruction_plan(&plans, 18), None);
        assert_eq!(sparse_instruction_plan(&plans, usize::MAX), None);
    }

    #[test]
    fn compact_instruction_indices_use_four_bytes_per_slot() {
        assert_eq!(std::mem::size_of::<u32>(), 4);
        assert!(std::mem::size_of::<u32>() < std::mem::size_of::<Option<usize>>());
    }

    #[test]
    fn real_generation_preserves_sparse_fastpaths_globals_sources_and_progress() {
        let artifact = Arc::new(compiled_generation_fixture());
        let mut progress = Vec::new();
        let generation = ProgramGeneration::new_with_progress(
            Arc::clone(&artifact),
            Some(&mut |event| progress.push(event)),
        );

        assert!(
            generation
                .bulk_fill_loop_plans
                .iter()
                .any(|plans| !plans.is_empty())
        );
        assert!(
            generation
                .literal_group_match_plans
                .iter()
                .any(|plans| !plans.is_empty())
        );
        let mut saw_global = false;
        let mut saw_sentinel = false;
        for (function_index, function) in artifact.functions.iter().enumerate() {
            let mut code_offset = 0;
            for (instruction_index, instruction) in function.code.iter().enumerate() {
                let expected_source = artifact.source_map.resolve(function.key, code_offset);
                assert_eq!(
                    generation.source_location(function.key, instruction_index),
                    expected_source
                );
                code_offset = code_offset.saturating_add(instruction.encoded_len());

                if generation
                    .instruction_global(function_index, instruction_index)
                    .is_some()
                {
                    saw_global = true;
                } else {
                    saw_sentinel = true;
                }
            }
        }
        assert!(saw_global);
        assert!(saw_sentinel);
        assert_eq!(progress.first().map(|event| event.completed), Some(0));
        assert_eq!(
            progress.last().map(|event| (event.completed, event.total)),
            progress.last().map(|event| (event.total, event.total))
        );
        assert!(progress.windows(2).all(|events| {
            events[0].stage == VmPreparationStage::IndexingProgram
                && events[1].stage == VmPreparationStage::IndexingProgram
                && events[0].completed <= events[1].completed
                && events[0].total == events[1].total
        }));
    }
}
