impl RuntimeSession {
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
                    context: None,
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
                            context: None,
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
                    context: None,
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
                    context: None,
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
                source: self.active_input_source.clone(),
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

}
