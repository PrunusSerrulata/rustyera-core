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

}
