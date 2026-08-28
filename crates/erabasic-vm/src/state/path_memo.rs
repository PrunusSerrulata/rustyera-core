#[allow(clippy::wildcard_imports)]
use super::*;
use crate::VmHost;

const MAX_PATH_MEMO_KEYS: usize = 8_192;
const MAX_PATHS_PER_KEY: usize = 4;
const MAX_DEPENDENCIES: usize = 128;
const MAX_MUTATIONS: usize = 512;
const MAX_RETAINED_BYTES: usize = 64 * 1024;
const MAX_CACHE_RETAINED_BYTES: usize = 16 * 1024 * 1024;
const MAX_IDEMPOTENT_REPLAY_CELL_ELEMENTS: usize = 4_096;

impl Vm {
    pub(crate) fn observe_path_memo_arguments(
        &self,
        fiber: FiberId,
        generation: GenerationId,
        function: &BytecodeFunction,
        program: &ProgramGeneration,
        arguments: &[VmValue],
    ) {
        if !self.path_memo_can_observe(fiber) {
            return;
        }
        for (parameter, argument) in function.parameters.iter().zip(arguments) {
            let destination =
                persistent_argument_destination(&self.memory, generation, parameter, program);
            let Ok(destination) = destination else {
                self.invalidate_path_memo(fiber);
                return;
            };
            if let Some(PersistentArgumentDestination {
                definition,
                character,
                implicit_target,
                indices,
            }) = destination
            {
                self.observe_path_memo_write(
                    fiber,
                    generation,
                    definition,
                    character,
                    implicit_target,
                    indices,
                    argument,
                );
            }
        }
    }

    pub(crate) fn path_memo_is_active_for(&self, fiber: FiberId) -> bool {
        if self.active_path_memo_fiber.get() != Some(fiber) {
            return false;
        }
        self.active_path_memo
            .borrow()
            .as_ref()
            .is_some_and(|active| active.fiber == fiber && active.valid)
    }

    fn path_memo_can_observe(&self, fiber: FiberId) -> bool {
        if self.active_path_memo_fiber.get() != Some(fiber) {
            return false;
        }
        self.active_path_memo
            .borrow()
            .as_ref()
            .is_some_and(|active| active.fiber == fiber && active.valid)
    }

    pub(crate) fn path_memo_head(
        generation: GenerationId,
        function: SymbolKey,
        arguments: &[VmValue],
    ) -> Option<PathMemoHead> {
        if arguments
            .iter()
            .any(|argument| matches!(argument, VmValue::IntegerPlace(_) | VmValue::StringPlace(_)))
        {
            return None;
        }
        Some(PathMemoHead {
            generation,
            function,
        })
    }

    pub(crate) fn begin_path_memo(
        &self,
        fiber: &Fiber,
        frame: FrameId,
        function: &BytecodeFunction,
        head: PathMemoHead,
        arguments: &[VmValue],
        maximum_body_instructions: u64,
    ) {
        if self
            .active_path_memo
            .borrow()
            .as_ref()
            .is_some_and(|active| active.valid)
            || !self.runnable.is_empty()
            || function.result.is_none()
            || function.parameters.iter().any(|parameter| {
                parameter.by_reference
                    || matches!(
                        parameter.value_type,
                        BytecodeType::IntegerPlace | BytecodeType::StringPlace
                    )
            })
        {
            return;
        }
        let retained_bytes = arguments.iter().fold(0_usize, |retained, argument| {
            retained.saturating_add(retained_value_bytes(argument))
        });
        if maximum_body_instructions == 0 || retained_bytes > MAX_RETAINED_BYTES {
            return;
        }
        *self.active_path_memo.borrow_mut() = Some(ActivePathMemo {
            fiber: fiber.id,
            frame,
            key: PathMemoBaseKey {
                head,
                arguments: arguments.to_vec(),
            },
            dependencies: Vec::new(),
            repeated_value_dependencies: BTreeSet::new(),
            safe_natives: Vec::new(),
            safe_hosts: Vec::new(),
            mutations: Vec::new(),
            pending_result_dependency: None,
            result_dependency: None,
            retained_bytes,
            body_instructions: 0,
            maximum_body_instructions,
            backward_branches_before: fiber.backward_branches_without_progress,
            skip_call_instruction: true,
            valid: true,
        });
        self.active_path_memo_fiber.set(Some(fiber.id));
    }

    pub(crate) fn clear_path_memo_cache(&mut self) {
        self.path_memo_cache.clear();
        self.path_memo_key_count = 0;
        self.path_memo_retained_bytes = 0;
    }

    pub(crate) fn observe_path_memo_instruction(&self, fiber: FiberId, instructions: u64) {
        if self.active_path_memo_fiber.get() != Some(fiber) {
            return;
        }
        let mut active = self.active_path_memo.borrow_mut();
        let Some(active) = active.as_mut().filter(|active| active.fiber == fiber) else {
            return;
        };
        if !active.valid {
            return;
        }
        if active.skip_call_instruction {
            active.skip_call_instruction = false;
            return;
        }
        active.body_instructions = active.body_instructions.saturating_add(instructions);
        if active.body_instructions >= active.maximum_body_instructions {
            active.valid = false;
        }
    }

    pub(crate) fn invalidate_path_memo(&self, fiber: FiberId) {
        if self.active_path_memo_fiber.get() != Some(fiber) {
            return;
        }
        let mut active = self.active_path_memo.borrow_mut();
        if active.as_ref().is_some_and(|active| active.fiber == fiber) {
            active.take();
            self.active_path_memo_fiber.set(None);
        }
    }

    pub(crate) fn abort_path_memo(&self, fiber: FiberId) {
        if self.active_path_memo_fiber.get() != Some(fiber) {
            return;
        }
        let remove = self
            .active_path_memo
            .borrow()
            .as_ref()
            .is_some_and(|active| active.fiber == fiber);
        if remove {
            self.active_path_memo.borrow_mut().take();
            self.active_path_memo_fiber.set(None);
        }
    }

    pub(crate) fn observe_path_memo_opcode(&self, fiber: FiberId, opcode: Opcode) {
        if matches!(
            opcode,
            Opcode::JumpDynamicLabel
                | Opcode::InvokeEvent
                | Opcode::Yield
                | Opcode::AwaitResume
                | Opcode::Trap
        ) {
            self.invalidate_path_memo(fiber);
        }
    }

    pub(crate) fn observe_path_memo_native(&self, fiber: FiberId, name: &str) {
        if !matches!(
            name,
            "__indexbyname"
                | "clearbit"
                | "escape"
                | "findelement"
                | "findlastelement"
                | "getnum"
                | "invertbit"
                | "isnumeric"
                | "setbit"
                | "split"
                | "swap"
                | "swapvar"
                | "toint"
                | "varset"
        ) {
            self.invalidate_path_memo(fiber);
        }
    }

    pub(crate) fn observe_path_memo_safe_native(&self, fiber: FiberId, key: SymbolKey) {
        if !self.path_memo_can_observe(fiber) {
            return;
        }
        let mut active = self.active_path_memo.borrow_mut();
        let Some(active) = active.as_mut().filter(|active| active.fiber == fiber) else {
            return;
        };
        if active.safe_natives.contains(&key) {
            return;
        }
        active.safe_natives.push(key);
        active.retained_bytes = active
            .retained_bytes
            .saturating_add(std::mem::size_of::<SymbolKey>());
        enforce_path_memo_limits(active);
    }

    pub(crate) fn observe_path_memo_safe_host(&self, fiber: FiberId, key: SymbolKey) {
        if !self.path_memo_can_observe(fiber) {
            return;
        }
        let mut active = self.active_path_memo.borrow_mut();
        let Some(active) = active.as_mut().filter(|active| active.fiber == fiber) else {
            return;
        };
        if active.safe_hosts.contains(&key) {
            return;
        }
        active.safe_hosts.push(key);
        active.retained_bytes = active
            .retained_bytes
            .saturating_add(std::mem::size_of::<SymbolKey>());
        enforce_path_memo_limits(active);
    }

    fn observe_path_memo_target_identity(
        &self,
        fiber: FiberId,
        generation: GenerationId,
        character: usize,
    ) -> bool {
        let mut active = self.active_path_memo.borrow_mut();
        let Some(active) = active.as_mut().filter(|active| active.fiber == fiber) else {
            return false;
        };
        if !active.valid {
            return false;
        }
        if let Some(observed_character) = active.dependencies.iter().find_map(|dependency| {
            let PathMemoDependency::TargetIdentity {
                generation: observed_generation,
                character,
            } = dependency
            else {
                return None;
            };
            (*observed_generation == generation).then_some(*character)
        }) {
            if observed_character != character {
                active.valid = false;
                return false;
            }
            return true;
        }
        active.retained_bytes = active
            .retained_bytes
            .saturating_add(std::mem::size_of::<PathMemoDependency>());
        active
            .dependencies
            .push(PathMemoDependency::TargetIdentity {
                generation,
                character,
            });
        enforce_path_memo_limits(active);
        active.valid
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe_path_memo_read(
        &self,
        fiber: FiberId,
        generation: GenerationId,
        definition: &erabasic_bytecode::BytecodeGlobal,
        character: usize,
        implicit_target: bool,
        indices: &[u64],
        value: &VmValue,
    ) {
        if definition.storage == BytecodeStorage::FunctionLocal {
            return;
        }
        if !self.path_memo_can_observe(fiber) {
            return;
        }
        if implicit_target && !self.observe_path_memo_target_identity(fiber, generation, character)
        {
            return;
        }
        let place = PathMemoPlace {
            generation,
            variable: definition.key,
            character,
            indices: indices.to_vec(),
        };
        let mut active = self.active_path_memo.borrow_mut();
        let Some(active) = active.as_mut().filter(|active| active.fiber == fiber) else {
            return;
        };
        if !active.valid {
            return;
        }
        if let Some(index) = active.dependencies.iter().position(|dependency| {
            matches!(
                dependency,
                PathMemoDependency::Value {
                    place: observed,
                    ..
                } if *observed == place
            )
        }) {
            active.repeated_value_dependencies.insert(index);
            return;
        }
        if active
            .mutations
            .iter()
            .any(|mutation| mutation.writes(&place))
        {
            return;
        }
        active.retained_bytes = active
            .retained_bytes
            .saturating_add(retained_value_bytes(value))
            .saturating_add(indices.len().saturating_mul(std::mem::size_of::<u64>()));
        active.dependencies.push(PathMemoDependency::Value {
            place,
            value: value.clone(),
        });
        enforce_path_memo_limits(active);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn mark_path_memo_result_read(
        &self,
        fiber: FiberId,
        frame: FrameId,
        generation: GenerationId,
        function: SymbolKey,
        instruction: usize,
        definition: &erabasic_bytecode::BytecodeGlobal,
        character: usize,
        indices: &[u64],
    ) {
        if self.active_path_memo_fiber.get() != Some(fiber) {
            return;
        }
        if !self
            .active_path_memo
            .borrow()
            .as_ref()
            .is_some_and(|active| active.frame == frame && active.valid)
        {
            return;
        }
        // Static candidates only avoid inspecting arbitrary bytecode at runtime. The actual
        // trace must still prove that this exact load is the root frame's unique, unmutated
        // result read before a replay may refresh it from current storage.
        let Some(_) = self
            .generations
            .get(&generation)
            .and_then(|program| program.path_memo_result_read_plan(function, instruction))
            .filter(|plan| plan.variable == definition.key)
        else {
            return;
        };
        let place = PathMemoPlace {
            generation,
            variable: definition.key,
            character,
            indices: indices.to_vec(),
        };
        let mut active = self.active_path_memo.borrow_mut();
        let Some(active) = active
            .as_mut()
            .filter(|active| active.fiber == fiber && active.frame == frame && active.valid)
        else {
            return;
        };
        let Some(dependency) = active.dependencies.iter().position(|dependency| {
            matches!(
                dependency,
                PathMemoDependency::Value {
                    place: observed,
                    ..
                } if *observed == place
            )
        }) else {
            return;
        };
        let has_cell_revision = active.dependencies.iter().any(|dependency| {
            matches!(
                dependency,
                PathMemoDependency::CellRevision {
                    generation: observed_generation,
                    variable,
                    ..
                } if *observed_generation == generation && *variable == definition.key
            )
        });
        if !active.repeated_value_dependencies.contains(&dependency)
            && !has_cell_revision
            && !active
                .mutations
                .iter()
                .any(|mutation| mutation.writes_cell(generation, definition.key))
        {
            active.pending_result_dependency = Some((instruction, dependency));
        }
    }

    pub(crate) fn confirm_path_memo_result_read(
        &self,
        fiber: FiberId,
        frame: FrameId,
        return_instruction: usize,
    ) {
        if self.active_path_memo_fiber.get() != Some(fiber) {
            return;
        }
        let mut active = self.active_path_memo.borrow_mut();
        let Some(active) = active
            .as_mut()
            .filter(|active| active.fiber == fiber && active.frame == frame && active.valid)
        else {
            return;
        };
        active.result_dependency = active
            .pending_result_dependency
            .filter(|(load_instruction, _)| {
                load_instruction.saturating_add(1) == return_instruction
            })
            .map(|(_, dependency)| dependency);
    }

    pub(crate) fn observe_path_memo_cell_revision(
        &self,
        fiber: FiberId,
        generation: GenerationId,
        variable: SymbolKey,
        revision: u64,
    ) -> bool {
        if !self.path_memo_can_observe(fiber) {
            return false;
        }
        let mut active = self.active_path_memo.borrow_mut();
        let Some(active) = active.as_mut().filter(|active| active.fiber == fiber) else {
            return false;
        };
        if !active.valid
            || active
                .mutations
                .iter()
                .any(|mutation| mutation.writes_cell(generation, variable))
        {
            return false;
        }
        if let Some(observed_revision) = active.dependencies.iter().find_map(|dependency| {
            let PathMemoDependency::CellRevision {
                generation: observed_generation,
                variable: observed_variable,
                revision,
            } = dependency
            else {
                return None;
            };
            (*observed_generation == generation && *observed_variable == variable)
                .then_some(*revision)
        }) {
            if observed_revision != revision {
                active.valid = false;
                return false;
            }
            return true;
        }
        active.retained_bytes = active
            .retained_bytes
            .saturating_add(std::mem::size_of::<PathMemoDependency>());
        active.dependencies.push(PathMemoDependency::CellRevision {
            generation,
            variable,
            revision,
        });
        enforce_path_memo_limits(active);
        active.valid
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe_path_memo_range_read(
        &self,
        fiber: FiberId,
        generation: GenerationId,
        definition: &erabasic_bytecode::BytecodeGlobal,
        character: usize,
        implicit_target: bool,
        start: usize,
        values: &[VmValue],
    ) {
        if !self.path_memo_can_observe(fiber) {
            return;
        }
        for (offset, value) in values.iter().enumerate() {
            self.observe_path_memo_read(
                fiber,
                generation,
                definition,
                character,
                implicit_target,
                &[u64::try_from(start.saturating_add(offset)).unwrap_or(u64::MAX)],
                value,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe_path_memo_write(
        &self,
        fiber: FiberId,
        generation: GenerationId,
        definition: &erabasic_bytecode::BytecodeGlobal,
        character: usize,
        implicit_target: bool,
        indices: &[u64],
        value: &VmValue,
    ) {
        if definition.storage == BytecodeStorage::FunctionLocal {
            return;
        }
        if !self.path_memo_can_observe(fiber) {
            return;
        }
        if implicit_target && !self.observe_path_memo_target_identity(fiber, generation, character)
        {
            return;
        }
        let mut active = self.active_path_memo.borrow_mut();
        let Some(active) = active.as_mut().filter(|active| active.fiber == fiber) else {
            return;
        };
        if !active.valid {
            return;
        }
        if active
            .dependencies
            .iter()
            .any(|dependency| dependency.observes_cell_revision(generation, definition.key))
        {
            active.valid = false;
            return;
        }
        active.retained_bytes = active
            .retained_bytes
            .saturating_add(retained_value_bytes(value))
            .saturating_add(indices.len().saturating_mul(std::mem::size_of::<u64>()));
        active.mutations.push(PathMemoMutation::Write {
            place: PathMemoPlace {
                generation,
                variable: definition.key,
                character,
                indices: indices.to_vec(),
            },
            value: value.clone(),
        });
        enforce_path_memo_limits(active);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe_path_memo_fill(
        &self,
        fiber: FiberId,
        generation: GenerationId,
        definition: &erabasic_bytecode::BytecodeGlobal,
        character: usize,
        implicit_target: bool,
        start: usize,
        end: usize,
        value: &VmValue,
    ) {
        if definition.storage == BytecodeStorage::FunctionLocal {
            return;
        }
        if !self.path_memo_can_observe(fiber) {
            return;
        }
        if implicit_target && !self.observe_path_memo_target_identity(fiber, generation, character)
        {
            return;
        }
        let mut active = self.active_path_memo.borrow_mut();
        let Some(active) = active.as_mut().filter(|active| active.fiber == fiber) else {
            return;
        };
        if !active.valid {
            return;
        }
        if active
            .dependencies
            .iter()
            .any(|dependency| dependency.observes_cell_revision(generation, definition.key))
        {
            active.valid = false;
            return;
        }
        active.retained_bytes = active
            .retained_bytes
            .saturating_add(retained_value_bytes(value));
        active.mutations.push(PathMemoMutation::Fill {
            generation,
            variable: definition.key,
            character,
            start,
            end,
            value: value.clone(),
        });
        enforce_path_memo_limits(active);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe_path_memo_replace(
        &self,
        fiber: FiberId,
        generation: GenerationId,
        definition: &erabasic_bytecode::BytecodeGlobal,
        character: usize,
        implicit_target: bool,
        values: &[VmValue],
    ) {
        if definition.storage == BytecodeStorage::FunctionLocal {
            return;
        }
        if !self.path_memo_can_observe(fiber) {
            return;
        }
        if implicit_target && !self.observe_path_memo_target_identity(fiber, generation, character)
        {
            return;
        }
        let mut active = self.active_path_memo.borrow_mut();
        let Some(active) = active.as_mut().filter(|active| active.fiber == fiber) else {
            return;
        };
        if !active.valid {
            return;
        }
        if active
            .dependencies
            .iter()
            .any(|dependency| dependency.observes_cell_revision(generation, definition.key))
        {
            active.valid = false;
            return;
        }
        active.retained_bytes = values
            .iter()
            .fold(active.retained_bytes, |retained, value| {
                retained.saturating_add(retained_value_bytes(value))
            });
        active.mutations.push(PathMemoMutation::Replace {
            generation,
            variable: definition.key,
            character,
            values: values.to_vec(),
        });
        enforce_path_memo_limits(active);
    }

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

fn path_memo_mutation_retained_bytes(mutation: &PathMemoMutation) -> usize {
    match mutation {
        PathMemoMutation::Write { place, value } => retained_value_bytes(value).saturating_add(
            place
                .indices
                .len()
                .saturating_mul(std::mem::size_of::<u64>()),
        ),
        PathMemoMutation::Fill { value, .. } => retained_value_bytes(value),
        PathMemoMutation::Replace { values, .. } => {
            values.iter().map(retained_value_bytes).sum::<usize>()
        }
    }
}

fn path_memo_entries<'a>(
    cache: &'a PathMemoCache,
    head: &PathMemoHead,
    arguments: &[VmValue],
) -> Option<&'a [Arc<PathMemoEntry>]> {
    cache
        .get(head)
        .and_then(|paths| paths.get(arguments))
        .map(Vec::as_slice)
}

fn path_memo_dependencies_match(
    generations: &BTreeMap<GenerationId, Arc<ProgramGeneration>>,
    memory: &Memory,
    entry: &PathMemoEntry,
) -> bool {
    entry
        .dependencies
        .iter()
        .enumerate()
        .all(|(index, dependency)| {
            if entry.result_dependency == Some(index) {
                return true;
            }
            match dependency {
                PathMemoDependency::Value { place, value } => {
                    let Some(program) = generations.get(&place.generation) else {
                        return false;
                    };
                    let Some(definition) = program.global(place.variable) else {
                        return false;
                    };
                    memory
                        .cell(place.generation, definition, place.character)
                        .and_then(|cell| cell.read(&place.indices).ok())
                        .is_some_and(|observed| observed.eq(value))
                }
                PathMemoDependency::CellRevision {
                    generation,
                    variable,
                    revision,
                } => generations
                    .get(generation)
                    .and_then(|program| program.global(*variable))
                    .and_then(|definition| memory.cell(*generation, definition, 0))
                    .is_some_and(|cell| cell.revision() == *revision),
                PathMemoDependency::TargetIdentity {
                    generation,
                    character,
                } => {
                    generations.get(generation).map_or(0, |program| {
                        memory
                            .target_character_from_definition(program.target_global(), *generation)
                    }) == *character
                }
            }
        })
}

fn read_path_memo_place(
    generations: &BTreeMap<GenerationId, Arc<ProgramGeneration>>,
    memory: &Memory,
    place: &PathMemoPlace,
) -> Option<VmValue> {
    let definition = generations.get(&place.generation)?.global(place.variable)?;
    memory
        .cell(place.generation, definition, place.character)?
        .read(&place.indices)
        .ok()
}

fn replay_path_memo_mutation(
    generations: &BTreeMap<GenerationId, Arc<ProgramGeneration>>,
    memory: &mut Memory,
    mutation: &PathMemoMutation,
) -> Result<(), VmError> {
    let (generation, variable, character) = mutation.cell_key();
    let definition = generations
        .get(&generation)
        .and_then(|program| program.global(variable))
        .ok_or_else(|| VmError::InvalidState("path memo variable is missing".into()))?;
    let storage = definition.storage;
    let cell = memory
        .cell_mut(generation, definition.key, storage, character)
        .ok_or_else(|| VmError::InvalidState("path memo storage is missing".into()))?;
    apply_path_memo_mutation(cell, mutation)
}

pub(crate) fn path_memo_cache_usage(cache: &PathMemoCache) -> (usize, usize) {
    cache.values().flat_map(|paths| paths.values()).fold(
        (0_usize, 0_usize),
        |(key_count, retained_bytes), entries| {
            (
                key_count.saturating_add(1),
                entries.iter().fold(retained_bytes, |bytes, entry| {
                    bytes.saturating_add(entry.retained_bytes)
                }),
            )
        },
    )
}

fn apply_path_memo_mutation(
    cell: &mut VariableCell,
    mutation: &PathMemoMutation,
) -> Result<(), VmError> {
    match mutation {
        PathMemoMutation::Write { place, value } => cell
            .write(&place.indices, value.clone())
            .map_err(VmError::InvalidState),
        PathMemoMutation::Fill {
            start, end, value, ..
        } => cell
            .fill_range(*start, *end, value.clone())
            .map_err(VmError::InvalidState),
        PathMemoMutation::Replace { values, .. } => cell
            .replace_values(values.clone())
            .map_err(VmError::InvalidState),
    }
}

fn retained_value_bytes(value: &VmValue) -> usize {
    std::mem::size_of::<VmValue>().saturating_add(match value {
        VmValue::String(value) => value.len(),
        VmValue::Integer(_) => 0,
        VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => MAX_RETAINED_BYTES,
    })
}

fn enforce_path_memo_limits(active: &mut ActivePathMemo) {
    if active.dependencies.len() > MAX_DEPENDENCIES
        || active.mutations.len() > MAX_MUTATIONS
        || active.retained_bytes > MAX_RETAINED_BYTES
    {
        active.valid = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        HostCallRequest, HostCallResult, HostReady, ImmediateHostCall, ImmediateHostCallResult,
        NativeServiceRegistry, RunBudget, VmEvent, VmHost,
    };
    use erabasic_analyzer::{
        AnalysisInput, AnalyzerOptions, ExtensionRegistry, ProjectSource, SourcePayload,
        analyze_project,
    };
    use erabasic_compiler::{CompilerOptions, compile_project, default_host_registry};
    use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};
    use erabasic_validator::{ValidationContext, validate_bytecode};

    struct RejectHost;

    impl VmHost for RejectHost {
        fn call(&mut self, _request: HostCallRequest) -> HostCallResult {
            HostCallResult::Error("unexpected host call".into())
        }
    }

    #[derive(Default)]
    struct PureTextHost {
        calls: usize,
        safe: bool,
    }

    impl VmHost for PureTextHost {
        fn path_memo_safe(&self, import: &erabasic_bytecode::RuntimeImport) -> bool {
            self.safe && import.name.eq_ignore_ascii_case("HTML_TOPLAINTEXT")
        }

        fn call_immediate(&mut self, request: ImmediateHostCall<'_>) -> ImmediateHostCallResult {
            if !request
                .normalized_name
                .eq_ignore_ascii_case("HTML_TOPLAINTEXT")
            {
                return ImmediateHostCallResult::Unsupported;
            }
            self.calls += 1;
            ImmediateHostCallResult::Ready(HostReady {
                value: Some(VmValue::String("plain".into())),
                writes: Vec::new(),
            })
        }

        fn call(&mut self, _request: HostCallRequest) -> HostCallResult {
            HostCallResult::Error("unexpected deferred host call".into())
        }
    }

    fn compile_vm(source: &str) -> (Vm, Arc<BytecodeArtifact>) {
        compile_vm_with_profile(source, erabasic_compat::CompatibilityProfileId::EmueraEm)
    }

    fn compile_vm_with_profile(
        source: &str,
        profile: erabasic_compat::CompatibilityProfileId,
    ) -> (Vm, Arc<BytecodeArtifact>) {
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
            &AnalyzerOptions {
                compatibility: erabasic_compat::CompatibilityIdentity::for_profile(profile),
                ..AnalyzerOptions::analysis_mode()
            },
            &ExtensionRegistry::default(),
        );
        let compilation = compile_project(
            analysis.project.as_ref().expect("analyzed project"),
            &CompilerOptions::default(),
            &default_host_registry(),
            None,
        );
        let artifact = Arc::new(
            compilation
                .artifact
                .unwrap_or_else(|| panic!("{:#?}", compilation.diagnostics)),
        );
        let validation = validate_bytecode(
            artifact.as_ref().clone().into_unvalidated(),
            &ValidationContext::for_artifact(&artifact),
        );
        (
            Vm::new(
                validation.value.expect("validated artifact"),
                VmConfig::default(),
            ),
            artifact,
        )
    }

    fn cached_entry(retained_bytes: usize) -> Arc<PathMemoEntry> {
        Arc::new(PathMemoEntry {
            dependencies: Vec::new(),
            safe_natives: Vec::new(),
            safe_hosts: Vec::new(),
            mutation_groups: Vec::new(),
            result: VmValue::Integer(0),
            result_dependency: None,
            body_instructions: 1,
            backward_branches: 0,
            retained_bytes,
        })
    }

    #[test]
    fn path_memo_cache_borrows_value_arguments_without_weakening_equality() {
        let head = PathMemoHead {
            generation: GenerationId(1),
            function: SymbolKey::derive("test", b"function"),
        };
        let stored = vec![VmValue::String("same".into()), VmValue::Integer(7)];
        let mut cache = PathMemoCache::new();
        cache
            .entry(head)
            .or_default()
            .insert(stored, vec![cached_entry(1)]);

        let equal = [VmValue::String("same".into()), VmValue::Integer(7)];
        assert!(path_memo_entries(&cache, &head, &equal).is_some());
        assert!(
            path_memo_entries(
                &cache,
                &head,
                &[VmValue::Integer(7), VmValue::String("same".into())]
            )
            .is_none(),
            "argument order remains significant"
        );
        assert!(
            path_memo_entries(&cache, &head, &equal[..1]).is_none(),
            "argument length remains significant"
        );
        assert!(
            path_memo_entries(
                &cache,
                &head,
                &[VmValue::String("same".into()), VmValue::String("7".into())]
            )
            .is_none(),
            "integer and string values remain distinct"
        );
        assert!(Vm::path_memo_head(GenerationId(1), head.function, &equal).is_some());
        assert!(
            Vm::path_memo_head(
                GenerationId(1),
                head.function,
                &[VmValue::IntegerPlace(Box::default())]
            )
            .is_none(),
            "place arguments must be rejected before probing the cache"
        );
    }

    #[test]
    fn path_memo_clear_and_generation_reclaim_keep_usage_exact() {
        let (mut vm, _) = compile_vm("@SYSTEM_TITLE\nRETURN\n");
        let retained_generation = vm.current_generation;
        let obsolete_generation = GenerationId(retained_generation.0 + 1);
        let program = vm
            .generations
            .get(&retained_generation)
            .expect("current generation")
            .clone();
        vm.generations.insert(obsolete_generation, program);
        let function = SymbolKey::derive("test", b"function");
        vm.path_memo_cache
            .entry(PathMemoHead {
                generation: retained_generation,
                function,
            })
            .or_default()
            .insert(vec![VmValue::Integer(1)], vec![cached_entry(11)]);
        vm.path_memo_cache
            .entry(PathMemoHead {
                generation: obsolete_generation,
                function,
            })
            .or_default()
            .insert(
                vec![VmValue::String("obsolete".into())],
                vec![cached_entry(13), cached_entry(17)],
            );
        (vm.path_memo_key_count, vm.path_memo_retained_bytes) =
            path_memo_cache_usage(&vm.path_memo_cache);
        assert_eq!(
            (vm.path_memo_key_count, vm.path_memo_retained_bytes),
            (2, 41)
        );

        vm.reclaim_generations();
        assert_eq!(
            (vm.path_memo_key_count, vm.path_memo_retained_bytes),
            (1, 11)
        );
        assert!(
            vm.path_memo_cache
                .keys()
                .all(|head| head.generation == retained_generation)
        );

        vm.clear_path_memo_cache();
        assert!(vm.path_memo_cache.is_empty());
        assert_eq!(
            (vm.path_memo_key_count, vm.path_memo_retained_bytes),
            (0, 0)
        );
    }

    #[test]
    fn dynamic_path_memo_replays_an_explicit_character_parameter() {
        let (mut vm, artifact) = compile_vm(
            "@SYSTEM_TITLE\n\
             ADDVOIDCHARA\nADDVOIDCHARA\n\
             RESULT:10 = DYNAMIC_SET(0, 7)\n\
             CFLAG:1:5 = 0\n\
             RESULT:11 = DYNAMIC_SET(0, 7)\nRETURN RESULT\n\
             @DYNAMIC_SET, ARG, ARG:1\n#FUNCTION\n\
             CALLFORMF TARGET_{ARG}, ARG:1\nRETURNF RESULT\n\
             @TARGET_0(CFLAG:1:5)\n#FUNCTION\nRETURNF 1\n",
        );
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .expect("SYSTEM_TITLE")
            .key;
        let cflag = artifact
            .globals
            .iter()
            .find(|global| global.name == "CFLAG")
            .expect("CFLAG")
            .key;
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        vm.spawn_entry(entry, Vec::new()).expect("spawn entry");
        let report = vm.run_slice(&mut RejectHost, &mut natives, RunBudget::default());

        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{:#?}",
            report.events
        );
        assert!(
            vm.path_memo_replays > 0,
            "the second call must physically replay"
        );
        assert_eq!(
            vm.read_variable(cflag, &[5], Some(1)),
            Ok(VmValue::Integer(7))
        );
        assert_eq!(
            vm.read_variable(cflag, &[5], Some(0)),
            Ok(VmValue::Integer(0))
        );
    }

    #[test]
    fn path_memo_refreshes_a_unique_tail_result_read() {
        let (mut vm, artifact) = compile_vm(
            "@SYSTEM_TITLE\n\
             FLAG:0 = 10\n\
             RESULT:10 = REFRESH_RESULT(0)\n\
             FLAG:0 = 20\n\
             RESULT:11 = REFRESH_RESULT(0)\nRETURN RESULT\n\
             @REFRESH_RESULT, ARG\n#FUNCTION\n#DIM DYNAMIC OFFSET\n\
             SELECTCASE ARG\nCASE 0\n\
                 OFFSET = ARG + STRCOUNT(\"aaa\", \"a\") - 3\n\
                 RETURNF FLAG:OFFSET\n\
             CASEELSE\nRETURNF 0\nENDSELECT\n",
        );
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .expect("SYSTEM_TITLE")
            .key;
        let result = artifact
            .globals
            .iter()
            .find(|global| global.name == "RESULT")
            .expect("RESULT")
            .key;
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        vm.spawn_entry(entry, Vec::new()).expect("spawn entry");
        let report = vm.run_slice(&mut RejectHost, &mut natives, RunBudget::default());

        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{:#?}",
            report.events
        );
        assert_eq!(vm.path_memo_replays, 1);
        assert_eq!(
            (10..=11)
                .map(|index| vm.read_variable(result, &[index], None).unwrap())
                .collect::<Vec<_>>(),
            [10, 20].map(VmValue::Integer)
        );
    }

    #[test]
    fn path_memo_does_not_refresh_a_tail_place_read_twice() {
        let (mut vm, artifact) = compile_vm(
            "@SYSTEM_TITLE\n\
             FLAG:0 = 10\n\
             RESULT:10 = READ_TWICE(0)\n\
             FLAG:0 = 20\n\
             RESULT:11 = READ_TWICE(0)\nRETURN RESULT\n\
             @READ_TWICE, ARG\n#FUNCTION\n#DIM DYNAMIC DISCARD\n\
             DISCARD = FLAG:ARG\n\
             RETURNF FLAG:ARG\n",
        );
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .expect("SYSTEM_TITLE")
            .key;
        let result = artifact
            .globals
            .iter()
            .find(|global| global.name == "RESULT")
            .expect("RESULT")
            .key;
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        vm.spawn_entry(entry, Vec::new()).expect("spawn entry");
        let report = vm.run_slice(&mut RejectHost, &mut natives, RunBudget::default());

        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{:#?}",
            report.events
        );
        assert_eq!(vm.path_memo_replays, 0);
        assert_eq!(
            (10..=11)
                .map(|index| vm.read_variable(result, &[index], None).unwrap())
                .collect::<Vec<_>>(),
            [10, 20].map(VmValue::Integer)
        );
    }

    #[test]
    fn path_memo_result_refresh_still_validates_index_dependencies() {
        let (mut vm, artifact) = compile_vm(
            "@SYSTEM_TITLE\n\
             FLAG:0 = 10\nFLAG:1 = 20\nCOUNT = 0\n\
             RESULT:10 = READ_SELECTED()\n\
             COUNT = 1\n\
             RESULT:11 = READ_SELECTED()\nRETURN RESULT\n\
             @READ_SELECTED\n#FUNCTION\n#DIM DYNAMIC INDEX\n\
             INDEX = COUNT\n\
             RETURNF FLAG:INDEX\n",
        );
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .expect("SYSTEM_TITLE")
            .key;
        let result = artifact
            .globals
            .iter()
            .find(|global| global.name == "RESULT")
            .expect("RESULT")
            .key;
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        vm.spawn_entry(entry, Vec::new()).expect("spawn entry");
        let report = vm.run_slice(&mut RejectHost, &mut natives, RunBudget::default());

        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{:#?}",
            report.events
        );
        assert_eq!(vm.path_memo_replays, 0);
        assert_eq!(
            (10..=11)
                .map(|index| vm.read_variable(result, &[index], None).unwrap())
                .collect::<Vec<_>>(),
            [10, 20].map(VmValue::Integer)
        );
    }

    #[test]
    fn path_memo_only_crosses_hosts_with_a_current_purity_guarantee() {
        fn run(safe: bool) -> (usize, u64, Vec<VmValue>) {
            let (mut vm, artifact) = compile_vm(
                "@SYSTEM_TITLE\n\
                 RESULTS:10 '= PURE_TEXT(\"<b>x</b>\")\n\
                 RESULTS:11 '= PURE_TEXT(\"<b>x</b>\")\nRETURN\n\
                 @PURE_TEXT, ARGS\n#FUNCTIONS\n\
                 RETURNF HTML_TOPLAINTEXT(ARGS)\n",
            );
            let entry = artifact
                .functions
                .iter()
                .find(|function| function.name == "SYSTEM_TITLE")
                .expect("SYSTEM_TITLE")
                .key;
            let results = artifact
                .globals
                .iter()
                .find(|global| global.name == "RESULTS")
                .expect("RESULTS")
                .key;
            let mut natives = NativeServiceRegistry::for_artifact(&artifact);
            let mut host = PureTextHost { calls: 0, safe };
            vm.spawn_entry(entry, Vec::new()).expect("spawn entry");
            let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{:#?}",
                report.events
            );
            (
                host.calls,
                vm.path_memo_replays,
                (10..=11)
                    .map(|index| vm.read_variable(results, &[index], None).unwrap())
                    .collect(),
            )
        }

        let safe = run(true);
        assert_eq!(safe.0, 1, "the second pure Host call should be replayed");
        assert_eq!(safe.1, 1);
        assert_eq!(
            safe.2,
            ["plain", "plain"].map(|value| VmValue::String(value.into()))
        );

        let unsafe_host = run(false);
        assert_eq!(
            unsafe_host.0, 2,
            "unclassified Host calls remain boundaries"
        );
        assert_eq!(unsafe_host.1, 0);
        assert_eq!(unsafe_host.2, safe.2);
    }

    #[test]
    fn full_cell_replay_keeps_only_the_canonical_final_snapshot() {
        let (mut vm, artifact) = compile_vm(
            "@SYSTEM_TITLE\n\
             RESULT:10 = DYNAMIC_RESET(0)\n\
             RESULT:11 = DYNAMIC_RESET(0)\nRETURN\n\
             @DYNAMIC_RESET, ARG\n#FUNCTION\n\
             CALLFORMF RESET_{ARG}\nRETURNF RESULT\n\
             @RESET_0\n#FUNCTION\n#LOCALSSIZE 32\n\
             VARSET LOCALS\nLOCALS:5 = \"done\"\nRETURNF 7\n",
        );
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .expect("SYSTEM_TITLE")
            .key;
        let locals = artifact
            .globals
            .iter()
            .find(|global| global.name == "LOCALS" && global.dimensions == [32])
            .expect("resized LOCALS")
            .key;
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        vm.spawn_entry(entry, Vec::new()).expect("spawn entry");
        let report = vm.run_slice(&mut RejectHost, &mut natives, RunBudget::default());

        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{:#?}",
            report.events
        );
        assert_eq!(vm.path_memo_replays, 1);
        let group = vm
            .path_memo_cache
            .values()
            .flat_map(|paths| paths.values())
            .flatten()
            .flat_map(|entry| &entry.mutation_groups)
            .find(|group| group.variable == locals)
            .expect("LOCALS mutation group");
        assert!(group.final_cell.is_some());
        assert!(
            group.mutations.is_empty(),
            "the final snapshot replaces the redundant mutation log"
        );
    }

    #[test]
    fn dynamic_call_warnings_remain_path_memo_boundaries_after_site_deduplication() {
        let (mut vm, artifact) = compile_vm_with_profile(
            "@SYSTEM_TITLE\nRESULT:10 = WRAPPER()\nRESULT:11 = WRAPPER()\nRESULT:12 = WRAPPER()\nRETURN\n\
             @WRAPPER\n#FUNCTION\nCALLFORMF TARGET_0, 1, EXTRA()\nRETURNF RESULT\n\
             @TARGET_0(ARG)\n#FUNCTION\nFLAG:1 = 7\nRETURNF ARG\n\
             @EXTRA\n#FUNCTION\nFLAG:0 += 1\nRETURNF 99\n",
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        );
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .unwrap()
            .key;
        let flag = artifact
            .globals
            .iter()
            .find(|variable| variable.name == "FLAG")
            .unwrap()
            .key;
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        vm.spawn_entry(entry, Vec::new()).unwrap();
        let report = vm.run_slice(&mut RejectHost, &mut natives, RunBudget::default());
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        assert_eq!(report.events.iter().filter(|event| matches!(event,
            VmEvent::Diagnostic { code, origin, .. } if code == "compat.call.excess_arguments" && origin.function_name == "WRAPPER"
        )).count(), 1, "{report:?}");
        assert_eq!(vm.path_memo_replays, 0);
        assert!(vm.path_memo_cache.is_empty());
        assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(0)));
        assert_eq!(vm.read_variable(flag, &[1], None), Ok(VmValue::Integer(7)));
    }

    #[test]
    fn over_budget_full_cell_groups_do_not_create_a_path_memo_entry() {
        let value = "x".repeat(512);
        let source = format!(
            "@SYSTEM_TITLE\n\
             RESULT:10 = DYNAMIC_GET(0)\n\
             RESULT:11 = DYNAMIC_GET(0)\nRETURN RESULT\n\
             @DYNAMIC_GET, ARG\n#FUNCTION\n\
             CALLFORMF TARGET_{{ARG}}\nRETURNF RESULT\n\
             @TARGET_0\n#FUNCTION\n\
             VARSET RESULTS, \"{value}\"\n\
             VARSET LOCALS, \"{value}\"\nRETURNF 1\n"
        );
        let (mut vm, artifact) = compile_vm(&source);
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .expect("SYSTEM_TITLE")
            .key;
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        vm.spawn_entry(entry, Vec::new()).expect("spawn entry");
        let report = vm.run_slice(&mut RejectHost, &mut natives, RunBudget::default());

        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{:#?}",
            report.events
        );
        assert_eq!(vm.path_memo_replays, 0);
        assert!(
            vm.path_memo_cache.is_empty(),
            "the combined final-cell snapshots exceed the per-entry retained-memory budget"
        );
    }
}
