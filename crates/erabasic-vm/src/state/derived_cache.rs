#[allow(clippy::wildcard_imports)]
use super::*;
use std::mem::{size_of, take};

const MAXIMUM_DERIVED_CACHE_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_DERIVED_CACHE_ENTRIES: usize = 65_536;

impl Vm {
    pub(crate) fn clear_derived_caches(&mut self) {
        drop(take(&mut self.find_element_cache));
        self.find_element_cache_retained_bytes = 0;
        drop(take(&mut self.function_memo_cache));
        self.function_memo_cache_retained_bytes = 0;
    }

    pub(crate) fn cache_find_element(&mut self, key: FindElementCacheKey, result: i64) {
        let retained = find_element_key_bytes(&key).saturating_add(size_of::<i64>());
        if retained > MAXIMUM_DERIVED_CACHE_BYTES {
            return;
        }
        let previous_retained = self
            .find_element_cache
            .get_key_value(&key)
            .map_or(0, |(key, _)| {
                find_element_key_bytes(key).saturating_add(size_of::<i64>())
            });
        let replacing = previous_retained != 0;
        let projected = self
            .find_element_cache_retained_bytes
            .saturating_sub(previous_retained)
            .saturating_add(retained);
        if (!replacing && self.find_element_cache.len() >= MAXIMUM_DERIVED_CACHE_ENTRIES)
            || projected > MAXIMUM_DERIVED_CACHE_BYTES
        {
            drop(take(&mut self.find_element_cache));
            self.find_element_cache_retained_bytes = 0;
        } else if replacing {
            self.find_element_cache.remove(&key);
            self.find_element_cache_retained_bytes = self
                .find_element_cache_retained_bytes
                .saturating_sub(previous_retained);
        }
        self.find_element_cache.insert(key, result);
        self.find_element_cache_retained_bytes = self
            .find_element_cache_retained_bytes
            .saturating_add(retained);
    }

    pub(crate) fn cache_function_memo(&mut self, key: FunctionMemoKey, entry: FunctionMemoEntry) {
        let retained = function_memo_bytes(&key, &entry);
        if retained > MAXIMUM_DERIVED_CACHE_BYTES {
            return;
        }
        let previous_retained = self
            .function_memo_cache
            .get_key_value(&key)
            .map_or(0, |(key, entry)| function_memo_bytes(key, entry));
        let replacing = previous_retained != 0;
        let projected = self
            .function_memo_cache_retained_bytes
            .saturating_sub(previous_retained)
            .saturating_add(retained);
        if (!replacing && self.function_memo_cache.len() >= MAXIMUM_DERIVED_CACHE_ENTRIES)
            || projected > MAXIMUM_DERIVED_CACHE_BYTES
        {
            drop(take(&mut self.function_memo_cache));
            self.function_memo_cache_retained_bytes = 0;
        } else if replacing {
            self.function_memo_cache.remove(&key);
            self.function_memo_cache_retained_bytes = self
                .function_memo_cache_retained_bytes
                .saturating_sub(previous_retained);
        }
        self.function_memo_cache.insert(key, entry);
        self.function_memo_cache_retained_bytes = self
            .function_memo_cache_retained_bytes
            .saturating_add(retained);
    }

    pub(crate) fn function_memo_key(
        &self,
        generation: GenerationId,
        function: SymbolKey,
        arguments: &[VmValue],
    ) -> Option<FunctionMemoKey> {
        let program = self.generations.get(&generation)?;
        let plan = program.function_memo_plan(function)?;
        let arguments = arguments
            .iter()
            .map(MemoValue::from_vm)
            .collect::<Option<Vec<_>>>()?;
        let dependency_revisions = plan
            .dependency_indices
            .iter()
            .map(|index| {
                let definition = program.artifact.globals.get(*index)?;
                self.memory
                    .cell(generation, definition, 0)
                    .map(VariableCell::revision)
            })
            .collect::<Option<Vec<_>>>()?;
        Some(FunctionMemoKey {
            generation,
            function,
            arguments,
            dependency_revisions,
        })
    }

    pub(crate) fn capture_function_memo_entry(
        &self,
        key: &FunctionMemoKey,
        result: VmValue,
    ) -> Option<FunctionMemoEntry> {
        let program = self.generations.get(&key.generation)?;
        let scratch = program
            .function_memo_plan(key.function)?
            .scratch_indices
            .iter()
            .map(|index| {
                let definition = program.artifact.globals.get(*index)?;
                self.memory
                    .cell(key.generation, definition, 0)?
                    .read(&[])
                    .ok()
                    .map(|value| (definition.key, value))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(FunctionMemoEntry { result, scratch })
    }

    pub(crate) fn replay_function_memo_entry(
        &mut self,
        generation: GenerationId,
        entry: &FunctionMemoEntry,
    ) -> Result<(), VmError> {
        let program = self
            .generations
            .get(&generation)
            .ok_or_else(|| VmError::InvalidState("memo generation is missing".into()))?;
        for (variable, value) in &entry.scratch {
            let definition = program
                .global(*variable)
                .ok_or_else(|| VmError::InvalidState("memo scratch variable is missing".into()))?;
            self.memory
                .cell_mut(generation, definition.key, definition.storage, 0)
                .ok_or_else(|| VmError::InvalidState("memo scratch storage is missing".into()))?
                .write(&[], value.clone())
                .map_err(VmError::InvalidState)?;
        }
        Ok(())
    }
}

fn find_element_key_bytes(key: &FindElementCacheKey) -> usize {
    size_of::<FindElementCacheKey>().saturating_add(match &key.needle {
        FindElementNeedle::Integer(_) => 0,
        FindElementNeedle::String(value) => value.len(),
    })
}

fn function_memo_bytes(key: &FunctionMemoKey, entry: &FunctionMemoEntry) -> usize {
    let arguments = key.arguments.iter().map(memo_value_bytes).sum::<usize>();
    let dependencies = key
        .dependency_revisions
        .len()
        .saturating_mul(size_of::<u64>());
    let scratch = entry
        .scratch
        .iter()
        .map(|(_, value)| size_of::<SymbolKey>().saturating_add(vm_value_bytes(value)))
        .sum::<usize>();
    size_of::<FunctionMemoKey>()
        .saturating_add(size_of::<FunctionMemoEntry>())
        .saturating_add(arguments)
        .saturating_add(dependencies)
        .saturating_add(vm_value_bytes(&entry.result))
        .saturating_add(scratch)
}

fn memo_value_bytes(value: &MemoValue) -> usize {
    size_of::<MemoValue>().saturating_add(match value {
        MemoValue::Integer(_) => 0,
        MemoValue::String(value) => value.len(),
    })
}

fn vm_value_bytes(value: &VmValue) -> usize {
    size_of::<VmValue>().saturating_add(match value {
        VmValue::Integer(_) => 0,
        VmValue::String(value) => value.len(),
        VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => {
            place.indices.len().saturating_mul(size_of::<u64>())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use erabasic_analyzer::{
        AnalysisInput, AnalyzerOptions, ExtensionRegistry, ProjectSource, SourcePayload,
        analyze_project,
    };
    use erabasic_compiler::{CompilerOptions, compile_project, default_host_registry};
    use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};
    use erabasic_validator::{ValidationContext, validate_bytecode};

    fn vm_fixture() -> Vm {
        let project_data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
            .data
            .expect("default project data");
        let analysis = analyze_project(
            AnalysisInput {
                project_data,
                sources: vec![ProjectSource {
                    relative_path: "main.erb".into(),
                    payload: SourcePayload::Utf8("@SYSTEM_TITLE\nRETURN RESULT\n".into()),
                }],
            },
            &AnalyzerOptions::default(),
            &ExtensionRegistry::default(),
        );
        let artifact = compile_project(
            analysis.project.as_ref().expect("analyzed project"),
            &CompilerOptions::default(),
            &default_host_registry(),
            None,
        )
        .artifact
        .expect("compiled artifact");
        let report = validate_bytecode(
            artifact.clone().into_unvalidated(),
            &ValidationContext::for_artifact(&artifact),
        );
        Vm::new(
            report.value.expect("validated artifact"),
            VmConfig::default(),
        )
    }

    fn find_key(revision: u8, length: usize) -> FindElementCacheKey {
        FindElementCacheKey {
            generation: GenerationId(1),
            variable: SymbolKey([revision; 16]),
            revision: u64::from(revision),
            start: 0,
            end: 1,
            last: false,
            exact: true,
            needle: FindElementNeedle::String("x".repeat(length)),
        }
    }

    #[test]
    fn derived_caches_are_bounded_by_retained_bytes_and_drop_allocations_on_clear() {
        let mut vm = vm_fixture();
        for revision in 0..17 {
            vm.cache_find_element(find_key(revision, 1024 * 1024), -1);
        }
        assert!(vm.find_element_cache.len() < 17);
        assert!(vm.find_element_cache_retained_bytes <= MAXIMUM_DERIVED_CACHE_BYTES);

        let key = FunctionMemoKey {
            generation: GenerationId(1),
            function: SymbolKey([1; 16]),
            arguments: vec![MemoValue::String("x".repeat(1024 * 1024))],
            dependency_revisions: Vec::new(),
        };
        for revision in 0..17_u8 {
            let mut key = key.clone();
            key.function = SymbolKey([revision; 16]);
            vm.cache_function_memo(
                key,
                FunctionMemoEntry {
                    result: VmValue::Integer(0),
                    scratch: Vec::new(),
                },
            );
        }
        assert!(vm.function_memo_cache.len() < 17);
        assert!(vm.function_memo_cache_retained_bytes <= MAXIMUM_DERIVED_CACHE_BYTES);

        vm.clear_derived_caches();
        assert_eq!(vm.find_element_cache.capacity(), 0);
        assert_eq!(vm.function_memo_cache.capacity(), 0);
        assert_eq!(vm.find_element_cache_retained_bytes, 0);
        assert_eq!(vm.function_memo_cache_retained_bytes, 0);
    }

    #[test]
    fn replacing_a_derived_cache_entry_uses_the_net_retained_size() {
        let mut vm = vm_fixture();
        let key = find_key(1, MAXIMUM_DERIVED_CACHE_BYTES / 2);
        vm.cache_find_element(key.clone(), 1);
        let retained = vm.find_element_cache_retained_bytes;
        vm.cache_find_element(key, 2);
        assert_eq!(vm.find_element_cache.len(), 1);
        assert_eq!(vm.find_element_cache_retained_bytes, retained);
    }

    #[test]
    fn generation_reclamation_only_clears_caches_when_a_generation_is_removed() {
        let mut active_vm = vm_fixture();
        let first = active_vm.current_generation;
        let artifact = Arc::clone(&active_vm.generations[&first].artifact);
        let entry = artifact.functions[0].key;
        active_vm
            .spawn_entry(entry, Vec::new())
            .expect("spawn fixture");
        let second = GenerationId(first.0 + 1);
        active_vm.generations.insert(
            second,
            Arc::new(ProgramGeneration::new(Arc::clone(&artifact))),
        );
        active_vm.current_generation = second;
        active_vm.cache_find_element(find_key(1, 32), -1);
        let retained = active_vm.find_element_cache_retained_bytes;

        active_vm.reclaim_generations();
        assert!(active_vm.generations.contains_key(&first));
        assert_eq!(active_vm.find_element_cache_retained_bytes, retained);

        let mut reclaiming_vm = vm_fixture();
        let first = reclaiming_vm.current_generation;
        let artifact = Arc::clone(&reclaiming_vm.generations[&first].artifact);
        let second = GenerationId(first.0 + 1);
        reclaiming_vm
            .generations
            .insert(second, Arc::new(ProgramGeneration::new(artifact)));
        reclaiming_vm.current_generation = second;
        reclaiming_vm.cache_find_element(find_key(2, 32), -1);

        reclaiming_vm.reclaim_generations();
        assert!(!reclaiming_vm.generations.contains_key(&first));
        assert!(reclaiming_vm.find_element_cache.is_empty());
        assert_eq!(reclaiming_vm.find_element_cache_retained_bytes, 0);
    }
}
