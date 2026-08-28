#[allow(clippy::wildcard_imports)]
use super::super::*;

impl Vm {
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(in crate::interpreter) fn dispatch_calls(
        &mut self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        opcode: Opcode,
        host: &mut impl VmHost,
        natives: &mut NativeServiceRegistry,
        host_calls: &mut u32,
        policy: ExecutionPolicy,
    ) -> Result<Option<StepOutcome>, StepError> {
        fiber
            .frames
            .last()
            .ok_or_else(|| StepError::new(VmFaultCode::InvalidInstruction, "missing frame"))?;
        match opcode {
            Opcode::ResolveFunction => {
                let missing_target = read_u32(position.encoded.payload, 0)? as usize;
                let allow_missing = position.encoded.payload.get(4).copied() == Some(1);
                let method = position.encoded.payload.get(5).copied() == Some(1);
                let VmValue::String(name) =
                    pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?
                else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "dynamic function target must be a string",
                    ));
                };
                let generation = self
                    .generations
                    .get(&position.generation)
                    .expect("validated frame generation exists");
                let artifact = &generation.artifact;
                if let Some(target) = generation.function_by_name(&name) {
                    let kind_matches = if method {
                        target.kind == BytecodeFunctionKind::Method
                    } else {
                        target.kind != BytecodeFunctionKind::Method
                            && (target.kind != BytecodeFunctionKind::Event
                                || artifact.call_compatibility.allow_event_as_normal)
                    };
                    if !kind_matches {
                        if method && allow_missing {
                            fiber
                                .frames
                                .last_mut()
                                .expect("frame exists")
                                .stack
                                .push(VmValue::String(String::new()));
                            fiber.frames.last_mut().expect("frame exists").instruction =
                                missing_target;
                            return Ok(Some(StepOutcome::Continue));
                        }
                        return Err(StepError::new(
                            VmFaultCode::TypeMismatch,
                            format!("dynamic target {name} has an incompatible function kind"),
                        ));
                    }
                    let resolved_name = if target.name == name {
                        name
                    } else {
                        target.name.clone()
                    };
                    fiber
                        .frames
                        .last_mut()
                        .expect("frame exists")
                        .stack
                        .push(VmValue::String(resolved_name));
                } else if allow_missing {
                    fiber
                        .frames
                        .last_mut()
                        .expect("frame exists")
                        .stack
                        .push(VmValue::String(String::new()));
                    fiber.frames.last_mut().expect("frame exists").instruction = missing_target;
                } else {
                    return Err(StepError::new(
                        VmFaultCode::MissingSymbol,
                        format!("dynamic function {name} is missing"),
                    ));
                }
            }
            Opcode::InvokeDynamic => {
                let argument_count = read_u16(position.encoded.payload, 0)? as usize;
                let tail = position.encoded.payload.get(2).copied() == Some(1);
                // A tail call replaces the frame that owns an active trace, so there is no
                // matching return at which to complete that trace. Ordinary dynamic calls keep
                // their owner frame and can be observed safely.
                if tail {
                    self.invalidate_path_memo(fiber.id);
                }
                let new_frame = self.allocate_frame_id();
                let arguments = pop_arguments(
                    &mut fiber.frames.last_mut().expect("frame exists").stack,
                    argument_count,
                )?;
                let VmValue::String(name) =
                    pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?
                else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "resolved function target must be a string",
                    ));
                };
                let generation = self
                    .generations
                    .get(&position.generation)
                    .expect("validated frame generation exists");
                let artifact = &generation.artifact;
                let target = generation.function_by_name(&name).ok_or_else(|| {
                    StepError::new(VmFaultCode::MissingSymbol, "resolved function disappeared")
                })?;
                let mut arguments = arguments;
                for (parameter, argument) in target.parameters.iter().zip(&mut arguments) {
                    if parameter.by_reference {
                        continue;
                    }
                    let place = match argument {
                        VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => place.clone(),
                        _ => continue,
                    };
                    *argument = self.read_place(fiber, &place).map_err(map_vm_error)?;
                }
                let arguments =
                    prepare_dynamic_arguments(target, arguments, artifact.call_compatibility)
                        .map_err(map_vm_error)?;
                self.memory.ensure_function_statics(
                    position.generation,
                    target.key,
                    generation.function_statics(target.key),
                );
                bind_persistent_arguments(
                    &mut self.memory,
                    position.generation,
                    target,
                    generation,
                    &arguments,
                )
                .map_err(map_vm_error)?;
                self.observe_path_memo_arguments(
                    fiber.id,
                    position.generation,
                    target,
                    generation,
                    &arguments,
                );
                let event_context = fiber.frames.last().expect("frame exists").event_context
                    || target.kind == BytecodeFunctionKind::Event;
                let frame = make_frame(
                    new_frame,
                    position.generation,
                    target,
                    generation.function_locals(target.key),
                    arguments,
                    false,
                    event_context,
                );
                if tail {
                    *fiber.frames.last_mut().expect("frame exists") = frame;
                } else {
                    if fiber.frames.len() >= self.config.maximum_call_depth {
                        return Err(StepError::new(
                            VmFaultCode::ResourceLimit,
                            "maximum call depth exceeded",
                        ));
                    }
                    fiber.frames.push(frame);
                }
            }
            Opcode::Call | Opcode::CallNative | Opcode::CallHost => {
                let import_index = read_u32(position.encoded.payload, 0)? as usize;
                let argument_count = read_u16(position.encoded.payload, 4)? as usize;
                let new_frame = (opcode == Opcode::Call).then(|| self.allocate_frame_id());
                let generation = Arc::clone(
                    self.generations
                        .get(&position.generation)
                        .expect("validated frame generation exists"),
                );
                let artifact = &generation.artifact;
                let function = generation
                    .function(position.function)
                    .expect("validated function exists");
                let import = function.imports.get(import_index).cloned().ok_or_else(|| {
                    StepError::new(VmFaultCode::MissingSymbol, "call import is missing")
                })?;
                let arguments = pop_arguments(
                    &mut fiber.frames.last_mut().expect("frame exists").stack,
                    argument_count,
                )?;
                match (opcode, import.kind) {
                    (Opcode::Call, ImportKind::Function) => {
                        if fiber.frames.len() >= self.config.maximum_call_depth {
                            return Err(StepError::new(
                                VmFaultCode::ResourceLimit,
                                "maximum call depth exceeded",
                            ));
                        }
                        let target = generation.function(import.key).ok_or_else(|| {
                            StepError::new(VmFaultCode::MissingSymbol, "called function is missing")
                        })?;
                        if target.kind == BytecodeFunctionKind::Event
                            && !artifact.call_compatibility.allow_event_as_normal
                        {
                            return Err(StepError::new(
                                VmFaultCode::TypeMismatch,
                                format!(
                                    "event function {} cannot be called as an ordinary function",
                                    target.name
                                ),
                            ));
                        }
                        validate_arguments(target, &arguments).map_err(map_vm_error)?;
                        self.memory.ensure_function_statics(
                            position.generation,
                            target.key,
                            generation.function_statics(target.key),
                        );
                        bind_persistent_arguments(
                            &mut self.memory,
                            position.generation,
                            target,
                            &generation,
                            &arguments,
                        )
                        .map_err(map_vm_error)?;
                        self.observe_path_memo_arguments(
                            fiber.id,
                            position.generation,
                            target,
                            &generation,
                            &arguments,
                        );
                        let path_memo_active = self.path_memo_is_active_for(fiber.id);
                        if policy.allow_function_memo
                            && !path_memo_active
                            && let Some(value) = self.try_memoized_indexed_read(
                                fiber,
                                position.generation,
                                target.key,
                                &arguments,
                            )?
                        {
                            fiber
                                .frames
                                .last_mut()
                                .expect("caller frame exists")
                                .stack
                                .push(value);
                            return Ok(Some(StepOutcome::Continue));
                        }
                        let memo_key = (policy.allow_function_memo
                            && !path_memo_active
                            && usize::try_from(policy.remaining_quantum)
                                .ok()
                                .is_some_and(|remaining| target.code.len() < remaining))
                        .then(|| {
                            self.function_memo_key(position.generation, target.key, &arguments)
                        })
                        .flatten();
                        if let Some(entry) = memo_key
                            .as_ref()
                            .and_then(|key| self.function_memo_cache.get(key))
                            .cloned()
                        {
                            self.replay_function_memo_entry(position.generation, &entry)
                                .map_err(map_vm_error)?;
                            fiber
                                .frames
                                .last_mut()
                                .expect("caller frame exists")
                                .stack
                                .push(entry.result);
                            return Ok(Some(StepOutcome::Continue));
                        }
                        // The revision-keyed memo remains the cheapest first choice. A plan can
                        // still miss when its private scratch revision changes on every call, so
                        // let the value-validated path memo cover that case. Indexed getters are
                        // excluded because their first execution must warm the nested selector
                        // memo used by the dedicated fast path.
                        let path_memo_candidate = target.result.is_some()
                            && generation.memoized_indexed_read_plan(target.key).is_none();
                        let frame_id = new_frame.expect("function call reserved a frame id");
                        let path_memo_head = (policy.allow_function_memo
                            && !path_memo_active
                            && path_memo_candidate)
                            .then(|| {
                                Self::path_memo_head(position.generation, target.key, &arguments)
                            })
                            .flatten();
                        if let Some(path_memo_head) = path_memo_head.as_ref()
                            && let Some((value, body_instructions)) = self
                                .try_replay_path_memo(
                                    fiber,
                                    (path_memo_head, &arguments),
                                    host,
                                    natives,
                                    policy.remaining_quantum,
                                    policy.remaining_instructions,
                                )
                                .map_err(map_vm_error)?
                        {
                            fiber
                                .frames
                                .last_mut()
                                .expect("caller frame exists")
                                .stack
                                .push(value);
                            return Ok(Some(StepOutcome::BulkProgress(body_instructions)));
                        }
                        let event_context = fiber
                            .frames
                            .last()
                            .expect("caller frame exists")
                            .event_context
                            || target.kind == BytecodeFunctionKind::Event;
                        if let Some(path_memo_head) = path_memo_head {
                            self.begin_path_memo(
                                fiber,
                                frame_id,
                                target,
                                path_memo_head,
                                &arguments,
                                u64::from(policy.remaining_quantum.saturating_sub(1))
                                    .min(policy.remaining_instructions.saturating_sub(1)),
                            );
                        }
                        let frame = make_frame(
                            frame_id,
                            position.generation,
                            target,
                            generation.function_locals(target.key),
                            arguments,
                            target.result.is_some(),
                            event_context,
                        );
                        if let Some(key) = memo_key {
                            self.active_function_memos.insert(frame_id, key);
                        }
                        fiber.frames.push(frame);
                    }
                    (Opcode::CallNative, ImportKind::Native) => {
                        let target_index =
                            generation.native_import_index(import.key).ok_or_else(|| {
                                StepError::new(
                                    VmFaultCode::MissingSymbol,
                                    "native import is missing",
                                )
                            })?;
                        let target = &artifact
                            .native_imports
                            .get(target_index)
                            .ok_or_else(|| {
                                StepError::new(
                                    VmFaultCode::MissingSymbol,
                                    "native import is missing",
                                )
                            })?
                            .import;
                        let result_type = target.result;
                        let native_name = generation
                            .normalized_native_name(target_index)
                            .ok_or_else(|| {
                                StepError::new(
                                    VmFaultCode::MissingSymbol,
                                    "native import is missing",
                                )
                            })?;
                        // Registered pure Core natives are safe inside a path trace, and the trace
                        // records their keys so replay can reject a later service override.
                        // Unregistered interpreter-special natives use the conservative name
                        // policy. STRFORM can evaluate arbitrary script and is always a boundary.
                        if native_name == "strform" || natives.contains(import.key) {
                            if natives.path_memo_safe(import.key) && native_name != "strform" {
                                self.observe_path_memo_safe_native(fiber.id, import.key);
                            } else {
                                self.invalidate_path_memo(fiber.id);
                            }
                        } else {
                            self.observe_path_memo_native(fiber.id, native_name);
                        }
                        let mut rollback = None;
                        let ready = if native_name == "existmeth" {
                            if result_type != Some(BytecodeType::Integer)
                                || target.parameters != [BytecodeType::String]
                            {
                                return Err(StepError::new(
                                    VmFaultCode::InvalidInstruction,
                                    "EXISTMETH import signature is invalid",
                                ));
                            }
                            let [VmValue::String(name)] = arguments.as_slice() else {
                                return Err(StepError::new(
                                    VmFaultCode::TypeMismatch,
                                    "EXISTMETH expects one string",
                                ));
                            };
                            self.invalidate_path_memo(fiber.id);
                            NativeReady::value(VmValue::Integer(
                                crate::state::methods::exists_method(
                                    &generation,
                                    position.generation,
                                    name,
                                ),
                            ))
                        } else if native_name == "strform" {
                            if result_type != Some(BytecodeType::String) || arguments.len() != 1 {
                                return Err(StepError::new(
                                    VmFaultCode::InvalidInstruction,
                                    "STRFORM import signature is invalid",
                                ));
                            }
                            let value = arguments.first().and_then(|value| match value {
                                VmValue::String(value) => Some(value.as_str()),
                                _ => None,
                            });
                            let value = value.ok_or_else(|| {
                                StepError::new(
                                    VmFaultCode::TypeMismatch,
                                    "STRFORM expects a string",
                                )
                            })?;
                            begin_runtime_form(
                                self,
                                fiber,
                                natives,
                                position.generation,
                                position.function,
                                position.instruction,
                                value,
                            )?;
                            return Ok(Some(StepOutcome::DeferredNative));
                        } else if matches!(native_name, "initrand" | "dumprand") {
                            execute_random_place_transaction(
                                &mut self.memory,
                                position.generation,
                                artifact,
                                natives,
                                native_name,
                            )?;
                            NativeReady::default()
                        } else if native_name == "__mutate_integer" {
                            NativeReady::value(
                                execute_integer_mutation(self, fiber, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if matches!(native_name, "swap" | "swapvar") {
                            execute_swap_transaction(self, fiber, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if matches!(native_name, "setbit" | "clearbit" | "invertbit") {
                            execute_bit_mutation(self, fiber, native_name, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if native_name == "split" {
                            execute_split_transaction(self, fiber, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if native_name == "getnum" {
                            NativeReady::value(
                                execute_getnum(self, fiber, &arguments).map_err(map_vm_error)?,
                            )
                        } else if native_name == "erdname" {
                            NativeReady::value(
                                execute_erdname(self, fiber, &arguments).map_err(map_vm_error)?,
                            )
                        } else if native_name == "__indexbyname" {
                            NativeReady::value(
                                execute_index_by_name(self, fiber, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if native_name == "setvar" {
                            NativeReady::value(
                                execute_set_var(self, fiber, &arguments).map_err(map_vm_error)?,
                            )
                        } else if matches!(native_name, "getvar" | "getvars") {
                            NativeReady::value(
                                execute_get_var(self, fiber, &arguments, native_name == "getvars")
                                    .map_err(map_vm_error)?,
                            )
                        } else if native_name == "__encodetouni_result" {
                            execute_encode_to_uni_result(self, fiber, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if native_name == "strjoin" {
                            NativeReady::value(
                                execute_strjoin(self, fiber, &arguments).map_err(map_vm_error)?,
                            )
                        } else if matches!(native_name, "arrayremove" | "arrayshift" | "arraysort")
                        {
                            execute_array_mutation(self, fiber, native_name, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if native_name == "arraycopy" {
                            execute_array_copy(self, fiber, &arguments).map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if matches!(native_name, "varset" | "cvarset") {
                            execute_variable_fill(self, fiber, native_name, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if native_name == "arraymsort" {
                            NativeReady::value(
                                execute_array_multi_sort(self, fiber, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if native_name == "arraymsortex" {
                            NativeReady::value(
                                execute_array_multi_sort_ex(self, fiber, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if matches!(native_name, "findelement" | "findlastelement") {
                            NativeReady::value(
                                execute_find_element(
                                    self,
                                    fiber,
                                    native_name == "findlastelement",
                                    &arguments,
                                )
                                .map_err(map_vm_error)?,
                            )
                        } else if native_name == "regexpmatch" {
                            NativeReady::value(
                                execute_regex_match(self, fiber, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if matches!(
                            native_name,
                            "sumarray"
                                | "sumcarray"
                                | "maxarray"
                                | "maxcarray"
                                | "minarray"
                                | "mincarray"
                                | "match"
                                | "cmatch"
                                | "inrangearray"
                                | "inrangecarray"
                                | "groupmatch"
                                | "nosames"
                                | "allsames"
                        ) {
                            NativeReady::value(
                                execute_array_query(self, fiber, native_name, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if matches!(
                            native_name,
                            "charanum"
                                | "getchara"
                                | "getspchara"
                                | "existcsv"
                                | "csvname"
                                | "csvcallname"
                                | "csvnickname"
                                | "csvmastername"
                                | "csvcstr"
                                | "csvbase"
                                | "csvabl"
                                | "csvmark"
                                | "csvexp"
                                | "csvrelation"
                                | "csvtalent"
                                | "csvcflag"
                                | "csvequip"
                                | "csvjuel"
                                | "findchara"
                                | "findlastchara"
                        ) {
                            NativeReady::value(
                                execute_character_query(self, fiber, native_name, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if matches!(
                            native_name,
                            "addchara"
                                | "addspchara"
                                | "adddefchara"
                                | "addvoidchara"
                                | "delchara"
                                | "delallchara"
                                | "swapchara"
                                | "copychara"
                                | "addcopychara"
                                | "pickupchara"
                                | "sortchara"
                                | "reset_stain"
                        ) {
                            execute_character_mutation(self, native_name, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else {
                            let (ready, checkpoint) = self.call_registered_native(
                                fiber,
                                import.key,
                                (*target).clone(),
                                arguments,
                                natives,
                            )?;
                            rollback = checkpoint;
                            ready
                        };
                        let result = validate_native_ready(self, fiber, result_type, &ready)
                            .and_then(|()| {
                                self.apply_host_ready(
                                    fiber,
                                    result_type,
                                    HostReady {
                                        value: ready.value,
                                        writes: ready.writes,
                                    },
                                )
                            });
                        if let Err(error) = result {
                            if let Some(state) = rollback {
                                natives.rollback(import.key, &state).map_err(|failure| {
                                    StepError::classified(
                                        crate::FaultCategory::HostContract,
                                        VmFaultCode::Native,
                                        format!("native rollback failed: {failure}"),
                                    )
                                })?;
                            }
                            return Err(StepError::classified(
                                crate::FaultCategory::HostContract,
                                VmFaultCode::Native,
                                error.to_string(),
                            ));
                        }
                    }
                    (Opcode::CallHost, ImportKind::Host) => {
                        let target_index =
                            generation.host_import_index(import.key).ok_or_else(|| {
                                StepError::new(VmFaultCode::MissingSymbol, "host import is missing")
                            })?;
                        let target = artifact.host_imports.get(target_index).ok_or_else(|| {
                            StepError::new(VmFaultCode::MissingSymbol, "host import is missing")
                        })?;
                        let normalized_name = generation
                            .normalized_host_name(target_index)
                            .ok_or_else(|| {
                                StepError::new(VmFaultCode::MissingSymbol, "host import is missing")
                            })?;
                        if policy.allow_immediate_host {
                            match host.call_immediate(ImmediateHostCall {
                                fiber: fiber.id,
                                import: target,
                                normalized_name,
                                arguments: &arguments,
                            }) {
                                ImmediateHostCallResult::Unsupported => {}
                                ImmediateHostCallResult::Ready(ready) => {
                                    if host.path_memo_safe(&target.import) {
                                        self.observe_path_memo_safe_host(fiber.id, import.key);
                                    } else {
                                        self.invalidate_path_memo(fiber.id);
                                    }
                                    *host_calls = host_calls.saturating_add(1);
                                    self.apply_host_ready(fiber, target.import.result, ready)
                                        .map_err(|error| {
                                            StepError::classified(
                                                crate::FaultCategory::HostContract,
                                                VmFaultCode::Host,
                                                error.to_string(),
                                            )
                                        })?;
                                    return Ok(Some(StepOutcome::Continue));
                                }
                            }
                        }
                        self.invalidate_path_memo(fiber.id);
                        let target = target.clone();
                        let request = self.allocate_request_id();
                        *host_calls = host_calls.saturating_add(1);
                        let origin = self.execution_origin(position, &target.import.name);
                        match host.call(HostCallRequest {
                            id: request,
                            fiber: fiber.id,
                            import: target.import.clone(),
                            arguments,
                            origin: origin.clone(),
                        }) {
                            HostCallResult::Ready(ready) => self
                                .apply_host_ready(fiber, target.import.result, ready)
                                .map_err(|error| {
                                    StepError::classified(
                                        crate::FaultCategory::HostContract,
                                        VmFaultCode::Host,
                                        error.to_string(),
                                    )
                                })?,
                            HostCallResult::Pending {
                                stability,
                                rebind_payload,
                            } => {
                                if !target.effect.may_suspend {
                                    return Err(StepError::new(
                                        VmFaultCode::Host,
                                        "non-suspending host import returned pending",
                                    ));
                                }
                                if stability == HostWaitStability::StableInput
                                    && target.snapshot_capability
                                        != HostSnapshotCapability::StableWait
                                {
                                    return Err(StepError::new(
                                        VmFaultCode::Host,
                                        "host import reported a wait above its snapshot capability",
                                    ));
                                }
                                let result = target.import.result;
                                fiber.state = FiberState::WaitingHost(WaitingHost {
                                    request,
                                    import: target.import,
                                    result,
                                    stability,
                                    rebind_payload,
                                    origin: origin.clone(),
                                });
                                return Ok(Some(StepOutcome::Blocked));
                            }
                            HostCallResult::Error(error) => {
                                return Err(error);
                            }
                            HostCallResult::Deferred => {
                                let result = target.import.result;
                                fiber.state = FiberState::WaitingHost(WaitingHost {
                                    request,
                                    import: target.import,
                                    result,
                                    stability: HostWaitStability::Transient,
                                    rebind_payload: Vec::new(),
                                    origin,
                                });
                                return Ok(Some(StepOutcome::Blocked));
                            }
                        }
                    }
                    _ => {
                        return Err(StepError::new(
                            VmFaultCode::InvalidInstruction,
                            "call opcode and import kind differ",
                        ));
                    }
                }
            }
            _ => return Ok(None),
        }
        Ok(Some(StepOutcome::Continue))
    }
}
