use era_debug_protocol::{
    AuthorizedDebugRequest, Breakpoint, BreakpointBinding, BreakpointLocation, CallStack,
    ConsoleCommand, ConsoleOutcome, DEBUG_PROTOCOL_VERSION, DebugCommand, DebugDiagnostic,
    DebugError, DebugErrorCode, DebugGrant, DebugHello, DebugMessage, DebugResponse, DebugScope,
    DebugSourceLocation, DebugStop, DebugValue, FiberPage, FiberState, FiberSummary,
    FieldMutability, FrameSummary, GameFieldDescriptor, GameFieldPage, GameFieldValue,
    GameFieldWriteOutcome, GrantToken, OperandStackPage, OperandValue, ResolvedBreakpoint,
    ScriptOutputChunk, StepKind, StopReason, StopToken, ValueKind, VariableDescriptor,
    VariablePage, VariableReference, VariableStorage, VariableValue, VariableWriteOutcome,
    grant_scopes,
};
use era_protocol::{ProtocolBytes, SessionId, VersionRange, encode_envelope, negotiate_version};
use erabasic_ast::{BinaryOp, Expr, ExprKind, UnaryOp};
use erabasic_bytecode::{BytecodeStorage, Digest, SymbolKey};
use erabasic_parser::{DefaultParserContext, ParserContext, parse_expression};
use erabasic_vm::{
    FiberId, FiberStatus, FrameId, GenerationId, PlaceDescriptor, VmBreakpoint,
    VmBreakpointBinding, VmBreakpointLocation, VmDebugControl, VmDebugInspect, VmDebugStop,
    VmDebugStopReason, VmDebugVariable, VmDebugVariableRef, VmDebugVariableWrite, VmError,
    VmResolvedBreakpoint, VmRuntimePort, VmStepKind, VmStopToken, VmValue, evaluate_pure_native,
};

use super::{ActiveDebugGrant, RuntimeError, RuntimeLogLevel, RuntimePhase, RuntimeSession};

const DEBUG_REQUEST_REJECTED: &str = "debug request rejected";

impl RuntimeSession {
    pub(super) fn handle_debug_message(
        &mut self,
        message_id: u64,
        message: DebugMessage,
    ) -> Result<(), RuntimeError> {
        match message {
            DebugMessage::Hello(hello) => self.debug_hello(message_id, &hello),
            DebugMessage::Request(request) => match self.debug_request(message_id, request) {
                Err(RuntimeError::Internal(message)) if message == DEBUG_REQUEST_REJECTED => Ok(()),
                result => result,
            },
            DebugMessage::Revoke(revoke) => self.revoke_debug_grant(revoke.grant_id),
            _ => self.emit_debug_error(
                DebugErrorCode::InvalidState,
                "debug message direction is frontend-incompatible",
                Some(message_id),
            ),
        }
    }

    fn revoke_debug_grant(&mut self, grant_id: SessionId) -> Result<(), RuntimeError> {
        if self
            .active_debug_grant
            .as_ref()
            .is_none_or(|grant| grant.token.grant_id != grant_id)
        {
            return Ok(());
        }
        if self.phase == RuntimePhase::DebugPaused {
            let stop = self
                .vm
                .as_ref()
                .and_then(VmDebugInspect::stop_token)
                .ok_or_else(|| {
                    RuntimeError::Internal("debug-paused runtime has no VM stop token".into())
                })?;
            self.vm
                .as_mut()
                .ok_or_else(|| RuntimeError::Internal("debug-paused runtime has no VM".into()))?
                .continue_execution(stop)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            self.resume_debug_time();
            let phase = self
                .debug_resume_phase
                .take()
                .unwrap_or(RuntimePhase::Running);
            self.set_phase(phase)?;
        }
        self.active_debug_grant = None;
        Ok(())
    }

    fn debug_hello(&mut self, message_id: u64, hello: &DebugHello) -> Result<(), RuntimeError> {
        let supported = VersionRange::exact(DEBUG_PROTOCOL_VERSION);
        if negotiate_version(hello.versions, supported).is_none() {
            return self.emit_debug_error(
                DebugErrorCode::InvalidState,
                "debug protocol 4.0 is required",
                Some(message_id),
            );
        }
        let policy = all_debug_scopes()
            .into_iter()
            .filter(|scope| self.options.debug_scope_mask & scope_bit(*scope) != 0)
            .collect::<Vec<_>>();
        let scopes = grant_scopes(&policy, &hello.requested_scopes);
        let token = GrantToken {
            grant_id: SessionId {
                high: self.options.session_id.high ^ 0x4445_4255_4747_5241,
                low: self.next_debug_grant_id,
            },
            session_epoch: self.epoch.0,
            program_generation: self.vm.as_ref().map_or(0, |vm| vm.current_generation().0),
            issued_runtime_revision: self.revision,
        };
        self.next_debug_grant_id = self.next_debug_grant_id.saturating_add(1);
        self.active_debug_grant = Some(ActiveDebugGrant {
            token,
            scopes: scopes.iter().copied().collect(),
        });
        self.emit_debug(
            DebugMessage::Grant(DebugGrant {
                version: DEBUG_PROTOCOL_VERSION,
                token,
                scopes,
            }),
            Some(message_id),
        )
    }

    fn debug_request(
        &mut self,
        message_id: u64,
        request: AuthorizedDebugRequest,
    ) -> Result<(), RuntimeError> {
        let Some(grant) = self.active_debug_grant.as_ref() else {
            return self.emit_debug_error(
                DebugErrorCode::PermissionDenied,
                "no active debug grant",
                Some(message_id),
            );
        };
        if request.grant != grant.token {
            return self.emit_debug_error(
                DebugErrorCode::PermissionDenied,
                "debug grant is stale or belongs to another session generation",
                Some(message_id),
            );
        }
        let required = command_scope(&request.command);
        if !grant.scopes.contains(&required) {
            return self.emit_debug_error(
                DebugErrorCode::PermissionDenied,
                "debug grant does not include the required scope",
                Some(message_id),
            );
        }
        self.dispatch_debug_command(message_id, request.command)
    }

    #[allow(clippy::too_many_lines)]
    fn dispatch_debug_command(
        &mut self,
        message_id: u64,
        command: DebugCommand,
    ) -> Result<(), RuntimeError> {
        match command {
            DebugCommand::Pause => {
                if !matches!(
                    self.phase,
                    RuntimePhase::Running
                        | RuntimePhase::WaitingInput
                        | RuntimePhase::WaitingExternal
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

    pub(super) fn enter_debug_stop(
        &mut self,
        stop: VmDebugStop,
        correlation_id: Option<u64>,
    ) -> Result<(), RuntimeError> {
        if self.debug_resume_phase.is_none() {
            self.debug_resume_phase = Some(self.phase);
        }
        self.set_phase(RuntimePhase::DebugPaused)?;
        let stop = self.protocol_stop(stop);
        self.emit_log(
            RuntimeLogLevel::Debug,
            format!("debug stopped: {:?}", stop.reason),
        )?;
        self.emit_debug(DebugMessage::Stopped(stop), correlation_id)
    }

    /// Re-issue an existing grant after an epoch/generation transition. The
    /// creator policy and granted scope set remain unchanged, while stale tokens
    /// become unusable immediately.
    pub(super) fn renew_debug_grant(&mut self) -> Result<(), RuntimeError> {
        let Some(previous) = self.active_debug_grant.clone() else {
            return Ok(());
        };
        let token = GrantToken {
            grant_id: SessionId {
                high: self.options.session_id.high ^ 0x4445_4255_4747_5241,
                low: self.next_debug_grant_id,
            },
            session_epoch: self.epoch.0,
            program_generation: self.vm.as_ref().map_or(0, |vm| vm.current_generation().0),
            issued_runtime_revision: self.revision,
        };
        self.next_debug_grant_id = self.next_debug_grant_id.saturating_add(1);
        let scopes = previous.scopes.into_iter().collect::<Vec<_>>();
        self.active_debug_grant = Some(ActiveDebugGrant {
            token,
            scopes: scopes.iter().copied().collect(),
        });
        self.emit_debug(
            DebugMessage::Grant(DebugGrant {
                version: DEBUG_PROTOCOL_VERSION,
                token,
                scopes,
            }),
            None,
        )
    }

    fn protocol_stop(&self, stop: VmDebugStop) -> DebugStop {
        DebugStop {
            stop: StopToken {
                session_epoch: self.epoch.0,
                pause_epoch: stop.token.pause_epoch,
                program_generation: stop.token.generation.0,
                runtime_revision: self.revision,
            },
            reason: match stop.reason {
                VmDebugStopReason::PauseRequested => StopReason::PauseRequested,
                VmDebugStopReason::Breakpoint(breakpoint_id) => {
                    StopReason::Breakpoint { breakpoint_id }
                }
                VmDebugStopReason::StepCompleted => StopReason::StepCompleted,
                VmDebugStopReason::HostWait => StopReason::HostWait,
                VmDebugStopReason::FiberCompleted => StopReason::FiberCompleted,
                VmDebugStopReason::Fault(fault) => StopReason::Fault {
                    message: fault.message,
                },
                VmDebugStopReason::Reload => StopReason::Reload,
            },
            selected_fiber: stop.selected_fiber.map(|value| value.0),
            source: stop.source.map(protocol_source),
        }
    }

    fn validate_stop(
        &mut self,
        stop: StopToken,
        message_id: u64,
    ) -> Result<VmStopToken, RuntimeError> {
        if self.phase != RuntimePhase::DebugPaused
            || stop.session_epoch != self.epoch.0
            || stop.runtime_revision != self.revision
        {
            self.emit_debug_error(
                DebugErrorCode::StaleStop,
                "debug stop token is stale",
                Some(message_id),
            )?;
            return Err(RuntimeError::Internal(DEBUG_REQUEST_REJECTED.into()));
        }
        let vm_stop = VmStopToken {
            pause_epoch: stop.pause_epoch,
            generation: GenerationId(stop.program_generation),
        };
        if self.debug_vm(message_id)?.stop_token() != Some(vm_stop) {
            self.emit_debug_error(
                DebugErrorCode::StaleStop,
                "VM stop token is stale",
                Some(message_id),
            )?;
            return Err(RuntimeError::Internal(DEBUG_REQUEST_REJECTED.into()));
        }
        Ok(vm_stop)
    }

    fn refreshed_stop(&self, stop: StopToken) -> StopToken {
        StopToken {
            runtime_revision: self.revision,
            ..stop
        }
    }

    fn debug_vm(&mut self, message_id: u64) -> Result<&erabasic_vm::RuntimeVm, RuntimeError> {
        if self.vm.is_none() {
            self.emit_debug_error(
                DebugErrorCode::InvalidState,
                "runtime has no active VM",
                Some(message_id),
            )?;
            return Err(RuntimeError::Internal(DEBUG_REQUEST_REJECTED.into()));
        }
        Ok(self.vm.as_ref().expect("checked above"))
    }

    fn debug_vm_mut(
        &mut self,
        message_id: u64,
    ) -> Result<&mut erabasic_vm::RuntimeVm, RuntimeError> {
        if self.vm.is_none() {
            self.emit_debug_error(
                DebugErrorCode::InvalidState,
                "runtime has no active VM",
                Some(message_id),
            )?;
            return Err(RuntimeError::Internal(DEBUG_REQUEST_REJECTED.into()));
        }
        match self.vm.as_mut() {
            Some(vm) => Ok(vm),
            None => unreachable!("VM presence was checked above"),
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    fn emit_vm_debug_error(
        &mut self,
        error: VmError,
        correlation: Option<u64>,
    ) -> Result<(), RuntimeError> {
        let code = match &error {
            VmError::ResourceLimit(_) => DebugErrorCode::ResourceLimit,
            VmError::InvalidArguments(_) | VmError::UnknownFiber(_) => {
                DebugErrorCode::UnknownTarget
            }
            VmError::InvalidState(message) if message.contains("stale") => {
                DebugErrorCode::StaleStop
            }
            _ => DebugErrorCode::InvalidState,
        };
        self.emit_debug_error(code, &error.to_string(), correlation)
    }

    fn emit_debug_error(
        &mut self,
        code: DebugErrorCode,
        message: &str,
        correlation_id: Option<u64>,
    ) -> Result<(), RuntimeError> {
        self.emit_log(
            RuntimeLogLevel::Error,
            format!("debug request failed [{code:?}]: {message}"),
        )?;
        self.emit_debug(
            DebugMessage::Error(DebugError {
                code,
                message: message.into(),
            }),
            correlation_id,
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    pub(super) fn emit_debug(
        &mut self,
        message: DebugMessage,
        correlation_id: Option<u64>,
    ) -> Result<(), RuntimeError> {
        let envelope = message.envelope(
            Some(self.options.session_id),
            Some(self.epoch),
            self.debug_outbound_sequence,
            self.next_message_id,
            correlation_id,
        )?;
        let bytes = encode_envelope(&envelope, self.options.wire_limits)?;
        self.outbound.push_back(bytes);
        self.debug_outbound_sequence = self.debug_outbound_sequence.saturating_add(1);
        self.next_message_id = self.next_message_id.saturating_add(1);
        Ok(())
    }

    pub(super) fn resume_debug_time(&mut self) {
        if let Some(sample) = self.debug_frontend_time_sample.take() {
            self.frontend_time_origin = Some((sample, self.logical_time_ns));
        }
    }

    fn variable_descriptor(&self, value: &VmDebugVariable) -> VariableDescriptor {
        let definition = self.vm.as_ref().and_then(|vm| {
            vm.vm()
                .artifact()
                .globals
                .iter()
                .find(|item| item.key == value.target.target.variable)
        });
        VariableDescriptor {
            symbol_key: ProtocolBytes::new(value.target.target.variable.0),
            name: value.name.clone(),
            storage: definition.map_or(VariableStorage::Global, |item| {
                protocol_storage(item.storage)
            }),
            value_kind: protocol_value(value.value.clone()).kind(),
            dimensions: definition.map_or_else(Vec::new, |item| item.dimensions.clone()),
            mutable: value.mutable,
        }
    }

    fn read_game_field(&self, key: &str) -> Option<GameFieldValue> {
        let value = match key {
            "input.message_skip" => DebugValue::Boolean(self.message_skip),
            "runtime.logical_time_ns" => {
                DebugValue::Integer(i64::try_from(self.logical_time_ns).unwrap_or(i64::MAX))
            }
            "runtime.phase" => DebugValue::String(format!("{:?}", self.phase)),
            "runtime.revision" => {
                DebugValue::Integer(i64::try_from(self.revision).unwrap_or(i64::MAX))
            }
            _ => return None,
        };
        Some(GameFieldValue {
            key: key.into(),
            value,
            revision: self.revision,
        })
    }

    fn debug_console(
        &mut self,
        message_id: u64,
        stop: StopToken,
        command: ConsoleCommand,
    ) -> Result<(), RuntimeError> {
        let vm_stop = self.validate_stop(stop, message_id)?;
        let (source, execute) = match command {
            ConsoleCommand::Evaluate { source } => (source, false),
            ConsoleCommand::ExecuteSafe { source } => (source, true),
        };
        let trimmed = source.trim();
        let variables = match self.debug_vm(message_id)?.variables(vm_stop, None, 1024) {
            Ok(page) => page.values,
            Err(error) => return self.emit_vm_debug_error(error, Some(message_id)),
        };
        let mut value = None;
        let mut changed_variables = Vec::new();
        let mut diagnostics = Vec::new();
        if execute {
            let Some((target_name, expression)) = trimmed.split_once('=') else {
                diagnostics.push(console_diagnostic(
                    "debug.console.unsafe_statement",
                    "only a single EraBasic assignment is accepted by the safe console",
                ));
                return self.emit_console_outcome(
                    message_id,
                    stop,
                    value,
                    changed_variables,
                    diagnostics,
                );
            };
            let target_name = target_name.trim();
            let Some(target) = variables
                .iter()
                .find(|item| item.name.eq_ignore_ascii_case(target_name))
            else {
                diagnostics.push(console_diagnostic(
                    "debug.console.unknown_variable",
                    "assignment target is not a visible scalar variable",
                ));
                return self.emit_console_outcome(
                    message_id,
                    stop,
                    value,
                    changed_variables,
                    diagnostics,
                );
            };
            let parsed = match parse_console_expression(expression.trim(), &variables) {
                Ok(value) => value,
                Err((code, message)) => {
                    diagnostics.push(console_diagnostic(code, &message));
                    return self.emit_console_outcome(
                        message_id,
                        stop,
                        value,
                        changed_variables,
                        diagnostics,
                    );
                }
            };
            let writes = [VmDebugVariableWrite {
                target: target.target.clone(),
                value: parsed,
                expected_revision: target.revision,
            }];
            match self
                .debug_vm_mut(message_id)?
                .write_variables(vm_stop, &writes)
            {
                Ok(values) => {
                    self.revision = self.revision.saturating_add(1);
                    changed_variables = values.into_iter().map(protocol_variable_value).collect();
                }
                Err(error) => return self.emit_vm_debug_error(error, Some(message_id)),
            }
        } else {
            match parse_console_expression(trimmed, &variables) {
                Ok(parsed) => value = Some(protocol_value(parsed)),
                Err((code, message)) => diagnostics.push(console_diagnostic(code, &message)),
            }
        }
        self.emit_console_outcome(message_id, stop, value, changed_variables, diagnostics)
    }

    fn emit_console_outcome(
        &mut self,
        message_id: u64,
        stop: StopToken,
        value: Option<DebugValue>,
        changed_variables: Vec<VariableValue>,
        diagnostics: Vec<DebugDiagnostic>,
    ) -> Result<(), RuntimeError> {
        let stop = self.refreshed_stop(stop);
        self.emit_debug(
            DebugMessage::Response(DebugResponse::Console(ConsoleOutcome {
                stop,
                value,
                output: Vec::new(),
                changed_variables,
                changed_game_fields: Vec::new(),
                diagnostics,
            })),
            Some(message_id),
        )
    }
}

fn console_diagnostic(code: &str, message: &str) -> DebugDiagnostic {
    DebugDiagnostic {
        code: code.into(),
        message: message.into(),
        source: None,
    }
}

fn parse_console_expression(
    source: &str,
    variables: &[VmDebugVariable],
) -> Result<VmValue, (&'static str, String)> {
    let mut context = DefaultParserContext::default();
    for variable in variables {
        context.register_variable(&variable.name);
    }
    for function in PURE_CONSOLE_METHODS {
        context.register_function(function);
    }
    let parsed = parse_expression(source, &context);
    if parsed.has_errors() {
        let message = parsed
            .diagnostics
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(("debug.console.parse_error", message));
    }
    let expression = parsed.value.ok_or_else(|| {
        (
            "debug.console.parse_error",
            "expression parser produced no value".into(),
        )
    })?;
    evaluate_console_expression(&expression, variables)
}

fn evaluate_console_expression(
    expression: &Expr,
    variables: &[VmDebugVariable],
) -> Result<VmValue, (&'static str, String)> {
    match &expression.kind {
        ExprKind::Integer(value) => Ok(VmValue::Integer(*value)),
        ExprKind::String(value) => Ok(VmValue::String(value.clone())),
        ExprKind::Identifier(name) => variables
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(name))
            .map(|item| item.value.clone())
            .ok_or_else(|| {
                (
                    "debug.console.unknown_variable",
                    format!("{name} is not a visible scalar variable"),
                )
            }),
        ExprKind::Variable { name, indices } if indices.is_empty() => variables
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(name))
            .map(|item| item.value.clone())
            .ok_or_else(|| {
                (
                    "debug.console.unknown_variable",
                    format!("{name} is not a visible scalar variable"),
                )
            }),
        ExprKind::Variable { .. } => Err((
            "debug.console.unsupported_expression",
            "indexed variable reads are not in the safe console subset".into(),
        )),
        ExprKind::Group(inner) => evaluate_console_expression(inner, variables),
        ExprKind::Unary { op, operand } => {
            let evaluated = evaluate_console_expression(operand, variables)?;
            let value = console_integer(&evaluated)?;
            match op {
                UnaryOp::Plus => Ok(VmValue::Integer(value)),
                UnaryOp::Minus => Ok(VmValue::Integer(value.wrapping_neg())),
                UnaryOp::LogicalNot => Ok(VmValue::Integer(i64::from(value == 0))),
                UnaryOp::BitNot => Ok(VmValue::Integer(!value)),
                UnaryOp::PreIncrement | UnaryOp::PreDecrement => Err((
                    "debug.console.unsafe_expression",
                    "increment and decrement are not allowed in the transactional console".into(),
                )),
            }
        }
        ExprKind::Postfix { .. } => Err((
            "debug.console.unsafe_expression",
            "increment and decrement are not allowed in the transactional console".into(),
        )),
        ExprKind::Binary { op, left, right } => {
            let left = evaluate_console_expression(left, variables)?;
            let right = evaluate_console_expression(right, variables)?;
            evaluate_console_binary(*op, &left, &right)
        }
        ExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            let evaluated = evaluate_console_expression(condition, variables)?;
            let condition = console_integer(&evaluated)?;
            evaluate_console_expression(
                if condition != 0 { then_expr } else { else_expr },
                variables,
            )
        }
        ExprKind::Call { name, args } => {
            let values = args
                .iter()
                .map(|argument| {
                    argument
                        .as_ref()
                        .ok_or_else(|| {
                            (
                                "debug.console.unsupported_expression",
                                "omitted method arguments are not supported".into(),
                            )
                        })
                        .and_then(|argument| evaluate_console_expression(argument, variables))
                })
                .collect::<Result<Vec<_>, _>>()?;
            evaluate_console_method(name, &values)
        }
        ExprKind::Formatted(_) => Err((
            "debug.console.unsupported_expression",
            "formatted strings are not part of the safe console subset".into(),
        )),
        ExprKind::Error => Err(("debug.console.parse_error", "invalid expression".into())),
    }
}

fn evaluate_console_binary(
    op: BinaryOp,
    left: &VmValue,
    right: &VmValue,
) -> Result<VmValue, (&'static str, String)> {
    if matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) {
        let equal = left == right;
        return Ok(VmValue::Integer(i64::from(if op == BinaryOp::Equal {
            equal
        } else {
            !equal
        })));
    }
    let left = console_integer(left)?;
    let right = console_integer(right)?;
    // EraBasic follows the CLR's masked 64-bit shift-count behavior.
    let shift = u32::try_from(right & 63).expect("masked shift count fits u32");
    let value = match op {
        BinaryOp::Multiply => left.wrapping_mul(right),
        BinaryOp::Divide if right != 0 => left.wrapping_div(right),
        BinaryOp::Modulo if right != 0 => left.wrapping_rem(right),
        BinaryOp::Divide | BinaryOp::Modulo => {
            return Err(("debug.console.execution_error", "division by zero".into()));
        }
        BinaryOp::Add => left.wrapping_add(right),
        BinaryOp::Subtract => left.wrapping_sub(right),
        BinaryOp::ShiftLeft => left.wrapping_shl(shift),
        BinaryOp::ShiftRight => left.wrapping_shr(shift),
        BinaryOp::Less => i64::from(left < right),
        BinaryOp::LessEqual => i64::from(left <= right),
        BinaryOp::Greater => i64::from(left > right),
        BinaryOp::GreaterEqual => i64::from(left >= right),
        BinaryOp::BitAnd => left & right,
        BinaryOp::BitXor => left ^ right,
        BinaryOp::BitOr => left | right,
        BinaryOp::LogicalAnd => i64::from(left != 0 && right != 0),
        BinaryOp::LogicalXor => i64::from((left != 0) ^ (right != 0)),
        BinaryOp::LogicalOr => i64::from(left != 0 || right != 0),
        BinaryOp::Nand => i64::from(!(left != 0 && right != 0)),
        BinaryOp::Nor => i64::from(!(left != 0 || right != 0)),
        BinaryOp::Equal | BinaryOp::NotEqual => unreachable!("handled above"),
    };
    Ok(VmValue::Integer(value))
}

fn evaluate_console_method(
    name: &str,
    values: &[VmValue],
) -> Result<VmValue, (&'static str, String)> {
    let upper = name.to_ascii_uppercase();
    if !PURE_CONSOLE_METHODS.contains(&upper.as_str()) {
        return Err((
            "debug.console.unsafe_method",
            format!("{name} is not in the debugger's pure method whitelist"),
        ));
    }
    evaluate_pure_native(&upper, values.to_vec())
        .map_err(|message| ("debug.console.execution_error", message))
}

const PURE_CONSOLE_METHODS: [&str; 35] = [
    "ABS",
    "SIGN",
    "SQRT",
    "CBRT",
    "LOG",
    "LOG10",
    "EXPONENT",
    "POWER",
    "GETBIT",
    "BITCOUNT",
    "STRLEN",
    "STRLENU",
    "TOINT",
    "ISNUMERIC",
    "UNICODE",
    "CONVERT",
    "COLOR_FROMRGB",
    "MAX",
    "MIN",
    "LIMIT",
    "INRANGE",
    "TOSTR",
    "SUBSTRING",
    "SUBSTRINGU",
    "STRFIND",
    "STRFINDU",
    "STRCOUNT",
    "STRLENS",
    "STRLENSU",
    "REPLACE",
    "ESCAPE",
    "UNICODETOSTR",
    "ENCODETOUNI",
    "UNICODEBYTE",
    "CHARATU",
];

fn console_integer(value: &VmValue) -> Result<i64, (&'static str, String)> {
    match value {
        VmValue::Integer(value) => Ok(*value),
        _ => console_type_error("integer"),
    }
}

fn console_type_error<T>(expected: &str) -> Result<T, (&'static str, String)> {
    Err((
        "debug.console.type_mismatch",
        format!("safe expression expected an {expected} value"),
    ))
}

fn all_debug_scopes() -> [DebugScope; 10] {
    [
        DebugScope::VariablesRead,
        DebugScope::VariablesWrite,
        DebugScope::GameFieldsRead,
        DebugScope::GameFieldsWrite,
        DebugScope::ExecutionRead,
        DebugScope::ExecutionControl,
        DebugScope::ConsoleEvaluate,
        DebugScope::ConsoleExecute,
        DebugScope::BreakpointsManage,
        DebugScope::ScriptOutput,
    ]
}

fn scope_bit(scope: DebugScope) -> u64 {
    1_u64
        << match scope {
            DebugScope::VariablesRead => 0,
            DebugScope::VariablesWrite => 1,
            DebugScope::GameFieldsRead => 2,
            DebugScope::GameFieldsWrite => 3,
            DebugScope::ExecutionRead => 4,
            DebugScope::ExecutionControl => 5,
            DebugScope::ConsoleEvaluate => 6,
            DebugScope::ConsoleExecute => 7,
            DebugScope::BreakpointsManage => 8,
            DebugScope::ScriptOutput => 9,
        }
}

fn command_scope(command: &DebugCommand) -> DebugScope {
    match command {
        DebugCommand::Pause | DebugCommand::Continue { .. } | DebugCommand::Step { .. } => {
            DebugScope::ExecutionControl
        }
        DebugCommand::ListVariables { .. } | DebugCommand::ReadVariable { .. } => {
            DebugScope::VariablesRead
        }
        DebugCommand::WriteVariables { .. } => DebugScope::VariablesWrite,
        DebugCommand::ListGameFields { .. } | DebugCommand::ReadGameField { .. } => {
            DebugScope::GameFieldsRead
        }
        DebugCommand::WriteGameFields { .. } => DebugScope::GameFieldsWrite,
        DebugCommand::ListFibers { .. }
        | DebugCommand::ReadCallStack { .. }
        | DebugCommand::ReadOperandStack { .. } => DebugScope::ExecutionRead,
        DebugCommand::Console {
            command: ConsoleCommand::Evaluate { .. },
            ..
        } => DebugScope::ConsoleEvaluate,
        DebugCommand::Console {
            command: ConsoleCommand::ExecuteSafe { .. },
            ..
        } => DebugScope::ConsoleExecute,
        DebugCommand::UpdateBreakpoints { .. } => DebugScope::BreakpointsManage,
        DebugCommand::ReadScriptOutput { .. } | DebugCommand::SubscribeScriptOutput { .. } => {
            DebugScope::ScriptOutput
        }
    }
}

fn next_char_boundary(value: &str, mut index: usize) -> usize {
    while index < value.len() && !value.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn previous_char_boundary(value: &str, mut index: usize) -> usize {
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod console_tests {
    use super::*;

    #[test]
    fn safe_console_uses_erabasic_precedence_and_pure_methods() {
        assert_eq!(
            parse_console_expression("1 + 2 * 3", &[]),
            Ok(VmValue::Integer(7))
        );
        assert_eq!(
            parse_console_expression("ABS(-4) + MAX(2, 5)", &[]),
            Ok(VmValue::Integer(9))
        );
        assert_eq!(
            parse_console_expression("STRLENS(\"界\")", &[]),
            Ok(VmValue::Integer(3))
        );
        assert_eq!(
            parse_console_expression("STRLENSU(\"😀\")", &[]),
            Ok(VmValue::Integer(2))
        );
    }

    #[test]
    fn safe_console_rejects_failed_or_non_whitelisted_work_before_commit() {
        assert!(matches!(
            parse_console_expression("1 / 0", &[]),
            Err(("debug.console.execution_error", _))
        ));
        assert!(parse_console_expression("GETKEY(1)", &[]).is_err());
    }
}

fn usize_cursor(cursor: Option<u64>) -> Result<Option<usize>, RuntimeError> {
    cursor
        .map(|value| {
            usize::try_from(value)
                .map_err(|_| RuntimeError::ResourceLimit("debug cursor is too large"))
        })
        .transpose()
}

fn vm_step_kind(kind: StepKind) -> VmStepKind {
    match kind {
        StepKind::Instruction => VmStepKind::Instruction,
        StepKind::SourceLine => VmStepKind::SourceLine,
        StepKind::Into => VmStepKind::Into,
        StepKind::Over => VmStepKind::Over,
        StepKind::Out => VmStepKind::Out,
    }
}

fn protocol_source(source: erabasic_bytecode::ResolvedSourceLocation) -> DebugSourceLocation {
    DebugSourceLocation {
        relative_path: source.relative_path,
        content_hash: ProtocolBytes::new(source.content_hash.0),
        byte_start: source.byte_start,
        byte_end: source.byte_end,
        line: source.line,
        byte_column: source.byte_column,
    }
}

fn protocol_fiber(fiber: &erabasic_vm::VmDebugFiber) -> FiberSummary {
    let state = match &fiber.status {
        FiberStatus::Runnable => FiberState::Runnable,
        FiberStatus::WaitingHost(_) => FiberState::WaitingHost,
        FiberStatus::WaitingResume => FiberState::WaitingResume,
        FiberStatus::Completed(_) => FiberState::Completed,
        FiberStatus::Faulted(_) => FiberState::Faulted,
        FiberStatus::Cancelled => FiberState::Cancelled,
    };
    FiberSummary {
        fiber_id: fiber.id.0,
        state,
        primary: fiber.primary,
        frame_count: u32::try_from(fiber.frame_count).unwrap_or(u32::MAX),
    }
}

fn protocol_frame(frame: erabasic_vm::VmDebugFrame) -> FrameSummary {
    FrameSummary {
        frame_id: frame.id.0,
        generation: frame.generation.0,
        function_key: ProtocolBytes::new(frame.function.0),
        function_name: frame.function_name,
        instruction: frame.instruction,
        source: frame.source.map(protocol_source),
    }
}

fn protocol_value(value: VmValue) -> DebugValue {
    protocol_value_in_generation(value, 0)
}

fn protocol_value_in_generation(value: VmValue, generation: u64) -> DebugValue {
    match value {
        VmValue::Integer(value) => DebugValue::Integer(value),
        VmValue::String(value) => DebugValue::String(value),
        VmValue::IntegerPlace(place) => {
            DebugValue::Place(protocol_place(*place, ValueKind::Integer, generation))
        }
        VmValue::StringPlace(place) => {
            DebugValue::Place(protocol_place(*place, ValueKind::String, generation))
        }
    }
}

fn protocol_place(
    place: PlaceDescriptor,
    value_kind: ValueKind,
    generation: u64,
) -> era_debug_protocol::DebugPlace {
    era_debug_protocol::DebugPlace {
        symbol_key: ProtocolBytes::new(place.variable.0),
        value_kind,
        indices: place.indices,
        character: place.character,
        fiber_id: place.fiber.map(|value| value.0),
        frame_id: place.frame.map(|value| value.0),
        generation,
    }
}

fn vm_value(value: &DebugValue) -> Result<VmValue, &'static str> {
    match value {
        DebugValue::Integer(value) => Ok(VmValue::Integer(*value)),
        DebugValue::String(value) => Ok(VmValue::String(value.clone())),
        _ => Err("VM variables accept only integer or string values"),
    }
}

fn vm_variable_reference(value: &VariableReference) -> Result<VmDebugVariableRef, &'static str> {
    let bytes: [u8; 16] = value
        .symbol_key
        .as_slice()
        .try_into()
        .map_err(|_| "variable symbol key must contain 16 bytes")?;
    Ok(VmDebugVariableRef {
        target: PlaceDescriptor {
            variable: SymbolKey(bytes),
            indices: value.indices.clone(),
            character: value.character,
            fiber: value.fiber_id.map(FiberId),
            frame: value.frame_id.map(FrameId),
        },
        generation: GenerationId(value.generation),
    })
}

fn protocol_variable_value(value: VmDebugVariable) -> VariableValue {
    let storage = if value.target.target.fiber.is_some() {
        VariableStorage::Local
    } else if value.target.target.character.is_some() {
        VariableStorage::Character
    } else {
        VariableStorage::Global
    };
    VariableValue {
        reference: VariableReference {
            symbol_key: ProtocolBytes::new(value.target.target.variable.0),
            storage,
            fiber_id: value.target.target.fiber.map(|item| item.0),
            frame_id: value.target.target.frame.map(|item| item.0),
            generation: value.target.generation.0,
            character: value.target.target.character,
            indices: value.target.target.indices,
        },
        value: protocol_value(value.value),
        revision: value.revision,
    }
}

fn protocol_storage(storage: BytecodeStorage) -> VariableStorage {
    match storage {
        BytecodeStorage::FunctionLocal => VariableStorage::Local,
        BytecodeStorage::FunctionStatic => VariableStorage::FunctionStatic,
        BytecodeStorage::Character => VariableStorage::Character,
        _ => VariableStorage::Global,
    }
}

fn game_field_descriptors() -> Vec<GameFieldDescriptor> {
    vec![
        GameFieldDescriptor {
            key: "input.message_skip".into(),
            value_kind: ValueKind::Boolean,
            mutability: FieldMutability::DebugWritable,
            description: "Runtime-owned message-skip latch".into(),
        },
        GameFieldDescriptor {
            key: "runtime.logical_time_ns".into(),
            value_kind: ValueKind::Integer,
            mutability: FieldMutability::ReadOnly,
            description: "Authoritative logical clock".into(),
        },
        GameFieldDescriptor {
            key: "runtime.phase".into(),
            value_kind: ValueKind::String,
            mutability: FieldMutability::ReadOnly,
            description: "Current runtime lifecycle phase".into(),
        },
        GameFieldDescriptor {
            key: "runtime.revision".into(),
            value_kind: ValueKind::Integer,
            mutability: FieldMutability::ReadOnly,
            description: "Runtime mutation revision".into(),
        },
    ]
}

fn vm_breakpoint(value: &Breakpoint) -> Result<VmBreakpoint, &'static str> {
    let location = match &value.location {
        BreakpointLocation::Function { symbol_key } => {
            let bytes: [u8; 16] = symbol_key
                .as_slice()
                .try_into()
                .map_err(|_| "function symbol key must contain 16 bytes")?;
            VmBreakpointLocation::Function(SymbolKey(bytes))
        }
        BreakpointLocation::Source {
            relative_path,
            content_hash,
            byte_offset,
        } => {
            let bytes: [u8; 32] = content_hash
                .as_slice()
                .try_into()
                .map_err(|_| "source content hash must contain 32 bytes")?;
            VmBreakpointLocation::Source {
                relative_path: relative_path.clone(),
                content_hash: Digest(bytes),
                byte_offset: *byte_offset,
            }
        }
    };
    Ok(VmBreakpoint {
        id: value.breakpoint_id,
        enabled: value.enabled,
        hit_count: 0,
        location,
    })
}

fn protocol_breakpoint(value: VmResolvedBreakpoint) -> ResolvedBreakpoint {
    ResolvedBreakpoint {
        breakpoint_id: value.id,
        generation: value.generation.0,
        binding: match value.binding {
            VmBreakpointBinding::Verified => BreakpointBinding::Verified,
            VmBreakpointBinding::Moved => BreakpointBinding::Moved,
            VmBreakpointBinding::Unbound => BreakpointBinding::Unbound,
        },
        source: value.source.map(protocol_source),
        message: value.message,
        hit_count: value.hit_count,
    }
}
