#[allow(clippy::wildcard_imports)]
use super::*;

const MAX_PATH_MEMO_KEYS: usize = 8_192;
const MAX_PATHS_PER_KEY: usize = 4;
const MAX_DEPENDENCIES: usize = 128;
const MAX_MUTATIONS: usize = 512;
const MAX_RETAINED_BYTES: usize = 64 * 1024;
const MAX_CACHE_RETAINED_BYTES: usize = 16 * 1024 * 1024;

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
            let Some(definition) = program.global(parameter.key) else {
                self.invalidate_path_memo(fiber);
                return;
            };
            if definition.storage != BytecodeStorage::FunctionLocal {
                self.observe_path_memo_write(
                    fiber,
                    generation,
                    definition,
                    &parameter.indices,
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

    pub(crate) fn path_memo_key(
        generation: GenerationId,
        function: SymbolKey,
        arguments: &[VmValue],
    ) -> Option<PathMemoBaseKey> {
        Some(PathMemoBaseKey {
            generation,
            function,
            arguments: arguments
                .iter()
                .map(MemoValue::from_vm)
                .collect::<Option<Vec<_>>>()?,
        })
    }

    pub(crate) fn begin_path_memo(
        &self,
        fiber: &Fiber,
        frame: FrameId,
        function: &BytecodeFunction,
        key: PathMemoBaseKey,
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
        let retained_bytes = key.arguments.iter().fold(0_usize, |retained, argument| {
            retained.saturating_add(retained_memo_value_bytes(argument))
        });
        *self.active_path_memo.borrow_mut() = Some(ActivePathMemo {
            fiber: fiber.id,
            frame,
            key,
            dependencies: Vec::new(),
            mutations: Vec::new(),
            retained_bytes,
            body_instructions: 0,
            maximum_body_instructions,
            backward_branches_before: fiber.backward_branches_without_progress,
            skip_call_instruction: true,
            valid: maximum_body_instructions != 0 && retained_bytes <= MAX_RETAINED_BYTES,
        });
        self.active_path_memo_fiber.set(Some(fiber.id));
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
            Opcode::ResolveFunction
                | Opcode::InvokeDynamic
                | Opcode::JumpDynamicLabel
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
            "escape"
                | "findelement"
                | "findlastelement"
                | "isnumeric"
                | "split"
                | "toint"
                | "varset"
        ) {
            self.invalidate_path_memo(fiber);
        }
    }

    pub(crate) fn observe_path_memo_read(
        &self,
        fiber: FiberId,
        generation: GenerationId,
        definition: &erabasic_bytecode::BytecodeGlobal,
        indices: &[u64],
        value: &VmValue,
    ) {
        if definition.storage == BytecodeStorage::FunctionLocal {
            return;
        }
        if definition.storage == BytecodeStorage::Character {
            self.invalidate_path_memo(fiber);
            return;
        }
        if !self.path_memo_can_observe(fiber) {
            return;
        }
        let place = PathMemoPlace {
            generation,
            variable: definition.key,
            indices: indices.to_vec(),
        };
        let mut active = self.active_path_memo.borrow_mut();
        let Some(active) = active.as_mut().filter(|active| active.fiber == fiber) else {
            return;
        };
        if !active.valid {
            return;
        }
        if active
            .mutations
            .iter()
            .any(|mutation| mutation.writes(&place))
            || active.dependencies.iter().any(|dependency| {
                matches!(
                    dependency,
                    PathMemoDependency::Value {
                        place: observed,
                        ..
                    } if *observed == place
                )
            })
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

    pub(crate) fn observe_path_memo_range_read(
        &self,
        fiber: FiberId,
        generation: GenerationId,
        definition: &erabasic_bytecode::BytecodeGlobal,
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
                &[u64::try_from(start.saturating_add(offset)).unwrap_or(u64::MAX)],
                value,
            );
        }
    }

    pub(crate) fn observe_path_memo_write(
        &self,
        fiber: FiberId,
        generation: GenerationId,
        definition: &erabasic_bytecode::BytecodeGlobal,
        indices: &[u64],
        value: &VmValue,
    ) {
        if definition.storage == BytecodeStorage::FunctionLocal {
            return;
        }
        if definition.storage == BytecodeStorage::Character {
            self.invalidate_path_memo(fiber);
            return;
        }
        if !self.path_memo_can_observe(fiber) {
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
        start: usize,
        end: usize,
        value: &VmValue,
    ) {
        if definition.storage == BytecodeStorage::FunctionLocal {
            return;
        }
        if definition.storage == BytecodeStorage::Character {
            self.invalidate_path_memo(fiber);
            return;
        }
        if !self.path_memo_can_observe(fiber) {
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
            start,
            end,
            value: value.clone(),
        });
        enforce_path_memo_limits(active);
    }

    pub(crate) fn observe_path_memo_replace(
        &self,
        fiber: FiberId,
        generation: GenerationId,
        definition: &erabasic_bytecode::BytecodeGlobal,
        values: &[VmValue],
    ) {
        if definition.storage == BytecodeStorage::FunctionLocal {
            return;
        }
        if definition.storage == BytecodeStorage::Character {
            self.invalidate_path_memo(fiber);
            return;
        }
        if !self.path_memo_can_observe(fiber) {
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
            values: values.to_vec(),
        });
        enforce_path_memo_limits(active);
    }

    pub(crate) fn try_replay_path_memo(
        &mut self,
        fiber: &mut Fiber,
        key: &PathMemoBaseKey,
        remaining_quantum: u32,
        remaining_instructions: u64,
    ) -> Result<Option<(VmValue, u64)>, VmError> {
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
        let Some(entry) = self.path_memo_cache.get(key).and_then(|entries| {
            entries
                .iter()
                .find(|entry| {
                    entry.body_instructions.saturating_add(1) <= u64::from(remaining_quantum)
                        && entry.body_instructions.saturating_add(1) <= remaining_instructions
                        && self.path_memo_dependencies_match(entry)
                })
                .cloned()
        }) else {
            return Ok(None);
        };
        for mutation in &entry.mutations {
            self.replay_path_memo_mutation(mutation)?;
        }
        fiber.backward_branches_without_progress = fiber
            .backward_branches_without_progress
            .saturating_add(entry.backward_branches);
        fiber.consecutive_budget_exhaustions = 0;
        Ok(Some((entry.result.clone(), entry.body_instructions)))
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
        if self.path_memo_cache.len() >= MAX_PATH_MEMO_KEYS {
            self.path_memo_cache.clear();
            self.path_memo_retained_bytes = 0;
        }
        if self
            .path_memo_retained_bytes
            .saturating_add(active.retained_bytes)
            > MAX_CACHE_RETAINED_BYTES
        {
            self.path_memo_cache.clear();
            self.path_memo_retained_bytes = 0;
        }
        let entries = self.path_memo_cache.entry(active.key).or_default();
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
            mutations: active.mutations,
            result: result.clone(),
            body_instructions: active.body_instructions,
            backward_branches,
            retained_bytes: active.retained_bytes,
        }));
    }

    fn path_memo_dependencies_match(&self, entry: &PathMemoEntry) -> bool {
        entry
            .dependencies
            .iter()
            .all(|dependency| match dependency {
                PathMemoDependency::Value { place, value } => {
                    let Some(program) = self.generations.get(&place.generation) else {
                        return false;
                    };
                    let Some(definition) = program.global(place.variable) else {
                        return false;
                    };
                    self.memory
                        .cell(place.generation, definition, 0)
                        .and_then(|cell| cell.read(&place.indices).ok())
                        .is_some_and(|observed| observed.eq(value))
                }
                PathMemoDependency::CellRevision {
                    generation,
                    variable,
                    revision,
                } => self
                    .generations
                    .get(generation)
                    .and_then(|program| program.global(*variable))
                    .and_then(|definition| self.memory.cell(*generation, definition, 0))
                    .is_some_and(|cell| cell.revision() == *revision),
            })
    }

    fn replay_path_memo_mutation(&mut self, mutation: &PathMemoMutation) -> Result<(), VmError> {
        let (generation, variable) = match mutation {
            PathMemoMutation::Write { place, .. } => (place.generation, place.variable),
            PathMemoMutation::Fill {
                generation,
                variable,
                ..
            }
            | PathMemoMutation::Replace {
                generation,
                variable,
                ..
            } => (*generation, *variable),
        };
        let definition = self
            .generations
            .get(&generation)
            .and_then(|program| program.global(variable))
            .ok_or_else(|| VmError::InvalidState("path memo variable is missing".into()))?;
        let storage = definition.storage;
        let cell = self
            .memory
            .cell_mut(generation, definition.key, storage, 0)
            .ok_or_else(|| VmError::InvalidState("path memo storage is missing".into()))?;
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
}

fn retained_value_bytes(value: &VmValue) -> usize {
    std::mem::size_of::<VmValue>().saturating_add(match value {
        VmValue::String(value) => value.len(),
        VmValue::Integer(_) => 0,
        VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => MAX_RETAINED_BYTES,
    })
}

fn retained_memo_value_bytes(value: &MemoValue) -> usize {
    std::mem::size_of::<MemoValue>().saturating_add(match value {
        MemoValue::String(value) => value.len(),
        MemoValue::Integer(_) => 0,
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
