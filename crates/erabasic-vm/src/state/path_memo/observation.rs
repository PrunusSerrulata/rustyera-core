#[allow(clippy::wildcard_imports)]
use super::*;
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
}
