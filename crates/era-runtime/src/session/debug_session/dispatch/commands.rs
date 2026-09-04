impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    fn dispatch_debug_command(
        &mut self,
        message_id: u64,
        command: DebugCommand,
    ) -> Result<(), RuntimeError> {
        // A postmortem stop exposes the existing fault without making the game
        // executable or writable again. Keep ordinary grant and stop checks intact.
        if self.phase == RuntimePhase::DebugPaused
            && self.debug_resume_phase == Some(RuntimePhase::Faulted)
            && !matches!(
                command_scope(&command),
                DebugScope::VariablesRead
                    | DebugScope::GameFieldsRead
                    | DebugScope::ExecutionRead
                    | DebugScope::ConsoleEvaluate
                    | DebugScope::ScriptOutput
            )
            && !matches!(&command, DebugCommand::Continue { .. })
        {
            match &command {
                DebugCommand::Step { stop, .. }
                | DebugCommand::WriteVariables { stop, .. }
                | DebugCommand::WriteGameFields { stop, .. }
                | DebugCommand::Console { stop, .. } => {
                    self.validate_stop(*stop, message_id)?;
                }
                _ => {}
            }
            return self.emit_debug_error(
                DebugErrorCode::InvalidState,
                "postmortem debug stops only allow read-only inspection and continue",
                Some(message_id),
            );
        }
        match command {
            DebugCommand::Pause => {
                if !matches!(
                    self.phase,
                    RuntimePhase::Running
                        | RuntimePhase::WaitingInput
                        | RuntimePhase::WaitingExternal
                        | RuntimePhase::Faulted
                ) {
                    return self.emit_debug_error(
                        DebugErrorCode::InvalidState,
                        "runtime cannot be paused in its current phase",
                        Some(message_id),
                    );
                }
                let previous = self.phase;
                let vm = self.debug_vm_mut(message_id)?;
                let stop = match vm.request_pause() {
                    Ok(stop) => stop,
                    Err(error) => return self.emit_vm_debug_error(error, Some(message_id)),
                };
                self.debug_resume_phase = Some(previous);
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::Accepted),
                    Some(message_id),
                )?;
                self.enter_debug_stop(stop, None)
            }
            DebugCommand::Continue { stop } => {
                let vm_stop = self.validate_stop(stop, message_id)?;
                if let Err(error) = self.debug_vm_mut(message_id)?.continue_execution(vm_stop) {
                    return self.emit_vm_debug_error(error, Some(message_id));
                }
                self.resume_debug_time();
                let phase = self
                    .debug_resume_phase
                    .take()
                    .unwrap_or(RuntimePhase::Running);
                self.set_phase(phase)?;
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::Accepted),
                    Some(message_id),
                )
            }
            DebugCommand::Step {
                stop,
                fiber_id,
                kind,
            } => {
                let vm_stop = self.validate_stop(stop, message_id)?;
                if let Err(error) = self.debug_vm_mut(message_id)?.step(
                    vm_stop,
                    FiberId(fiber_id),
                    vm_step_kind(kind),
                ) {
                    return self.emit_vm_debug_error(error, Some(message_id));
                }
                self.set_phase(RuntimePhase::Running)?;
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::Accepted),
                    Some(message_id),
                )
            }
            DebugCommand::ListFibers {
                stop,
                cursor,
                limit,
            } => {
                let vm_stop = self.validate_stop(stop, message_id)?;
                let page = match self.debug_vm(message_id)?.fibers(
                    vm_stop,
                    usize_cursor(cursor)?,
                    usize::try_from(limit).unwrap_or(usize::MAX),
                ) {
                    Ok(value) => value,
                    Err(error) => return self.emit_vm_debug_error(error, Some(message_id)),
                };
                let response = FiberPage {
                    stop,
                    fibers: page.values.iter().map(protocol_fiber).collect(),
                    next_cursor: page.next_cursor.map(|value| value as u64),
                };
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::FiberPage(response)),
                    Some(message_id),
                )
            }
            DebugCommand::ReadCallStack { stop, fiber_id } => {
                let vm_stop = self.validate_stop(stop, message_id)?;
                let frames = match self
                    .debug_vm(message_id)?
                    .call_stack(vm_stop, FiberId(fiber_id))
                {
                    Ok(value) => value,
                    Err(error) => return self.emit_vm_debug_error(error, Some(message_id)),
                };
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::CallStack(CallStack {
                        stop,
                        fiber_id,
                        frames: frames.into_iter().map(protocol_frame).collect(),
                    })),
                    Some(message_id),
                )
            }
            DebugCommand::ReadOperandStack {
                stop,
                fiber_id,
                frame_id,
                cursor,
                limit,
            } => {
                let vm_stop = self.validate_stop(stop, message_id)?;
                let generation = match self
                    .debug_vm(message_id)?
                    .call_stack(vm_stop, FiberId(fiber_id))
                {
                    Ok(frames) => frames
                        .into_iter()
                        .find(|frame| frame.id == FrameId(frame_id))
                        .map_or(0, |frame| frame.generation.0),
                    Err(error) => return self.emit_vm_debug_error(error, Some(message_id)),
                };
                let page = match self.debug_vm(message_id)?.operand_stack(
                    vm_stop,
                    FiberId(fiber_id),
                    FrameId(frame_id),
                    usize_cursor(cursor)?,
                    usize::try_from(limit).unwrap_or(usize::MAX),
                ) {
                    Ok(value) => value,
                    Err(error) => return self.emit_vm_debug_error(error, Some(message_id)),
                };
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::OperandStack(OperandStackPage {
                        stop,
                        fiber_id,
                        frame_id,
                        values: page
                            .values
                            .into_iter()
                            .map(|value| OperandValue {
                                offset: value.offset as u64,
                                value: protocol_value_in_generation(value.value, generation),
                            })
                            .collect(),
                        next_cursor: page.next_cursor.map(|value| value as u64),
                    })),
                    Some(message_id),
                )
            }
            DebugCommand::ListVariables {
                stop,
                cursor,
                limit,
            } => {
                let vm_stop = self.validate_stop(stop, message_id)?;
                let page = match self.debug_vm(message_id)?.variables(
                    vm_stop,
                    usize_cursor(cursor)?,
                    usize::try_from(limit).unwrap_or(usize::MAX),
                ) {
                    Ok(value) => value,
                    Err(error) => return self.emit_vm_debug_error(error, Some(message_id)),
                };
                let variables = page
                    .values
                    .iter()
                    .map(|value| self.variable_descriptor(value))
                    .collect();
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::VariablePage(VariablePage {
                        stop,
                        variables,
                        next_cursor: page.next_cursor.map(|value| value as u64),
                    })),
                    Some(message_id),
                )
            }
            DebugCommand::ReadVariable { stop, value } => {
                let vm_stop = self.validate_stop(stop, message_id)?;
                let reference = match vm_variable_reference(&value) {
                    Ok(value) => value,
                    Err(message) => {
                        return self.emit_debug_error(
                            DebugErrorCode::UnknownTarget,
                            message,
                            Some(message_id),
                        );
                    }
                };
                let result = match self
                    .debug_vm(message_id)?
                    .read_variable(vm_stop, &reference)
                {
                    Ok(value) => value,
                    Err(error) => return self.emit_vm_debug_error(error, Some(message_id)),
                };
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::VariableValue(protocol_variable_value(
                        result,
                    ))),
                    Some(message_id),
                )
            }
            DebugCommand::WriteVariables { stop, writes } => {
                let vm_stop = self.validate_stop(stop, message_id)?;
                let mut converted = Vec::with_capacity(writes.len());
                for write in &writes {
                    let target = match vm_variable_reference(&write.reference) {
                        Ok(value) => value,
                        Err(message) => {
                            return self.emit_debug_error(
                                DebugErrorCode::UnknownTarget,
                                message,
                                Some(message_id),
                            );
                        }
                    };
                    let value = match vm_value(&write.value) {
                        Ok(value) => value,
                        Err(message) => {
                            return self.emit_debug_error(
                                DebugErrorCode::TypeMismatch,
                                message,
                                Some(message_id),
                            );
                        }
                    };
                    converted.push(VmDebugVariableWrite {
                        target,
                        value,
                        expected_revision: write.expected_revision,
                    });
                }
                let values = match self
                    .debug_vm_mut(message_id)?
                    .write_variables(vm_stop, &converted)
                {
                    Ok(value) => value,
                    Err(error) => return self.emit_vm_debug_error(error, Some(message_id)),
                };
                self.revision = self.revision.saturating_add(1);
                let stop = self.refreshed_stop(stop);
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::VariablesWritten(VariableWriteOutcome {
                        stop,
                        values: values.into_iter().map(protocol_variable_value).collect(),
                    })),
                    Some(message_id),
                )
            }
            DebugCommand::ListGameFields {
                stop,
                cursor,
                limit,
            } => {
                self.validate_stop(stop, message_id)?;
                let start = usize_cursor(cursor)?.unwrap_or(0);
                let limit = usize::try_from(limit).unwrap_or(usize::MAX);
                if limit == 0 || limit > 1024 {
                    return self.emit_debug_error(
                        DebugErrorCode::ResourceLimit,
                        "invalid debugger page size",
                        Some(message_id),
                    );
                }
                let all = game_field_descriptors();
                let fields = all
                    .iter()
                    .skip(start)
                    .take(limit)
                    .cloned()
                    .collect::<Vec<_>>();
                let consumed = start.saturating_add(fields.len());
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::GameFieldPage(GameFieldPage {
                        stop,
                        fields,
                        next_cursor: (consumed < all.len()).then_some(consumed as u64),
                    })),
                    Some(message_id),
                )
            }
            DebugCommand::ReadGameField { stop, key } => {
                self.validate_stop(stop, message_id)?;
                let Some(value) = self.read_game_field(&key) else {
                    return self.emit_debug_error(
                        DebugErrorCode::UnknownTarget,
                        "unknown runtime game field",
                        Some(message_id),
                    );
                };
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::GameFieldValue(value)),
                    Some(message_id),
                )
            }
            DebugCommand::WriteGameFields { stop, writes } => {
                self.validate_stop(stop, message_id)?;
                if writes.is_empty() || writes.len() > 1024 {
                    return self.emit_debug_error(
                        DebugErrorCode::ResourceLimit,
                        "invalid game-field write batch",
                        Some(message_id),
                    );
                }
                if writes
                    .iter()
                    .any(|write| write.expected_revision != self.revision)
                {
                    return self.emit_debug_error(
                        DebugErrorCode::StaleRevision,
                        "stale runtime revision",
                        Some(message_id),
                    );
                }
                let mut next_message_skip = self.message_skip;
                for write in &writes {
                    match (&*write.key, &write.value) {
                        ("input.message_skip", DebugValue::Boolean(value)) => {
                            next_message_skip = *value;
                        }
                        ("input.message_skip", _) => {
                            return self.emit_debug_error(
                                DebugErrorCode::TypeMismatch,
                                "input.message_skip requires a boolean",
                                Some(message_id),
                            );
                        }
                        _ => {
                            return self.emit_debug_error(
                                DebugErrorCode::PermissionDenied,
                                "runtime game field is read-only",
                                Some(message_id),
                            );
                        }
                    }
                }
                self.message_skip = next_message_skip;
                self.revision = self.revision.saturating_add(1);
                let stop = self.refreshed_stop(stop);
                let values = writes
                    .iter()
                    .filter_map(|write| self.read_game_field(&write.key))
                    .collect();
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::GameFieldsWritten(
                        GameFieldWriteOutcome { stop, values },
                    )),
                    Some(message_id),
                )
            }
            DebugCommand::UpdateBreakpoints { update } => {
                let mut values = Vec::with_capacity(update.requested.len());
                for breakpoint in &update.requested {
                    match vm_breakpoint(breakpoint) {
                        Ok(value) => values.push(value),
                        Err(message) => {
                            return self.emit_debug_error(
                                DebugErrorCode::UnknownTarget,
                                message,
                                Some(message_id),
                            );
                        }
                    }
                }
                let resolved = match self
                    .debug_vm_mut(message_id)?
                    .update_breakpoints(&values, &update.remove)
                {
                    Ok(value) => value,
                    Err(error) => return self.emit_vm_debug_error(error, Some(message_id)),
                };
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::Breakpoints(
                        resolved.into_iter().map(protocol_breakpoint).collect(),
                    )),
                    Some(message_id),
                )
            }
            DebugCommand::Console { stop, command } => {
                self.debug_console(message_id, stop, command)
            }
            DebugCommand::ReadScriptOutput { cursor, limit } => {
                let truncated = cursor < self.debug_output_base;
                let relative = if truncated {
                    0
                } else {
                    usize::try_from(cursor.saturating_sub(self.debug_output_base))
                        .unwrap_or(usize::MAX)
                        .min(self.debug_output.len())
                };
                let relative = next_char_boundary(&self.debug_output, relative);
                let maximum = usize::try_from(limit.min(1_048_576)).unwrap_or(1_048_576);
                let end = previous_char_boundary(
                    &self.debug_output,
                    relative
                        .saturating_add(maximum)
                        .min(self.debug_output.len()),
                );
                let actual_cursor = self
                    .debug_output_base
                    .saturating_add(u64::try_from(relative).unwrap_or(u64::MAX));
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::ScriptOutput(ScriptOutputChunk {
                        cursor: actual_cursor,
                        next_cursor: self
                            .debug_output_base
                            .saturating_add(u64::try_from(end).unwrap_or(u64::MAX)),
                        text: self.debug_output[relative..end].to_owned(),
                        truncated,
                    })),
                    Some(message_id),
                )
            }
            DebugCommand::SubscribeScriptOutput { enabled } => {
                self.debug_output_subscribed = enabled;
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::Accepted),
                    Some(message_id),
                )
            }
        }
    }

}
