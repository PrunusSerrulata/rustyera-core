impl RuntimeSession {
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
        let program_generation = self.vm.as_ref().map_or(0, |vm| vm.current_generation().0);
        self.renew_debug_grant_for_generation(program_generation)
    }

    pub(in crate::session) fn renew_debug_grant_for_generation(
        &mut self,
        program_generation: u64,
    ) -> Result<(), RuntimeError> {
        let Some(previous) = self.active_debug_grant.clone() else {
            return Ok(());
        };
        let token = GrantToken {
            grant_id: SessionId {
                high: self.options.session_id.high ^ 0x4445_4255_4747_5241,
                low: self.next_debug_grant_id,
            },
            session_epoch: self.epoch.0,
            program_generation,
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
}
