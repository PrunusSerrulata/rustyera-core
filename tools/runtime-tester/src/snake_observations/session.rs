use super::input::{KeyState, parse_key_events};
use super::{AuditResult, COMPLETE, line_text};
use era_debug_protocol::{
    AuthorizedDebugRequest, DEBUG_PROTOCOL_VERSION, DebugCommand, DebugHello, DebugMessage,
    DebugResponse, DebugScope, DebugValue, GrantToken, StopToken, VariableReference,
    VariableStorage,
};
use era_protocol::{
    Channel, ProtocolBytes, SessionEpoch, SessionId, VersionRange, decode_canonical,
    decode_envelope, encode_canonical, encode_envelope,
};
use era_runtime::{ProjectProgressReporter, RuntimeDriveBudget, RuntimeOptions, RuntimeSession};
use era_runtime_protocol::{
    ClientCapabilities, ClientHello, ClientStateChanged, DeviceStateChanged, DisplayLine,
    GET_KEY_STATE_OPERATION, GET_KEY_STATE_OPERATION_VERSION, GetKeyStateRequest,
    GetKeyStateResponse, InputDeviceKind, InputModality, InputWait, ProjectLoadReport,
    RUNTIME_PROTOCOL_VERSION, RuntimeFeature, RuntimeMessage, ServiceCapability, ServiceKind,
    ServiceResponse, ServiceResult, StorageCapabilities, WaitChange,
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub(super) struct ObservationSession {
    pub(super) session: RuntimeSession,
    session_id: Option<SessionId>,
    epoch: Option<SessionEpoch>,
    sequence: u64,
    debug_sequence: u64,
    pub(super) load: Option<ProjectLoadReport>,
    pub(super) wait: Option<InputWait>,
    pub(super) lines: Vec<DisplayLine>,
    pub(super) diagnostics: Vec<Value>,
    pub(super) setup_diagnostics: Vec<Value>,
    pub(super) host_logs: Vec<Value>,
    pub(super) setup_host_logs: Vec<Value>,
    pub(super) raw_messages: Vec<Value>,
    pending: Vec<Value>,
    last_full_response: Value,
    observed: Arc<Mutex<Value>>,
    debug_messages: Vec<DebugMessage>,
    pub(super) blocked: Option<String>,
    keys: [KeyState; 256],
    active: bool,
    pub(super) await_pumps: VecDeque<Value>,
    pub(super) clock: u64,
    pub(super) input_evidence: Vec<Value>,
    pub(super) instructions: u64,
}

impl ObservationSession {
    pub(super) fn new(case: &str, group: &str, request: &Value) -> AuditResult<Self> {
        let options = RuntimeOptions {
            debug_scope_mask: u64::MAX,
            ..RuntimeOptions::default()
        };
        let limits = options.limits;
        let observed = Arc::new(Mutex::new(json!({
            "case": case, "group": group, "request": request, "phase": "negotiating",
            "lines": [], "wait": null, "diagnostics": [], "setupDiagnostics": [],
            "hostLogs": [], "pending": [], "lastFullResponse": null, "projectProgress": null
        })));
        let mut runtime = Self {
            session: RuntimeSession::new(options),
            session_id: None,
            epoch: None,
            sequence: 0,
            debug_sequence: 0,
            load: None,
            wait: None,
            lines: Vec::new(),
            diagnostics: Vec::new(),
            setup_diagnostics: Vec::new(),
            host_logs: Vec::new(),
            setup_host_logs: Vec::new(),
            raw_messages: Vec::new(),
            pending: Vec::new(),
            last_full_response: Value::Null,
            observed: Arc::clone(&observed),
            debug_messages: Vec::new(),
            blocked: None,
            keys: [KeyState::default(); 256],
            active: true,
            await_pumps: VecDeque::new(),
            clock: 0,
            input_evidence: Vec::new(),
            instructions: 0,
        };
        runtime.publish_snapshot()?;
        runtime
            .session
            .set_project_progress_reporter(Some(ProjectProgressReporter::new(move |progress| {
                let mut snapshot = observed.lock().expect("observation snapshot lock");
                // The callback can run inside synchronous compilation or VM preparation,
                // before a new RuntimeStateChanged envelope can be drained.
                snapshot["phase"] = json!({"projectStage": progress.stage});
                snapshot["projectProgress"] =
                    serde_json::to_value(progress).expect("project progress serialization");
                crate::watchdog::publish_or_exit(snapshot.clone());
            })));
        runtime.send(RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "snake-observations".into(),
            configuration_profile: None,
            preferred_locales: vec!["en".into()],
            requested_limits: limits,
            features: vec![
                RuntimeFeature::ExternalServices,
                RuntimeFeature::TimedInput,
                RuntimeFeature::StateResynchronization,
            ],
            capabilities: ClientCapabilities {
                input_modalities: vec![InputModality::Keyboard],
                rich_text: false,
                html: false,
                graphics: false,
                audio: false,
                video: false,
                font_metrics: false,
                column_cells: true,
                separators: true,
                available_fonts: Vec::new(),
                services: vec![ServiceCapability {
                    kind: ServiceKind::InputState,
                    operation: GET_KEY_STATE_OPERATION.into(),
                    versions: VersionRange::exact(GET_KEY_STATE_OPERATION_VERSION),
                }],
                storage: StorageCapabilities {
                    revisions: false,
                    atomic_replace: false,
                    missing_precondition: false,
                    delete: false,
                },
            },
        }))?;
        runtime.pump_until(|runtime| runtime.session_id.is_some())?;
        Ok(runtime)
    }

    pub(super) fn begin_step(&mut self, request: &Value, initial: bool) -> AuditResult<()> {
        if initial {
            self.setup_host_logs.append(&mut self.host_logs);
        }
        self.diagnostics.clear();
        self.host_logs.clear();
        self.observed.lock().expect("observation snapshot lock")["request"] = request.clone();
        self.publish_snapshot()
    }

    fn publish_snapshot(&self) -> AuditResult<()> {
        let mut snapshot = self.observed.lock().expect("observation snapshot lock");
        snapshot["phase"] = serde_json::to_value(self.session.phase())?;
        snapshot["lines"] = serde_json::to_value(&self.lines)?;
        snapshot["wait"] = serde_json::to_value(&self.wait)?;
        snapshot["diagnostics"] = json!(self.diagnostics);
        snapshot["setupDiagnostics"] = json!(self.setup_diagnostics);
        snapshot["hostLogs"] = json!(self.host_logs);
        snapshot["setupHostLogs"] = json!(self.setup_host_logs);
        snapshot["blocked"] = json!(self.blocked);
        snapshot["pending"] = json!(self.pending);
        snapshot["lastFullResponse"] = self.last_full_response.clone();
        snapshot["inputState"] = json!({"active": self.active,
            "keys": self.keys.iter().map(|key| json!({"down":key.down,"toggle":key.toggle})).collect::<Vec<_>>(),
            "awaitPumps":self.await_pumps});
        crate::watchdog::publish(snapshot.clone())?;
        Ok(())
    }

    pub(super) fn send(&mut self, message: RuntimeMessage) -> AuditResult<()> {
        let envelope = message.envelope(
            self.session_id,
            self.epoch,
            self.sequence,
            self.sequence + 1,
            None,
        )?;
        self.session
            .submit_envelope(&encode_envelope(&envelope, crate::audit_wire_limits())?)?;
        let message = serde_json::to_value(&message)?;
        self.raw_messages.push(
            json!({"direction": "frontend_to_runtime", "envelope": envelope, "message": message}),
        );
        self.pending.push(message);
        self.publish_snapshot()?;
        self.sequence += 1;
        Ok(())
    }

    fn send_debug(&mut self, message: DebugMessage) -> AuditResult<()> {
        let envelope = message.envelope(
            self.session_id,
            self.epoch,
            self.debug_sequence,
            self.debug_sequence + 1,
            None,
        )?;
        self.session
            .submit_envelope(&encode_envelope(&envelope, crate::audit_wire_limits())?)?;
        let message = serde_json::to_value(&message)?;
        self.raw_messages.push(
            json!({"direction": "frontend_to_debug", "envelope": envelope, "message": message}),
        );
        self.pending.push(message);
        self.publish_snapshot()?;
        self.debug_sequence += 1;
        Ok(())
    }

    pub(super) fn pump_until(&mut self, done: impl Fn(&Self) -> bool) -> AuditResult<()> {
        while !done(self) {
            self.pump()?;
            if let Some(reason) = &self.blocked {
                return Err(reason.clone().into());
            }
        }
        Ok(())
    }

    pub(super) fn pump(&mut self) -> AuditResult<()> {
        let report = self.session.drive(RuntimeDriveBudget {
            maximum_vm_instructions: 10_000,
            maximum_runtime_transitions: 128,
        })?;
        self.instructions = self.instructions.saturating_add(report.vm_instructions);
        self.pending.clear();
        let mut responses = Vec::new();
        while let Some(bytes) = self.session.poll_envelope() {
            let envelope = decode_envelope(&bytes, crate::audit_wire_limits())?;
            if envelope.channel == Channel::Debug {
                let message = DebugMessage::from_envelope(&envelope)?;
                self.raw_messages.push(json!({"direction": "debug_to_frontend", "envelope": envelope, "message": message}));
                self.last_full_response = json!({"channel": "debug", "message": message});
                self.debug_messages.push(message);
                self.publish_snapshot()?;
                continue;
            }
            if let Some(epoch) = envelope.session_epoch {
                self.epoch = Some(epoch);
            }
            let message = RuntimeMessage::from_envelope(&envelope)?;
            self.raw_messages.push(json!({"direction": "runtime_to_frontend", "envelope": envelope, "message": message}));
            self.last_full_response = json!({"channel": "runtime", "message": message});
            match message {
                RuntimeMessage::ServerHello(hello) => {
                    self.session_id = Some(hello.session);
                    self.epoch = Some(SessionEpoch(hello.epoch));
                }
                RuntimeMessage::StateChanged(state) => self.epoch = Some(SessionEpoch(state.epoch)),
                RuntimeMessage::ProjectLoadReport(report) => {
                    self.setup_diagnostics.extend(
                        report
                            .diagnostics
                            .iter()
                            .map(serde_json::to_value)
                            .collect::<Result<Vec<_>, _>>()?,
                    );
                    self.load = Some(report);
                }
                RuntimeMessage::Log(log) => self.host_logs.push(serde_json::to_value(log)?),
                RuntimeMessage::Diagnostic(diagnostic) => {
                    if diagnostic.source.is_some() {
                        self.diagnostics.push(serde_json::to_value(diagnostic)?);
                    } else {
                        self.host_logs.push(serde_json::to_value(diagnostic)?);
                    }
                }
                RuntimeMessage::Fault(fault) => self.diagnostics.push(serde_json::to_value(fault)?),
                RuntimeMessage::CommandRejected(rejection) => {
                    self.blocked = Some(rejection.message.clone());
                    self.host_logs.push(serde_json::to_value(rejection)?);
                }
                RuntimeMessage::PresentationSnapshot(snapshot) => {
                    self.lines = snapshot.history.logical_lines;
                    self.wait = snapshot.input_wait;
                }
                RuntimeMessage::PresentationDelta(delta) => {
                    crate::apply_presentation_delta(&mut self.lines, &delta.operations)
                }
                RuntimeMessage::WaitChanged(
                    WaitChange::Opened(wait) | WaitChange::Updated(wait),
                ) => self.wait = Some(wait),
                RuntimeMessage::WaitChanged(WaitChange::Closed(id)) => {
                    if self.wait.as_ref().is_some_and(|wait| wait.wait_id == id) {
                        self.wait = None;
                    }
                }
                RuntimeMessage::ServiceRequest(request) => {
                    if request.kind == ServiceKind::InputState
                        && request.operation == GET_KEY_STATE_OPERATION
                        && request.operation_version == GET_KEY_STATE_OPERATION_VERSION
                    {
                        let query: GetKeyStateRequest =
                            decode_canonical(request.payload.as_slice())?;
                        let state = self.keys[usize::from(query.key_code)];
                        let reply = GetKeyStateResponse {
                            frontend_active: self.active,
                            pressed: state.down,
                            toggle_state: state.toggle,
                        };
                        self.input_evidence
                            .push(json!({"query": query, "response": reply}));
                        responses.push(RuntimeMessage::ServiceResponse(ServiceResponse {
                            request_id: request.request_id,
                            result: ServiceResult::Ready {
                                payload: ProtocolBytes::new(encode_canonical(&reply)?),
                            },
                        }));
                    } else {
                        self.blocked = Some(format!(
                            "unsupported required service {:?}/{}@{:?}",
                            request.kind, request.operation, request.operation_version
                        ));
                        self.host_logs
                            .push(json!({"stage": "capability", "request": request}));
                    }
                }
                _ => {}
            }
        }
        for response in responses {
            self.send(response)?;
        }
        self.publish_snapshot()?;
        Ok(())
    }

    pub(super) fn completed(&self) -> bool {
        self.wait.is_some()
            && self
                .lines
                .iter()
                .any(|line| line_text(line).contains(COMPLETE))
    }

    pub(super) fn install_input_trace(&mut self, trace: &Value) -> AuditResult<()> {
        if trace.is_null() {
            return Ok(());
        }
        let active = trace["active"]
            .as_bool()
            .ok_or("inputTrace.active must be boolean")?;
        let before = trace.get("beforeRun").cloned().unwrap_or_else(|| json!([]));
        let pumps = trace
            .get("awaitPumps")
            .cloned()
            .unwrap_or_else(|| json!([]));
        parse_key_events(&before)?;
        for batch in pumps.as_array().ok_or("awaitPumps must be an array")? {
            parse_key_events(batch)?;
        }
        self.active = active;
        self.await_pumps = pumps.as_array().unwrap().iter().cloned().collect();
        self.send(RuntimeMessage::ClientStateChanged(ClientStateChanged {
            focused: active,
            visible: true,
            audio_available: false,
            reduce_motion: false,
            high_contrast: false,
            screen_reader: false,
        }))?;
        self.apply_key_events(&before)
    }

    pub(super) fn apply_key_events(&mut self, events: &Value) -> AuditResult<()> {
        // Validate the entire batch before mutating the host's state. No synthetic edge latch:
        // GETKEYTRIGGERED policy remains the product runtime's responsibility.
        let parsed = parse_key_events(events)?;
        for (code, state) in parsed {
            self.keys[usize::from(code)] = state;
            self.clock = self.clock.saturating_add(1);
            let event = DeviceStateChanged {
                device: InputDeviceKind::Keyboard,
                code: u32::from(code),
                pressed: state.down,
                x: 0,
                y: 0,
                monotonic_time_ns: self.clock,
            };
            self.input_evidence
                .push(json!({"device": event, "toggle": state.toggle}));
            self.send(RuntimeMessage::DeviceStateChanged(event))?;
        }
        Ok(())
    }

    pub(super) fn pause(&mut self) -> AuditResult<(GrantToken, StopToken)> {
        self.debug_messages.clear();
        self.send_debug(DebugMessage::Hello(DebugHello {
            versions: VersionRange::exact(DEBUG_PROTOCOL_VERSION),
            requested_scopes: vec![DebugScope::ExecutionControl, DebugScope::VariablesRead],
        }))?;
        self.pump_until(|runtime| !runtime.debug_messages.is_empty())?;
        let grant = self
            .debug_messages
            .iter()
            .find_map(|message| match message {
                DebugMessage::Grant(grant) => Some(grant.token),
                _ => None,
            })
            .ok_or_else(|| format!("debug grant missing: {:?}", self.debug_messages))?;
        self.debug_command(grant, DebugCommand::Pause)?;
        let stop = self
            .debug_messages
            .iter()
            .find_map(|message| match message {
                DebugMessage::Stopped(stop) => Some(stop.stop),
                _ => None,
            })
            .ok_or_else(|| format!("debug stop missing: {:?}", self.debug_messages))?;
        Ok((grant, stop))
    }

    pub(super) fn debug_command(
        &mut self,
        grant: GrantToken,
        command: DebugCommand,
    ) -> AuditResult<()> {
        self.debug_messages.clear();
        self.send_debug(DebugMessage::Request(AuthorizedDebugRequest {
            grant,
            command,
        }))?;
        self.pump_until(|runtime| !runtime.debug_messages.is_empty())?;
        if let Some(error) = self
            .debug_messages
            .iter()
            .find_map(|message| match message {
                DebugMessage::Error(error) => Some(error),
                _ => None,
            })
        {
            return Err(format!("debug request failed: {error:?}").into());
        }
        Ok(())
    }

    pub(super) fn read_watch(
        &mut self,
        grant: GrantToken,
        stop: StopToken,
        source: &str,
    ) -> AuditResult<Value> {
        let mut parts = source.split(':');
        let name = parts.next().ok_or("watch has no name")?;
        let indices: Vec<u64> = parts.map(str::parse).collect::<Result<_, _>>()?;
        let mut cursor = None;
        let descriptor = loop {
            self.debug_command(
                grant,
                DebugCommand::ListVariables {
                    stop,
                    cursor,
                    limit: 256,
                },
            )?;
            let page = self
                .debug_messages
                .iter()
                .find_map(|message| match message {
                    DebugMessage::Response(DebugResponse::VariablePage(page)) => Some(page),
                    _ => None,
                })
                .ok_or("debugger did not return a variable page")?;
            if let Some(descriptor) = page
                .variables
                .iter()
                .find(|variable| variable.name.eq_ignore_ascii_case(name))
            {
                break descriptor.clone();
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Err(format!("watch {name} is not present in the VM").into());
            }
        };
        if descriptor.storage != VariableStorage::Global
            && descriptor.storage != VariableStorage::FunctionStatic
        {
            return Err(format!(
                "watch {name} requires a frame/character selector absent from the fixture"
            )
            .into());
        }
        self.debug_command(
            grant,
            DebugCommand::ReadVariable {
                stop,
                value: VariableReference {
                    symbol_key: descriptor.symbol_key,
                    storage: descriptor.storage,
                    fiber_id: None,
                    frame_id: None,
                    generation: stop.program_generation,
                    character: None,
                    indices,
                },
            },
        )?;
        let value = self
            .debug_messages
            .iter()
            .find_map(|message| match message {
                DebugMessage::Response(DebugResponse::VariableValue(value)) => Some(&value.value),
                _ => None,
            })
            .ok_or("debugger did not return a variable value")?;
        match value {
            DebugValue::Integer(value) => Ok(json!(value)),
            DebugValue::String(value) => Ok(json!(value)),
            DebugValue::Boolean(value) => Ok(json!(value)),
            other => Err(format!("unsupported debug observation {other:?}").into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_step_starts_without_setup_or_previous_script_diagnostics() {
        let request = json!({"op": "run", "entry": "COMPAT_KEYS"});
        let mut session = ObservationSession::new("diagnostic-scope", "GETKEY", &request).unwrap();
        session.host_logs.clear();
        session
            .setup_diagnostics
            .push(json!({"code": "compile.warning"}));
        session
            .host_logs
            .push(json!({"message": "setup notification"}));
        session.begin_step(&request, true).unwrap();
        assert!(session.diagnostics.is_empty());
        assert!(session.host_logs.is_empty());
        assert_eq!(
            session.setup_host_logs,
            [json!({"message": "setup notification"})]
        );

        session.diagnostics.push(json!({"code": "script.warning"}));
        session
            .host_logs
            .push(json!({"message": "previous step notification"}));
        session
            .begin_step(&json!({"op": "run", "inputs": ["0"]}), false)
            .unwrap();
        assert!(session.diagnostics.is_empty());
        assert!(session.host_logs.is_empty());
        assert_eq!(
            session.setup_diagnostics,
            [json!({"code": "compile.warning"})]
        );
    }
}
