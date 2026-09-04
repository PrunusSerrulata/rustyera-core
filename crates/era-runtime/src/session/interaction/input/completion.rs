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
                viewport_policy: era_runtime_protocol::InputViewportPolicy::FollowOutput,
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
            && self.input_controller.macro_enabled
            && !pending.wait.one_input
        {
            return self.handle_system_input_command(message_id, command);
        }
        if let InputIntent::ActivateKeyMacro { group, slot } = &input.intent {
            return self.recall_key_macro(message_id, *group, *slot);
        }
        self.active_input_source = None;
        let submitted_message_skip = input.message_skip;
        let intent = match input.intent {
            InputIntent::CommitText(text) => {
                return self.complete_text_input(
                    message_id,
                    &pending,
                    text,
                    submitted_message_skip,
                );
            }
            intent => intent,
        };
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

    fn complete_text_input(
        &mut self,
        message_id: u64,
        pending: &PendingInput,
        text: String,
        submitted_message_skip: bool,
    ) -> Result<(), RuntimeError> {
        let intent = InputIntent::CommitText(text.clone());
        let source = self
            .input_controller
            .admit(InputRoot::External, text, submitted_message_skip)
            .map_err(RuntimeError::ResourceLimit)?;
        let submission = match self.prepare_text_submission(pending, &source) {
            Ok(Some(submission)) => submission,
            Ok(None) => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "input value does not match the active wait",
                );
            }
            Err(RuntimeError::ResourceLimit(message)) => {
                return self.reject(message_id, CommandErrorCode::ResourceLimit, message);
            }
            Err(error) => return Err(error),
        };
        let replay = self.replay_step_draft(pending, &intent, &submission, self.message_skip);
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
        let pending = pending.clone();
        self.queued_input.pop_front();
        let intent = super::admission::fragment_intent(&pending, &segment);
        let Some(submission) = self.prepare_fragment(&pending, segment) else {
            return Ok(());
        };
        let replay = (self.undo_replay.is_none()
            && !self
                .active_input_source
                .as_ref()
                .is_some_and(|source| matches!(source.root, InputRoot::Sequence(_))))
        .then(|| self.replay_step_draft(&pending, &intent, &submission, self.message_skip))
        .flatten();
        self.finish_input(submission, false)?;
        if let Some(replay) = replay {
            self.input_replay
                .record(replay, self.options.limits.maximum_transfer_bytes);
        }
        Ok(())
    }

    fn cancel_queued_input(&mut self) -> Result<(), RuntimeError> {
        self.queued_input.clear();
        self.active_input_source = None;
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

}
