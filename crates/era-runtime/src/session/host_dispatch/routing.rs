impl RuntimeSession {
    #[allow(clippy::single_match_else, clippy::too_many_lines)]
    pub(super) fn handle_host_call(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        match self.handle_host_call_inner(vm, request) {
            Err(RuntimeError::Script { kind, message }) => {
                complete_script_fault(vm, request, kind, message)
            }
            result => result,
        }
    }

    #[allow(clippy::single_match_else, clippy::too_many_lines)]
    fn handle_host_call_inner(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        if let Some(time) = self.candidate_clock {
            match request.import.contract.candidate {
                erabasic_bytecode::CandidatePolicy::Forbidden => {
                    return Err(RuntimeError::Internal(format!(
                        "{} is forbidden during candidate SAVEINFO execution",
                        request.import.import.name
                    )));
                }
                erabasic_bytecode::CandidatePolicy::FrozenClock => {
                    return complete_frozen_clock(vm, request, time);
                }
                erabasic_bytecode::CandidatePolicy::ReadOnly
                | erabasic_bytecode::CandidatePolicy::CloneCommit
                | erabasic_bytecode::CandidatePolicy::BufferedEffect => {}
            }
        }
        if request
            .import
            .import
            .namespace
            .eq_ignore_ascii_case("rustyera.extension")
        {
            return self.issue_extension(vm, request);
        }
        if request
            .import
            .import
            .namespace
            .eq_ignore_ascii_case(SQL_OPERATION)
        {
            return self.dispatch_sql(vm, request);
        }
        let name = request.import.import.name.to_ascii_uppercase();
        if self.dispatch_input_extensions(vm, request, &name)? {
            return Ok(());
        }
        if name == "SKIPDISP" {
            self.skip_print = integer_argument_value(request, 0)? != 0;
            self.user_defined_skip = self.skip_print;
            // Host calls execute while the caller-pumped drive loop temporarily
            // owns the VM, so RESULT must be resolved through that VM rather than
            // through the session's temporarily empty VM slot.
            return commit_host_result_write(vm, request.id, i64::from(self.skip_print));
        }
        if name == "SKIPLOG" {
            self.message_skip = integer_argument_value(request, 0)? != 0;
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "NOSKIP" {
            self.saved_skip = self.skip_print;
            self.skip_print = false;
            return commit_integer_result(vm, request.id, 1);
        }
        if name == "ENDNOSKIP" {
            if self.saved_skip {
                self.skip_print = true;
            }
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if matches!(name.as_str(), "GETDISPLAYLINE" | "HTML_GETPRINTEDSTR")
            && request.omitted_arguments.contains(&0)
        {
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                "display query dereferenced an omitted source argument",
            );
        }
        match evaluate_runtime_query(
            &name,
            request,
            &self.presentation,
            RuntimeQueryState {
                skip_print: self.skip_print,
                message_skip: self.message_skip,
                snake_display_state: self.project_snapshot.as_ref().is_some_and(|project| {
                    project
                        .manifest
                        .compatibility
                        .supports_snake_display_state()
                }),
            },
        )? {
            RuntimeQueryEvaluation::Ready(value) => {
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady {
                        value: Some(value),
                        writes: Vec::new(),
                    }),
                );
            }
            RuntimeQueryEvaluation::MalformedHtml => {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Parse,
                    "malformed HTML text",
                );
            }
            RuntimeQueryEvaluation::InvalidPrintedHtmlIndex => {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "HTML_GETPRINTEDSTR line number must be non-negative",
                );
            }
            RuntimeQueryEvaluation::Unhandled => {}
        }
        if name == "ASSERT" {
            if integer_argument_value(request, 0)? == 0 {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Assertion,
                    "ASSERT failed",
                );
            }
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "THROW" {
            let message = request.argument(0).map_or_else(String::new, display_value);
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::ExplicitThrow,
                &message,
            );
        }
        if name == "FORCEKANA" {
            let mode = integer_argument_value(request, 0)?;
            let Ok(mode) = u8::try_from(mode) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Argument,
                    "FORCEKANA mode must be between 0 and 3",
                );
            };
            if mode > 3 {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Argument,
                    "FORCEKANA mode must be between 0 and 3",
                );
            }
            self.force_kana_mode = mode;
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if matches!(name.as_str(), "UPCHECK" | "CUPCHECK") {
            let (character, character_scoped) = if name == "CUPCHECK" {
                let Ok(character) = u64::try_from(integer_argument_value(request, 0)?) else {
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Bounds,
                        "character index is negative",
                    );
                };
                (character, true)
            } else {
                let target = read_runtime_integer(vm, "TARGET", &[], None)?;
                let Ok(character) = u64::try_from(target) else {
                    clear_upcheck_arrays(vm, false, None)?;
                    return commit_completion(
                        vm,
                        request.id,
                        VmHostCompletion::Ready(HostReady::empty()),
                    );
                };
                (character, false)
            };
            let lines = apply_upcheck(vm, character, character_scoped)?;
            if !self.skip_print {
                for line in lines {
                    self.presentation.append_print_text(line, false, true);
                }
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "ISACTIVE" {
            let value = self.client_focused;
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(i64::from(value))),
                    writes: Vec::new(),
                }),
            );
        }
        if name == "SETANIMETIMER" {
            let milliseconds = integer_argument_value(request, 0)?;
            let (snake, normalized) = {
                let project = self
                    .project_snapshot
                    .as_mut()
                    .ok_or_else(|| RuntimeError::Internal("SETANIMETIMER has no project".into()))?;
                if !project.resource_graph.set_animation_timer(milliseconds) {
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Bounds,
                        "SETANIMETIMER expects a value between i32::MIN and 32767",
                    );
                }
                (
                    project
                        .manifest
                        .compatibility
                        .supports_snake_display_state(),
                    project.resource_graph.animation_timer(),
                )
            };
            // The timer is logical runtime state, so publish it immediately instead of
            // waiting for a graphics replay boundary that may never occur before a fault.
            self.presentation.set_animation_timer(normalized);
            if snake {
                commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            } else {
                commit_integer_result(vm, request.id, 1)?;
            }
            return self.emit_presentation();
        }
        if name == "GETANIMETIMER" {
            let value = self
                .project_snapshot
                .as_ref()
                .ok_or_else(|| RuntimeError::Internal("GETANIMETIMER has no project".into()))?
                .resource_graph
                .animation_timer();
            return commit_integer_result(vm, request.id, i64::from(value));
        }
        if self.controller.step == SystemStep::TrainEventComEnd
            && matches!(name.as_str(), "WAIT" | "WAITANYKEY" | "FORCEWAIT" | "TWAIT")
        {
            self.controller.event_com_end_wait_required = false;
        }
        if self.skip_print && is_runtime_print_command(&name) {
            if self.user_defined_skip && is_input_command(&name) {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Operation,
                    "an input command cannot execute while user SKIPDISP is active; wrap it in NOSKIP/ENDNOSKIP",
                );
            }
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        let mut status = HostDispatchStatus::Unhandled;
        let result = self.dispatch_control(vm, request, &name, &mut status);
        if status == HostDispatchStatus::Handled {
            return result;
        }
        let result = self.dispatch_storage(vm, request, &name, &mut status);
        if status == HostDispatchStatus::Handled {
            return result;
        }
        let result = self.dispatch_presentation(vm, request, &name, &mut status);
        if status == HostDispatchStatus::Handled {
            return result;
        }
        let result = self.dispatch_graphics(vm, request, &name, &mut status);
        if status == HostDispatchStatus::Handled {
            return result;
        }
        self.dispatch_services(vm, request, &name)
    }

    pub(in crate::session) fn require_host_service(
        &mut self,
        request: &VmHostRequest,
        kind: ServiceKind,
        operation: &str,
        operation_version: ProtocolVersion,
    ) -> Result<bool, RuntimeError> {
        if self.service_capabilities.get(&(kind, operation.to_owned())) != Some(&operation_version)
        {
            self.fault_with_context(
                FaultCode::UnsupportedRuntimeFeature,
                &format!(
                    "frontend did not negotiate service {kind:?}/{operation} {operation_version:?}"
                ),
                Some(request.origin.clone()),
                Some(Box::new(
                    era_runtime_protocol::CompatibilityDiagnosticContext {
                        artifact: None,
                        project_load_id: None,
                        runtime_epoch: None,
                        generation: None,
                        identity: self
                            .project_snapshot
                            .as_ref()
                            .map(|project| project.manifest.compatibility.clone()),
                        stage: "service".into(),
                        api: Some(request.import.import.name.clone()),
                        required_capability: Some(era_runtime_protocol::RequiredCapability {
                            kind,
                            operation: operation.into(),
                            version: operation_version,
                        }),
                    },
                )),
            )?;
            return Ok(false);
        }
        Ok(true)
    }

    // The typed operation tuple is deliberately explicit at this single protocol edge.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn issue_host_service<T: minicbor::Encode<()>>(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        completion: ExternalCompletion,
        kind: ServiceKind,
        operation: &str,
        operation_version: ProtocolVersion,
        payload: &T,
    ) -> Result<(), RuntimeError> {
        if !self.require_host_service(request, kind, operation, operation_version)? {
            return Ok(());
        }
        commit_completion(
            vm,
            request.id,
            VmHostCompletion::Pending {
                stability: HostWaitStability::Transient,
                rebind_payload: Vec::new(),
            },
        )?;
        let request_id = self.allocate_request()?;
        self.operations
            .insert_service(request_id, PendingService::Host(completion));
        self.emit(
            RuntimeMessage::ServiceRequest(ServiceRequest {
                request_id,
                kind,
                operation: operation.into(),
                operation_version,
                payload: ProtocolBytes::new(encode_canonical(payload)?),
                deadline_ns: None,
            }),
            None,
        )
    }

    pub(in crate::session) fn projection_query_context(&self) -> ProjectionQueryContext {
        ProjectionQueryContext {
            presentation_revision: self.presentation.revision(),
            environment_revision: self.projection_environment_revision,
            projection_space_revision: self.projection_space_revision,
        }
    }

    pub(in crate::session) fn presentation_observation_context(
        &mut self,
    ) -> Result<ProjectionQueryContext, RuntimeError> {
        // A presentation query is an observation barrier: its frontend response must describe
        // the canonical state that existed when the request was issued, even while ordinary
        // animation frames are being collapsed during a continuous message skip.
        self.flush_presentation_for_observation()?;
        Ok(self.projection_query_context())
    }

    fn issue_extension(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        let operation = request.import.import.name.to_ascii_lowercase();
        let declaration = self
            .project_snapshot
            .as_ref()
            .and_then(|project| project.extensions.get(&operation))
            .cloned()
            .ok_or_else(|| {
                RuntimeError::Internal(format!("extension import {operation} has no declaration"))
            })?;
        let mut arguments = Vec::with_capacity(request.arguments.len());
        let mut mutable_places = Vec::with_capacity(request.arguments.len());
        for (ordinal, argument) in request.arguments.iter().enumerate() {
            let (value, place) = match argument {
                VmValue::Integer(value) => {
                    (era_runtime_protocol::ProtocolValue::Integer(*value), None)
                }
                VmValue::String(value) => (
                    era_runtime_protocol::ProtocolValue::String(value.clone()),
                    None,
                ),
                VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => {
                    let value = vm
                        .read_host_place(request.fiber, place)
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                    let value = materialized_protocol_value(&value).ok_or_else(|| {
                        RuntimeError::Internal(
                            "reading an extension place returned another place".into(),
                        )
                    })?;
                    let declared_type = declaration
                        .arguments
                        .get(ordinal)
                        .map_or(era_runtime_protocol::ExtensionValueType::Any, |argument| {
                            argument.value_type
                        });
                    (value, Some((place.as_ref().clone(), declared_type)))
                }
            };
            arguments.push(value);
            mutable_places.push(place);
        }
        let invocation = era_runtime_protocol::ExtensionInvocation {
            extension_id: declaration.id,
            arguments,
        };
        self.issue_host_service(
            vm,
            request,
            ExternalCompletion::Extension {
                request: request.id,
                return_type: declaration.return_type,
                mutable_places,
            },
            ServiceKind::Extension,
            &declaration.operation,
            declaration.operation_version,
            &invocation,
        )
    }

    pub(super) fn issue_platform_effect<T: minicbor::Encode<()>>(
        &mut self,
        kind: ServiceKind,
        operation: &str,
        operation_version: ProtocolVersion,
        payload: &T,
    ) -> Result<(), RuntimeError> {
        if self.service_capabilities.get(&(kind, operation.to_owned())) != Some(&operation_version)
        {
            return self.emit(
                RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                    context: Some(Box::new(
                        era_runtime_protocol::CompatibilityDiagnosticContext {
                            artifact: None,
                            project_load_id: None,
                            runtime_epoch: None,
                            generation: None,
                            identity: self
                                .project_snapshot
                                .as_ref()
                                .map(|project| project.manifest.compatibility.clone()),
                            stage: "service".into(),
                            api: None,
                            required_capability: Some(era_runtime_protocol::RequiredCapability {
                                kind,
                                operation: operation.into(),
                                version: operation_version,
                            }),
                        },
                    )),
                    code: "runtime.platform_capability_unavailable".into(),
                    level: RuntimeLogLevel::Warning,
                    message: format!("frontend did not negotiate service {kind:?}/{operation}"),
                    source: None,
                    notification: DiagnosticNotification::default(),
                }),
                None,
            );
        }
        let request_id = self.allocate_request()?;
        self.operations.insert_service(
            request_id,
            PendingService::PlatformEffect {
                operation: operation.into(),
            },
        );
        self.emit(
            RuntimeMessage::ServiceRequest(ServiceRequest {
                request_id,
                kind,
                operation: operation.into(),
                operation_version,
                payload: ProtocolBytes::new(encode_canonical(payload)?),
                deadline_ns: None,
            }),
            None,
        )
    }

    pub(super) fn issue_host_storage(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        pending: PendingStorage,
        namespace: StorageNamespace,
        operation: StorageOperation,
        relative_path: String,
    ) -> Result<(), RuntimeError> {
        commit_completion(
            vm,
            request.id,
            VmHostCompletion::Pending {
                stability: HostWaitStability::Transient,
                rebind_payload: Vec::new(),
            },
        )?;
        self.issue_storage(pending, namespace, operation, relative_path)
    }
}
