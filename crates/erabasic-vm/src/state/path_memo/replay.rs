#[allow(clippy::wildcard_imports)]
use super::*;

impl Vm {
    pub(crate) fn try_replay_path_memo(
        &mut self,
        fiber: &mut Fiber,
        probe: (&PathMemoHead, &[VmValue]),
        host: &impl VmHost,
        natives: &NativeServiceRegistry,
        remaining_quantum: u32,
        remaining_instructions: u64,
    ) -> Result<Option<(VmValue, u64)>, VmError> {
        let (head, arguments) = probe;
        if self.active_path_memo_fiber.get().is_some() {
            let mut active = self.active_path_memo.borrow_mut();
            if active.as_ref().is_some_and(|active| active.valid) {
                return Ok(None);
            }
            active.take();
            self.active_path_memo_fiber.set(None);
        }
        if !self.runnable.is_empty() {
            return Ok(None);
        }
        let generations = &self.generations;
        let memory = &mut self.memory;
        let Some(entry) = path_memo_entries(&self.path_memo_cache, head, arguments)
            .and_then(|entries| {
                entries.iter().find(|entry| {
                    entry.body_instructions.saturating_add(1) <= u64::from(remaining_quantum)
                        && entry.body_instructions.saturating_add(1) <= remaining_instructions
                        && path_memo_dependencies_match(generations, memory, entry)
                        && entry
                            .safe_natives
                            .iter()
                            .all(|key| natives.path_memo_safe(*key))
                        && generations.get(&head.generation).is_some_and(|program| {
                            entry.safe_hosts.iter().all(|key| {
                                program
                                    .host_import_index(*key)
                                    .and_then(|index| program.artifact.host_imports.get(index))
                                    .is_some_and(|import| host.path_memo_safe(&import.import))
                            })
                        })
                })
            })
            .cloned()
        else {
            return Ok(None);
        };
        let result = if let Some(dependency) = entry.result_dependency {
            let Some(PathMemoDependency::Value { place, .. }) = entry.dependencies.get(dependency)
            else {
                return Ok(None);
            };
            let Some(value) = read_path_memo_place(generations, memory, place) else {
                return Ok(None);
            };
            value
        } else {
            entry.result.clone()
        };
        for group in &entry.mutation_groups {
            if let Some(final_cell) = &group.final_cell {
                let (generation, variable, character) = group.key();
                let definition = generations
                    .get(&generation)
                    .and_then(|program| program.global(variable))
                    .ok_or_else(|| VmError::InvalidState("path memo variable is missing".into()))?;
                let cell = memory
                    .cell_mut(generation, definition.key, definition.storage, character)
                    .ok_or_else(|| VmError::InvalidState("path memo storage is missing".into()))?;
                if cell != final_cell {
                    cell.replace_contents_from(final_cell)
                        .map_err(VmError::InvalidState)?;
                }
                continue;
            }
            for mutation in &group.mutations {
                replay_path_memo_mutation(generations, memory, mutation)?;
            }
        }
        fiber.backward_branches_without_progress = fiber
            .backward_branches_without_progress
            .saturating_add(entry.backward_branches);
        fiber.consecutive_budget_exhaustions = 0;
        #[cfg(test)]
        {
            self.path_memo_replays = self.path_memo_replays.saturating_add(1);
        }
        Ok(Some((result, entry.body_instructions)))
    }

    pub(crate) fn complete_path_memo(
        &mut self,
        fiber: &Fiber,
        frame: FrameId,
        result: Option<&VmValue>,
    ) {
        if self.active_path_memo_fiber.get() != Some(fiber.id) {
            return;
        }
        let matches = self
            .active_path_memo
            .borrow()
            .as_ref()
            .is_some_and(|active| active.fiber == fiber.id && active.frame == frame);
        if !matches {
            return;
        }
        let mut active = self
            .active_path_memo
            .borrow_mut()
            .take()
            .expect("matching path memo trace exists");
        self.active_path_memo_fiber.set(None);
        active.body_instructions = active.body_instructions.saturating_add(1);
        let backward_branches = fiber
            .backward_branches_without_progress
            .saturating_sub(active.backward_branches_before);
        let Some(result) = result else {
            return;
        };
        active.retained_bytes = active
            .retained_bytes
            .saturating_add(retained_value_bytes(result));
        if !active.valid
            || active.body_instructions > active.maximum_body_instructions
            || active.retained_bytes > MAX_RETAINED_BYTES
            || fiber.backward_branches_without_progress
                > self.config.maximum_backward_branches_without_progress
        {
            return;
        }
        let remaining_retained_bytes = MAX_RETAINED_BYTES - active.retained_bytes;
        let Some((mutation_groups, final_cells_retained_bytes, released_mutation_bytes)) = self
            .prepare_path_memo_mutation_groups(
                std::mem::take(&mut active.mutations),
                remaining_retained_bytes,
            )
        else {
            return;
        };
        active.retained_bytes = active
            .retained_bytes
            .saturating_sub(released_mutation_bytes)
            .saturating_add(final_cells_retained_bytes);
        debug_assert!(active.retained_bytes <= MAX_RETAINED_BYTES);
        let PathMemoBaseKey { head, arguments } = active.key;
        let mut new_key = self
            .path_memo_cache
            .get(&head)
            .is_none_or(|paths| !paths.contains_key(arguments.as_slice()));
        if new_key && self.path_memo_key_count >= MAX_PATH_MEMO_KEYS {
            self.clear_path_memo_cache();
            new_key = true;
        }
        if self
            .path_memo_retained_bytes
            .saturating_add(active.retained_bytes)
            > MAX_CACHE_RETAINED_BYTES
        {
            self.clear_path_memo_cache();
            new_key = true;
        }
        if new_key {
            self.path_memo_key_count = self.path_memo_key_count.saturating_add(1);
        }
        let entries = self
            .path_memo_cache
            .entry(head)
            .or_default()
            .entry(arguments)
            .or_default();
        if entries.len() >= MAX_PATHS_PER_KEY {
            self.path_memo_retained_bytes = self
                .path_memo_retained_bytes
                .saturating_sub(entries.remove(0).retained_bytes);
        }
        self.path_memo_retained_bytes = self
            .path_memo_retained_bytes
            .saturating_add(active.retained_bytes);
        entries.push(Arc::new(PathMemoEntry {
            dependencies: active.dependencies,
            safe_natives: active.safe_natives,
            safe_hosts: active.safe_hosts,
            mutation_groups,
            result: result.clone(),
            result_dependency: active.result_dependency,
            body_instructions: active.body_instructions,
            backward_branches,
            retained_bytes: active.retained_bytes,
        }));
    }

    fn prepare_path_memo_mutation_groups(
        &self,
        mutations: Vec<PathMemoMutation>,
        retained_bytes_budget: usize,
    ) -> Option<(Vec<PathMemoMutationGroup>, usize, usize)> {
        let mut group_indices = BTreeMap::new();
        let mut groups = Vec::<PathMemoMutationGroup>::new();
        for mutation in mutations {
            let key @ (generation, variable, character) = mutation.cell_key();
            let index = *group_indices.entry(key).or_insert_with(|| {
                groups.push(PathMemoMutationGroup {
                    generation,
                    variable,
                    character,
                    mutations: Vec::new(),
                    final_cell: None,
                });
                groups.len() - 1
            });
            groups[index].mutations.push(mutation);
        }
        let mut retained_bytes = 0_usize;
        let mut released_mutation_bytes = 0_usize;
        for group in &mut groups {
            let (generation, variable, character) = group.key();
            let definition = self
                .generations
                .get(&generation)
                .and_then(|program| program.global(variable))?;
            let cell = self.memory.cell(generation, definition, character)?;
            if cell.len() > MAX_IDEMPOTENT_REPLAY_CELL_ELEMENTS {
                continue;
            }
            let has_full_cover = group
                .mutations
                .iter()
                .any(|mutation| mutation.covers_entire_cell(cell.len()));
            if !has_full_cover {
                continue;
            }
            // Once a path overwrites a whole cell, only that cell's final contents remain
            // observable after the function returns. Retain one storage-native snapshot instead
            // of both the full mutation log and a duplicate snapshot. Replaying the snapshot also
            // collapses repeated writes into one revision change.
            let group_released_bytes = group
                .mutations
                .iter()
                .map(path_memo_mutation_retained_bytes)
                .sum::<usize>();
            let cell_retained_bytes = cell.retained_bytes();
            let available = retained_bytes_budget
                .saturating_add(released_mutation_bytes)
                .saturating_add(group_released_bytes)
                .saturating_sub(retained_bytes);
            if cell_retained_bytes > available {
                return None;
            }
            released_mutation_bytes = released_mutation_bytes.saturating_add(group_released_bytes);
            group.mutations.clear();
            retained_bytes = retained_bytes.saturating_add(cell_retained_bytes);
            group.final_cell = Some(cell.clone());
        }
        Some((groups, retained_bytes, released_mutation_bytes))
    }
}
