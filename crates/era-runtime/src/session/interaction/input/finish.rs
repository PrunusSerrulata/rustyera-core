impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn finish_input(
        &mut self,
        submission: InputSubmission,
        timed_out: bool,
    ) -> Result<(), RuntimeError> {
        let wait = self
            .operations
            .active_input()
            .ok_or_else(|| RuntimeError::Internal("input wait disappeared".into()))?;
        if !wait.wait.system_input
            && records_input_undo(wait.wait.kind)
            && let InputSubmission::Value(value) = &submission
        {
            self.verify_replayed_input(value)?;
        }
        let sequence_trace = if self.undo_replay.is_none()
            && self
                .active_input_source
                .as_ref()
                .is_some_and(|source| matches!(source.root, InputRoot::Sequence(_)))
        {
            let text = match &submission {
                InputSubmission::Value(value) => display_value(value),
                InputSubmission::Primitive(_) => String::new(),
            };
            self.replay_step_draft(
                self.operations.active_input().expect("checked wait"),
                &InputIntent::CommitText(text),
                &submission,
                self.message_skip,
            )
        } else {
            None
        };
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
            self.active_input_source = None;
            self.finish_system_input(pending, &value)?;
            if let Some(trace) = sequence_trace {
                self.input_replay
                    .record(trace, self.options.limits.maximum_transfer_bytes);
            }
            return self.emit_projection_state();
        }
        if records_input_undo(pending.wait.kind)
            && let InputSubmission::Value(value) = &submission
        {
            self.record_input_undo_value(value)?;
        }
        self.active_input_source = None;
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
        if let Some(trace) = sequence_trace {
            self.input_replay
                .record(trace, self.options.limits.maximum_transfer_bytes);
        }
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

}
