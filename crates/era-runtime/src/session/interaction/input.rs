// This is part of the split RuntimeSession interaction implementation.
#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    pub(in super::super) fn open_update_prompt(
        &mut self,
        request: erabasic_vm::HostRequestId,
        remote_version: &str,
        download_url: String,
    ) -> Result<(), RuntimeError> {
        self.presentation.append_text(
            format!("New version {remote_version} is available: {download_url}"),
            false,
        );
        let no = self.allocate_interaction();
        let yes = self.allocate_interaction();
        self.presentation.append_button(
            "No".into(),
            era_runtime_protocol::ProtocolValue::Integer(0),
            no,
            None,
        );
        self.presentation.append_button(
            "Yes".into(),
            era_runtime_protocol::ProtocolValue::Integer(1),
            yes,
            None,
        );
        let submission = self.allocate_interaction();
        let pending = PendingInput {
            host_request: Some(request),
            wait: InputWait {
                wait_id: self.allocate_wait(),
                kind: WaitKind::IntegerButton,
                stability: WaitStability::Transient,
                one_input: false,
                stop_message_skip: false,
                system_input: false,
                mouse_input: false,
                default_value: None,
                deadline_ns: None,
                display_time: false,
                timeout_message: None,
                submission_token: submission,
                countdown_remaining_ms: None,
            },
            result_name: Some("RESULT".into()),
            choices: BTreeMap::from([(no, VmValue::Integer(1)), (yes, VmValue::Integer(2))]),
            timeout_duration_ns: None,
            post_input: Some(PostInputAction::OpenUrl {
                url: download_url,
                trigger_value: 2,
            }),
        };
        self.open_wait(pending, false)
    }

    pub(in super::super) fn complete_input(
        &mut self,
        message_id: u64,
        input: FrontendInput,
    ) -> Result<(), RuntimeError> {
        let Some(pending) = self.operations.active_input().cloned() else {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "no input is pending",
            );
        };
        if pending.wait.wait_id != input.wait_id {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "input wait identity is stale",
            );
        }
        if pending.wait.submission_token != input.token {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "input submission token is stale",
            );
        }
        if input.intent == InputIntent::Cancel {
            if self.queued_input.is_empty() {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "no input macro is being processed",
                );
            }
            return self.cancel_queued_input();
        }
        // An input event and a timer event are ordered commands. If this wait is
        // still active when its matching input arrives, the user action wins;
        // only an explicit AdvanceTime command may complete it as a timeout.
        // Reinterpreting the input's timestamp as a timer silently discarded
        // every kind of input at transient-wait boundaries. Cancellation is not
        // a completion and therefore leaves the timed wait's clock untouched.
        self.observe_frontend_time(input.monotonic_time_ns);
        if !self.queued_input.is_empty() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "an input macro is already being processed",
            );
        }
        if let InputIntent::CommitText(command) = &input.intent
            && command.len() > 1
            && command.starts_with('@')
            && !pending.wait.one_input
        {
            return self.handle_system_input_command(message_id, command);
        }
        if let InputIntent::ActivateKeyMacro { group, slot } = &input.intent {
            return self.recall_key_macro(message_id, *group, *slot);
        }
        let mut submitted_message_skip = input.message_skip;
        let mut intent = input.intent;
        if let InputIntent::CommitText(text) = intent {
            let Ok(pieces) = preprocess_input(&text) else {
                return self.reject(
                    message_id,
                    CommandErrorCode::ResourceLimit,
                    "input macro expansion exceeds the runtime limit",
                );
            };
            let mut pieces = pieces.into_iter();
            let segment = pieces.next().expect("input preprocessing yields one piece");
            submitted_message_skip |= segment.message_skip;
            self.queued_input.extend(pieces);
            intent = InputIntent::CommitText(segment.text);
        }
        let allow_long_activation = self
            .project_snapshot
            .as_ref()
            .is_some_and(|project| project.allow_long_input_by_activation);
        let Some(submission) = self.input_value_with_visible_button(
            &pending,
            input.token,
            intent.clone(),
            allow_long_activation,
        ) else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "input value does not match the active wait",
            );
        };
        self.message_skip = submitted_message_skip;
        let replay = self.replay_step_draft(&pending, &intent, &submission, submitted_message_skip);
        self.finish_input(submission, false)?;
        if let Some(replay) = replay {
            self.input_replay
                .record(replay, self.options.limits.maximum_transfer_bytes);
        }
        Ok(())
    }

    fn recall_key_macro(
        &mut self,
        message_id: u64,
        group: u8,
        slot: u8,
    ) -> Result<(), RuntimeError> {
        if !self
            .negotiated_features
            .contains(&RuntimeFeature::KeyMacros)
        {
            return self.reject(
                message_id,
                CommandErrorCode::FeatureUnavailable,
                "key macros were not negotiated",
            );
        }
        let Some(text) = self.key_macros.recall(group, slot) else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "key macro is disabled or out of range",
            );
        };
        self.text_box = text.into();
        self.emit_projection_state()
    }

    fn input_value_with_visible_button(
        &self,
        pending: &PendingInput,
        submission_token: InteractionToken,
        intent: InputIntent,
        allow_long_activation: bool,
    ) -> Option<InputSubmission> {
        let fallback_choice = match &intent {
            InputIntent::Activate(token) if !pending.choices.contains_key(token) => {
                self.presentation.enabled_button_value(*token)
            }
            _ => None,
        };
        let mut pending_with_fallback;
        let pending =
            if let (InputIntent::Activate(token), Some(value)) = (&intent, fallback_choice) {
                pending_with_fallback = pending.clone();
                pending_with_fallback.choices.insert(*token, value);
                &pending_with_fallback
            } else {
                pending
            };
        input_value(pending, submission_token, intent, allow_long_activation)
    }

    /// Feed the next expanded keyboard-input segment into the next wait without
    /// accepting a new frontend event or bypassing the wait's ordinary validator.
    pub(in super::super) fn consume_queued_input(&mut self) -> Result<(), RuntimeError> {
        let segment = self
            .queued_input
            .front()
            .cloned()
            .ok_or_else(|| RuntimeError::Internal("queued input segment disappeared".into()))?;
        let pending = self
            .operations
            .active_input()
            .ok_or_else(|| RuntimeError::Internal("queued input has no active wait".into()))?;
        if pending.wait.kind == WaitKind::Void {
            return self.finish_input(InputSubmission::Value(VmValue::Integer(0)), false);
        }
        let intent = queued_text_intent(&pending.wait, segment.text);
        let token = pending.wait.submission_token;
        self.queued_input.pop_front();
        let pending = self
            .operations
            .active_input()
            .expect("active wait is unchanged");
        let Some(submission) = input_value(pending, token, intent.clone(), false) else {
            return Ok(());
        };
        self.message_skip = segment.message_skip;
        let replay = self.replay_step_draft(pending, &intent, &submission, segment.message_skip);
        self.finish_input(submission, false)?;
        if let Some(replay) = replay {
            self.input_replay
                .record(replay, self.options.limits.maximum_transfer_bytes);
        }
        Ok(())
    }

    fn cancel_queued_input(&mut self) -> Result<(), RuntimeError> {
        self.queued_input.clear();
        self.message_skip = false;
        let wait_id = self.allocate_wait();
        let submission_token = self.allocate_interaction();
        let wait = {
            let pending = self
                .operations
                .active_input_mut()
                .expect("input cancellation requires an active wait");
            pending.wait.wait_id = wait_id;
            pending.wait.submission_token = submission_token;
            pending.wait.clone()
        };
        self.presentation.set_wait(Some(wait.clone()));
        self.emit_wait_change(WaitChange::Updated(wait))
    }

    pub(in super::super) fn handle_system_input_command(
        &mut self,
        message_id: u64,
        command: &str,
    ) -> Result<(), RuntimeError> {
        if self
            .operations
            .active_input()
            .is_some_and(|pending| pending.wait.deadline_ns.is_some())
        {
            return self.emit(
                RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                    code: "runtime.system_command_during_timed_wait".into(),
                    level: RuntimeLogLevel::Warning,
                    message: "system commands cannot be entered during a timed wait".into(),
                    source: None,
                    notification: DiagnosticNotification::default(),
                }),
                Some(message_id),
            );
        }
        self.presentation
            .append_print_text(command.to_owned(), false, true);
        self.emit_presentation()?;
        match command[1..].trim().to_ascii_uppercase().as_str() {
            "QUIT" | "EXIT" => {
                let exit = ExitRequested {
                    reason: ExitReason::Quit,
                    force: false,
                    runtime_revision: self.revision.saturating_add(1),
                };
                self.exit_requested = Some(exit);
                self.emit(RuntimeMessage::ExitRequested(exit), Some(message_id))?;
                self.set_phase(RuntimePhase::Stopping)
            }
            "REBOOT" => {
                let exit = ExitRequested {
                    reason: ExitReason::Restart,
                    force: false,
                    runtime_revision: self.revision.saturating_add(1),
                };
                self.exit_requested = Some(exit);
                self.emit(RuntimeMessage::ExitRequested(exit), Some(message_id))?;
                self.set_phase(RuntimePhase::Stopping)
            }
            "OUTPUT" | "OUTPUTLOG" => {
                if !self.negotiated_features.contains(&RuntimeFeature::Storage) {
                    return self.emit(
                        RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                            code: "runtime.system_output_unavailable".into(),
                            level: RuntimeLogLevel::Warning,
                            message: "@OUTPUT requires negotiated frontend storage".into(),
                            source: None,
                            notification: DiagnosticNotification::default(),
                        }),
                        Some(message_id),
                    );
                }
                let mut data = vec![0xef, 0xbb, 0xbf];
                data.extend_from_slice(self.presentation.log_text(false).as_bytes());
                self.issue_storage(
                    PendingStorage::SystemOutputLog {
                        resume_phase: self.phase,
                    },
                    StorageNamespace::Log,
                    StorageOperation::Write {
                        data: ProtocolBytes::new(data),
                        atomic_replace: self.storage_capabilities.atomic_replace,
                        precondition: StoragePrecondition::Any,
                    },
                    "emuera.log".into(),
                )
            }
            "CONFIG" => self.emit_effect(EffectKind::OpenConfiguration),
            "DEBUG" => self.emit(
                RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                    code: "runtime.debug_command_requires_debug_channel".into(),
                    level: RuntimeLogLevel::Warning,
                    message: "@DEBUG is available only through the granted debug protocol".into(),
                    source: None,
                    notification: DiagnosticNotification::default(),
                }),
                Some(message_id),
            ),
            _ => self.emit(
                RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                    code: "runtime.debug_command_requires_debug_channel".into(),
                    level: RuntimeLogLevel::Warning,
                    message: "arbitrary input debug commands are available only through the granted debug protocol".into(),
                    source: None,
                    notification: DiagnosticNotification::default(),
                }),
                Some(message_id),
            ),
        }
    }

    pub(in super::super) fn advance_time(
        &mut self,
        _message_id: u64,
        time: AdvanceTime,
    ) -> Result<(), RuntimeError> {
        self.observe_frontend_time(time.monotonic_time_ns);
        let ready_delays = self.operations.take_ready_delays(self.logical_time_ns);
        for request in ready_delays {
            let vm = self
                .vm
                .as_mut()
                .ok_or_else(|| RuntimeError::Internal("pending AWAIT has no VM".into()))?;
            commit_completion(vm, request, VmHostCompletion::Ready(HostReady::empty()))?;
            self.set_phase(RuntimePhase::Running)?;
        }
        let timed_out = self
            .operations
            .active_input()
            .and_then(|pending| pending.wait.deadline_ns)
            .is_some_and(|deadline| self.logical_time_ns >= deadline);
        if !timed_out
            && let Some(pending) = self.operations.active_input_mut()
            && pending.wait.display_time
            && let Some(deadline) = pending.wait.deadline_ns
        {
            let remaining = deadline
                .saturating_sub(self.logical_time_ns)
                .saturating_add(999_999)
                / 1_000_000;
            pending.wait.countdown_remaining_ms = Some(remaining);
            let wait = pending.wait.clone();
            self.presentation.set_wait(Some(wait.clone()));
            self.emit_wait_change(WaitChange::Updated(wait))?;
        }
        if timed_out {
            let pending = self
                .operations
                .active_input()
                .expect("checked above")
                .clone();
            if let Some(message) = &pending.wait.timeout_message {
                self.presentation.append_text(message.clone(), false);
            }
            let submission = if pending.wait.kind == WaitKind::PrimitiveMouseKey {
                InputSubmission::Primitive(PrimitiveResult {
                    fields: [4, 0, 0, 0, 0],
                    selection: None,
                })
            } else {
                InputSubmission::Value(
                    pending
                        .wait
                        .default_value
                        .as_ref()
                        .map_or(VmValue::Integer(0), protocol_to_vm),
                )
            };
            let replay = crate::input_replay::ReplayStepDraft {
                action: crate::input_replay::ReplayAction::Timeout,
                wait_kind: pending.wait.kind.into(),
                result: match &submission {
                    InputSubmission::Value(value) => {
                        crate::input_replay::ReplayValue::from_vm(value)
                    }
                    InputSubmission::Primitive(_) => None,
                },
                message_skip: self.message_skip,
                text: None,
                button: None,
                primitive: match &submission {
                    InputSubmission::Primitive(result) => {
                        Some(crate::input_replay::ReplayPrimitive::from_result(
                            result.fields,
                            result
                                .selection
                                .as_ref()
                                .and_then(crate::input_replay::ReplayValue::from_vm),
                        ))
                    }
                    InputSubmission::Value(_) => None,
                },
            };
            self.finish_input(submission, true)?;
            self.input_replay
                .record(replay, self.options.limits.maximum_transfer_bytes);
        }
        Ok(())
    }

    fn replay_step_draft(
        &self,
        pending: &PendingInput,
        intent: &InputIntent,
        submission: &InputSubmission,
        message_skip: bool,
    ) -> Option<crate::input_replay::ReplayStepDraft> {
        let action = crate::input_replay::action_for_intent(intent)?;
        let result = match submission {
            InputSubmission::Value(value) => crate::input_replay::ReplayValue::from_vm(value),
            InputSubmission::Primitive(_) => None,
        };
        let text = match intent {
            InputIntent::AnyKey(value) | InputIntent::CommitText(value) => Some(value.clone()),
            _ => None,
        };
        let button = match intent {
            InputIntent::Activate(token) => {
                Some(self.presentation.replay_button(*token, result.clone()?)?)
            }
            _ => None,
        };
        let primitive = match (intent, submission) {
            (InputIntent::Primitive(_), InputSubmission::Primitive(result)) => {
                Some(crate::input_replay::ReplayPrimitive::from_result(
                    result.fields,
                    result
                        .selection
                        .as_ref()
                        .and_then(crate::input_replay::ReplayValue::from_vm),
                ))
            }
            _ => None,
        };
        Some(crate::input_replay::ReplayStepDraft {
            action,
            wait_kind: pending.wait.kind.into(),
            result,
            message_skip,
            text,
            button,
            primitive,
        })
    }

    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn finish_input(
        &mut self,
        submission: InputSubmission,
        timed_out: bool,
    ) -> Result<(), RuntimeError> {
        let pending = self
            .operations
            .take_active_input()
            .ok_or_else(|| RuntimeError::Internal("input wait disappeared".into()))?;
        // The reference TextBox restores its configured position only after a
        // value was accepted, never after a rejected frontend event.
        self.text_box_layout = TextBoxLayout::default();
        if pending.wait.system_input {
            let InputSubmission::Value(value) = submission else {
                return Err(RuntimeError::Internal(
                    "system input cannot accept primitive fields".into(),
                ));
            };
            self.finish_system_input(pending, &value)?;
            return self.emit_projection_state();
        }
        if let InputSubmission::Value(value) = &submission {
            self.record_input_undo_value(value)?;
        }
        // Emuera prints and flushes a console input row after a successful integer,
        // string, or any-value wait. A visible-button submission can leave that row
        // empty, but it still counts as a physical/logical line for LINECOUNT/CLEARLINE.
        if matches!(
            pending.wait.kind,
            WaitKind::IntegerValue | WaitKind::StringValue | WaitKind::AnyValue
        ) {
            self.presentation.force_new_line();
        }
        let request = pending
            .host_request
            .ok_or_else(|| RuntimeError::Internal("VM wait has no host request".into()))?;
        let post_url = match (&pending.post_input, &submission) {
            (
                Some(PostInputAction::OpenUrl { url, trigger_value }),
                InputSubmission::Value(VmValue::Integer(value)),
            ) if value == trigger_value => Some(url.clone()),
            _ => None,
        };
        let vm = self
            .vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("input wait has no VM".into()))?;
        let mut writes = Vec::new();
        match submission {
            InputSubmission::Value(value) => {
                let result_name = if pending.wait.kind == WaitKind::AnyValue {
                    Some(match &value {
                        VmValue::String(_) => "RESULTS",
                        _ => "RESULT",
                    })
                } else {
                    pending.result_name.as_deref()
                };
                if let Some(target) = result_name.and_then(|name| global_place(vm, name)) {
                    writes.push(HostWrite { target, value });
                }
            }
            InputSubmission::Primitive(primitive) => {
                for (index, value) in primitive.fields.into_iter().enumerate() {
                    if let Some(target) = global_place_at(vm, "RESULT", index) {
                        writes.push(HostWrite {
                            target,
                            value: VmValue::Integer(i64::from(value)),
                        });
                    }
                }
                let result_5 = match primitive.selection {
                    Some(VmValue::Integer(value)) => value,
                    Some(VmValue::String(value)) => {
                        if let Some(target) = global_place(vm, "RESULTS") {
                            writes.push(HostWrite {
                                target,
                                value: VmValue::String(value),
                            });
                        }
                        0
                    }
                    None => 0,
                    Some(VmValue::IntegerPlace(_) | VmValue::StringPlace(_)) => {
                        return Err(RuntimeError::Internal(
                            "an interaction token resolved to a VM place".into(),
                        ));
                    }
                };
                if let Some(target) = global_place_at(vm, "RESULT", 5) {
                    writes.push(HostWrite {
                        target,
                        value: VmValue::Integer(result_5),
                    });
                }
            }
        }
        // ISTIMEOUT is only changed by a timed input completion; untimed waits leave it sticky.
        if pending.wait.deadline_ns.is_some()
            && let Some(target) = global_place(vm, "ISTIMEOUT")
        {
            writes.push(HostWrite {
                target,
                value: VmValue::Integer(i64::from(timed_out)),
            });
        }
        commit_completion(
            vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: None,
                writes,
            }),
        )?;
        let pause_next_wait = !vm.has_runnable_fibers();
        self.close_wait(pending.wait.wait_id)?;
        if let Some(next) = self.operations.pop_queued_input() {
            self.activate_wait(next, pause_next_wait)?;
        } else {
            self.set_phase(RuntimePhase::Running)?;
        }
        if let Some(url) = post_url {
            self.issue_platform_effect(
                ServiceKind::OpenUrl,
                OPEN_URL_OPERATION,
                OPEN_URL_OPERATION_VERSION,
                &OpenUrlRequest { url },
            )?;
        }
        self.emit_projection_state()
    }

    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn finish_system_input(
        &mut self,
        pending: PendingInput,
        value: &VmValue,
    ) -> Result<(), RuntimeError> {
        if self.controller.step != SystemStep::None && self.system_menu_host_request.is_none() {
            return self.finish_flow_input(pending, value);
        }
        match (self.system_menu, value) {
            (SystemMenuState::Title, VmValue::Integer(0)) => {
                self.close_wait(pending.wait.wait_id)?;
                let vm = self
                    .vm
                    .as_mut()
                    .ok_or_else(|| RuntimeError::Internal("system wait has no VM".into()))?;
                let prepared = vm
                    .prepare_runtime_state(VmRuntimeStateTransaction::ResetNewGame)
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                vm.commit_runtime_state(prepared)
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                let draw_line = vm
                    .vm()
                    .artifact()
                    .project_data
                    .static_data
                    .replace
                    .draw_line_string
                    .clone();
                self.presentation.append_separator(draw_line);
                self.presentation.append_text(String::new(), false);
                self.controller.flow = Some(SystemFlow::First);
                if !self
                    .controller
                    .prepare_event(vm.vm().artifact(), "EVENTFIRST")
                {
                    return Err(RuntimeError::Internal("EVENTFIRST is not defined".into()));
                }
                let entry = self.controller.next().expect("prepared EVENTFIRST entry");
                let fiber = vm
                    .spawn_entry(entry, Vec::new())
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                self.controller.started(fiber);
                self.set_phase(RuntimePhase::Running)
            }
            (SystemMenuState::Title, VmValue::Integer(1)) => {
                self.close_wait(pending.wait.wait_id)?;
                let vm = self
                    .vm
                    .as_mut()
                    .ok_or_else(|| RuntimeError::Internal("system wait has no VM".into()))?;
                if self
                    .controller
                    .prepare_function(vm.vm().artifact(), "TITLE_LOADGAME")
                {
                    self.controller.flow = Some(SystemFlow::Title);
                    self.controller.step = SystemStep::TitleLoadOverride;
                    let entry = self
                        .controller
                        .next()
                        .expect("prepared TITLE_LOADGAME entry");
                    let fiber = vm
                        .spawn_entry(entry, Vec::new())
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                    self.controller.started(fiber);
                    return self.set_phase(RuntimePhase::Running);
                }
                self.issue_storage(
                    PendingStorage::ListLoadSlots,
                    StorageNamespace::Save,
                    StorageOperation::List {
                        pattern: Some("save*.sav".into()),
                        recursive: false,
                    },
                    String::new(),
                )
            }
            (
                SystemMenuState::LoadSlots | SystemMenuState::SaveSlots,
                VmValue::Integer(selection),
            ) if *selection <= -1_000 => {
                let index = usize::try_from(selection.saturating_neg().saturating_sub(1_000))
                    .unwrap_or(usize::MAX);
                let Some(path) = self.load_slot_paths.get(index).cloned() else {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "unknown delete slot");
                };
                let save = self.system_menu == SystemMenuState::SaveSlots;
                self.close_wait(pending.wait.wait_id)?;
                self.issue_storage(
                    PendingStorage::StatDeleteMenuSlot {
                        save,
                        path: path.clone(),
                    },
                    StorageNamespace::Save,
                    StorageOperation::Stat,
                    path,
                )
            }
            (SystemMenuState::LoadSlots | SystemMenuState::SaveSlots, VmValue::Integer(100)) => {
                self.close_wait(pending.wait.wait_id)?;
                self.resume_system_menu_host()
            }
            (
                menu @ (SystemMenuState::LoadSlots | SystemMenuState::SaveSlots),
                VmValue::Integer(selection),
            ) if *selection >= 0 && (*selection != 99 || menu == SystemMenuState::SaveSlots) => {
                let slot_count = self
                    .project_snapshot
                    .as_ref()
                    .map_or(20, |snapshot| snapshot.save_slot_count)
                    .max(20);
                let Ok(slot) = u32::try_from(*selection) else {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "unknown save slot");
                };
                if slot >= slot_count {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "unknown save slot");
                }
                let target_page = slot / 20;
                if target_page != self.system_menu_page {
                    self.close_wait(pending.wait.wait_id)?;
                    self.system_menu_page = target_page;
                    return self.scan_slot_page(menu == SystemMenuState::SaveSlots);
                }
                let path = save_slot_path(slot);
                if menu == SystemMenuState::SaveSlots {
                    self.close_wait(pending.wait.wait_id)?;
                    if self.occupied_slot_paths.contains(&path) {
                        self.system_menu = SystemMenuState::ConfirmOverwrite { slot };
                        self.presentation.append_system_text(
                            localized_system_text(
                                &self.selected_locale,
                                SystemTextKey::OverwriteQuestion,
                            ),
                            SystemTextKey::OverwriteQuestion,
                            vec![SystemTextArgument::Integer(i64::from(slot))],
                            false,
                        );
                        let yes = self.allocate_interaction();
                        let no = self.allocate_interaction();
                        self.presentation.append_system_button(
                            "Yes".into(),
                            SystemTextKey::OverwriteQuestion,
                            vec![SystemTextArgument::Integer(0)],
                            yes,
                        );
                        self.presentation.append_system_button(
                            "No".into(),
                            SystemTextKey::OverwriteQuestion,
                            vec![SystemTextArgument::Integer(1)],
                            no,
                        );
                        let submission = self.allocate_interaction();
                        let mut wait = self.system_wait(submission);
                        wait.kind = WaitKind::IntegerValue;
                        return self.open_wait(
                            PendingInput {
                                host_request: self.system_menu_host_request,
                                wait,
                                result_name: None,
                                choices: BTreeMap::from([
                                    (yes, VmValue::Integer(0)),
                                    (no, VmValue::Integer(1)),
                                ]),
                                timeout_duration_ns: None,
                                post_input: None,
                            },
                            true,
                        );
                    }
                    return self.begin_system_menu_candidate(slot);
                }
                if !self.occupied_slot_paths.contains(&path) {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "save slot is empty");
                }
                if self.invalid_slot_paths.contains(&path) {
                    self.operations.restore_active_input(pending);
                    return self.reject(
                        0,
                        CommandErrorCode::InvalidValue,
                        "save slot is incompatible or corrupt",
                    );
                }
                self.close_wait(pending.wait.wait_id)?;
                self.issue_storage(
                    PendingStorage::ReadLoadSlot { slot },
                    StorageNamespace::Save,
                    StorageOperation::Read,
                    path,
                )
            }
            (SystemMenuState::LoadSlots, VmValue::Integer(99)) => {
                let path = save_slot_path(99);
                if !self.occupied_slot_paths.contains(&path)
                    || self.invalid_slot_paths.contains(&path)
                {
                    self.operations.restore_active_input(pending);
                    return self.reject(
                        0,
                        CommandErrorCode::InvalidValue,
                        "autosave is unavailable",
                    );
                }
                self.close_wait(pending.wait.wait_id)?;
                self.issue_storage(
                    PendingStorage::ReadLoadSlot { slot: 99 },
                    StorageNamespace::Save,
                    StorageOperation::Read,
                    path,
                )
            }
            (SystemMenuState::ConfirmOverwrite { slot }, VmValue::Integer(0)) => {
                self.close_wait(pending.wait.wait_id)?;
                self.begin_system_menu_candidate(slot)
            }
            (SystemMenuState::ConfirmOverwrite { .. }, VmValue::Integer(1)) => {
                self.close_wait(pending.wait.wait_id)?;
                self.system_menu = SystemMenuState::SaveSlots;
                self.render_slot_menu(true)
            }
            _ => {
                if self.presentation.last_line_is_temporary()
                    && self.presentation.last_line_is_empty()
                {
                    self.presentation.delete_last_lines(2);
                    self.presentation.append_text(
                        localized_system_text(&self.selected_locale, SystemTextKey::InvalidValue),
                        true,
                    );
                } else {
                    self.presentation
                        .replace_last_temporary(localized_system_text(
                            &self.selected_locale,
                            SystemTextKey::InvalidValue,
                        ));
                }
                self.operations.restore_active_input(pending);
                self.emit_presentation()?;
                self.reject(
                    0,
                    CommandErrorCode::InvalidValue,
                    "unknown system menu item",
                )
            }
        }
    }
}
