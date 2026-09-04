use std::collections::BTreeMap;
use std::time::Instant;

use era_debug_protocol::{
    AuthorizedDebugRequest, DEBUG_PROTOCOL_VERSION, DebugCommand, DebugHello, DebugMessage,
    DebugResponse, DebugScope, DebugValue, GrantToken, StopToken, VariableReference,
    VariableStorage,
};
use era_protocol::{
    Channel, Envelope, ProtocolBytes, SessionEpoch, SessionId, VersionRange, WireLimits,
    decode_envelope, encode_envelope,
};
use era_runtime::{
    RuntimeDriveBudget, RuntimeDriveReport, RuntimeDriveState, RuntimeOptions, RuntimeSession,
};
use era_runtime_protocol::{
    DisplayLine, DisplayRun, FrontendInput, InputWait, PresentationOperation, ResourceReplay,
    RuntimeLimits, RuntimeMessage, RuntimePhase, SceneOperationV1, SceneStateV1, ServiceResponse,
    StorageResponse, WaitChange, RUNTIME_PROTOCOL_VERSION,
};
use serde::Serialize;
use serde_json::{Value, json};

use super::trace::{
    CheckpointExpectation, ServiceExpectation, StorageExpectation, TraceAction,
};
use super::{AuditResult, allocator};

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PresentationMetrics {
    pub(super) logical_lines: usize,
    pub(super) runs: usize,
    pub(super) scene_layers: usize,
    pub(super) sprites: usize,
    pub(super) canvases: usize,
    pub(super) canvas_commands: usize,
}

pub(super) struct PumpObservation {
    pub(super) report: RuntimeDriveReport,
    pub(super) elapsed_ns: u128,
    pub(super) allocations: Option<allocator::AllocationStats>,
    pub(super) envelope_count: usize,
    pub(super) envelope_bytes: usize,
    pub(super) snapshot_count: usize,
    pub(super) delta_count: usize,
    pub(super) messages: Vec<RuntimeMessage>,
}

#[derive(Default)]
struct PresentationState {
    wait: Option<InputWait>,
    lines: Vec<DisplayLine>,
    resources: ResourceReplay,
    scene: SceneStateV1,
    metrics: PresentationMetrics,
    text: Option<String>,
}

pub(super) struct PerfSession {
    pub(super) runtime: RuntimeSession,
    pub(super) requested_limits: RuntimeLimits,
    pub(super) wire_limits: WireLimits,
    session_id: Option<SessionId>,
    epoch: Option<SessionEpoch>,
    sequence: u64,
    debug_sequence: u64,
    phase: RuntimePhase,
    presentation: PresentationState,
}

impl PerfSession {
    pub(super) fn new() -> Self {
        let options = RuntimeOptions {
            debug_scope_mask: u64::MAX,
            ..RuntimeOptions::default()
        };
        let requested_limits = options.limits;
        let wire_limits = options.wire_limits;
        Self {
            runtime: RuntimeSession::new(options),
            requested_limits,
            wire_limits,
            session_id: None,
            epoch: None,
            sequence: 0,
            debug_sequence: 0,
            phase: RuntimePhase::Negotiating,
            presentation: PresentationState::default(),
        }
    }

    pub(super) const fn phase(&self) -> RuntimePhase {
        self.phase
    }

    pub(super) const fn metrics(&self) -> PresentationMetrics {
        self.presentation.metrics
    }

    pub(super) fn has_session(&self) -> bool {
        self.session_id.is_some()
    }

    pub(super) fn send(&mut self, message: RuntimeMessage) -> AuditResult<()> {
        let mut envelope = Envelope::new(
            Channel::Runtime,
            RUNTIME_PROTOCOL_VERSION,
            self.sequence,
            self.sequence.saturating_add(1),
            message.tag(),
            ProtocolBytes::new(message.encode_payload()?),
        );
        envelope.session = self.session_id;
        envelope.session_epoch = self.epoch;
        self.runtime
            .submit_envelope(&encode_envelope(&envelope, self.wire_limits)?)?;
        self.sequence = self.sequence.saturating_add(1);
        Ok(())
    }

    pub(super) fn pump(
        &mut self,
        allocation_mode: allocator::MeasurementMode,
    ) -> AuditResult<PumpObservation> {
        let (measured, allocations) = allocator::measure(allocation_mode, || -> AuditResult<_> {
            let started = Instant::now();
            let report = self.runtime.drive(RuntimeDriveBudget {
                maximum_vm_instructions: 10_000,
                maximum_runtime_transitions: 128,
            })?;
            let elapsed_ns = started.elapsed().as_nanos();
            let mut raw = Vec::new();
            while let Some(bytes) = self.runtime.poll_envelope() {
                raw.push(bytes);
            }
            Ok((report, elapsed_ns, raw))
        });
        let (report, elapsed_ns, raw) = measured?;
        let envelope_count = raw.len();
        let envelope_bytes = raw.iter().map(Vec::len).sum();
        let mut messages = Vec::with_capacity(raw.len());
        let mut presentation_changed = false;
        for bytes in raw {
            let envelope = decode_envelope(&bytes, self.wire_limits)?;
            if envelope.channel != Channel::Runtime {
                return Err("unexpected debug envelope during timed runtime pump".into());
            }
            self.observe_envelope(&envelope);
            let message = RuntimeMessage::from_envelope(&envelope)?;
            presentation_changed |= self.observe_message(&message);
            messages.push(message);
        }
        if presentation_changed {
            self.refresh_presentation_cache();
        }
        let snapshot_count = messages
            .iter()
            .filter(|message| matches!(message, RuntimeMessage::PresentationSnapshot(_)))
            .count();
        let delta_count = messages
            .iter()
            .filter(|message| matches!(message, RuntimeMessage::PresentationDelta(_)))
            .count();
        Ok(PumpObservation {
            report,
            elapsed_ns,
            allocations,
            envelope_count,
            envelope_bytes,
            snapshot_count,
            delta_count,
            messages,
        })
    }

    pub(super) fn checkpoint_matches(
        &mut self,
        messages: &[RuntimeMessage],
        expected: &CheckpointExpectation,
    ) -> bool {
        if expected.phase.is_some_and(|phase| phase != self.phase) {
            return false;
        }
        if expected.wait_kind.is_some_and(|kind| {
            self.presentation.wait.as_ref().map(|wait| wait.kind) != Some(kind)
        }) {
            return false;
        }
        if !expected.text_contains.is_empty() {
            let text = self.presentation_text();
            if expected
                .text_contains
                .iter()
                .any(|needle| !text.contains(needle))
            {
                return false;
            }
        }
        expected
            .outbound_tags
            .iter()
            .all(|tag| messages.iter().any(|message| message.tag() == *tag))
            && expected.services.iter().all(|service| {
                messages.iter().any(|message| {
                    matches!(message, RuntimeMessage::ServiceRequest(request)
                        if request.kind == service.kind && request.operation == service.operation)
                })
            })
            && expected.storage.iter().all(|storage| {
                messages.iter().any(|message| {
                    matches!(message, RuntimeMessage::StorageRequest(request)
                        if request.namespace == storage.namespace
                            && request.relative_path == storage.relative_path)
                })
            })
    }

    pub(super) fn normalized_state(
        &self,
        messages: &[RuntimeMessage],
        variables: &BTreeMap<String, Value>,
    ) -> AuditResult<Value> {
        let services = messages
            .iter()
            .filter_map(|message| match message {
                RuntimeMessage::ServiceRequest(request) => Some(json!({
                    "kind": request.kind,
                    "operation": request.operation,
                    "operationVersion": request.operation_version,
                    "payload": request.payload,
                })),
                _ => None,
            })
            .collect::<Vec<_>>();
        let storage = messages
            .iter()
            .filter_map(|message| match message {
                RuntimeMessage::StorageRequest(request) => Some(json!({
                    "namespace": request.namespace,
                    "relativePath": request.relative_path,
                    "operation": request.operation,
                })),
                _ => None,
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "phase": self.phase,
            "wait": &self.presentation.wait,
            "lines": &self.presentation.lines,
            "resources": &self.presentation.resources,
            "scene": &self.presentation.scene,
            "variables": variables,
            "services": services,
            "storage": storage,
            "otherOutboundTags": messages.iter().filter(|message| {
                !matches!(message, RuntimeMessage::ServiceRequest(_) | RuntimeMessage::StorageRequest(_))
            }).map(RuntimeMessage::tag).collect::<Vec<_>>(),
        }))
    }

    pub(super) fn apply_action(
        &mut self,
        action: &TraceAction,
        messages: &[RuntimeMessage],
        logical_time: u64,
    ) -> AuditResult<()> {
        match action {
            TraceAction::None => Ok(()),
            TraceAction::Input {
                intent,
                message_skip,
            } => {
                let wait = self
                    .presentation
                    .wait
                    .as_ref()
                    .ok_or("input action has no active wait")?;
                self.send(RuntimeMessage::Input(FrontendInput {
                    wait_id: wait.wait_id,
                    token: wait.submission_token,
                    monotonic_time_ns: logical_time.saturating_add(1) * 1_000_000,
                    intent: intent.clone(),
                    message_skip: *message_skip,
                }))
            }
            TraceAction::ServiceResponse { service, result } => {
                let request = unique_service_request(messages, service)?;
                self.send(RuntimeMessage::ServiceResponse(ServiceResponse {
                    request_id: request.request_id,
                    result: result.clone(),
                }))
            }
            TraceAction::StorageResponse { storage, result } => {
                let request = unique_storage_request(messages, storage)?;
                self.send(RuntimeMessage::StorageResponse(StorageResponse {
                    request_id: request.request_id,
                    result: result.clone(),
                }))
            }
            TraceAction::Submit { message } => self.send((**message).clone()),
        }
    }

    pub(super) fn validate_variables(
        &mut self,
        expected: &BTreeMap<String, Value>,
    ) -> AuditResult<BTreeMap<String, Value>> {
        if expected.is_empty() {
            return Ok(BTreeMap::new());
        }
        let (grant, stop) = self.debug_pause()?;
        let mut actual = BTreeMap::new();
        for (watch, expected_value) in expected {
            let value = self.read_watch(grant, stop, watch)?;
            if &value != expected_value {
                return Err(format!(
                    "variable {watch} mismatch: expected={expected_value} actual={value}"
                )
                .into());
            }
            actual.insert(watch.clone(), value);
        }
        self.debug_command(grant, DebugCommand::Continue { stop })?;
        Ok(actual)
    }

    fn observe_envelope(&mut self, envelope: &Envelope) {
        if let Some(session) = envelope.session {
            self.session_id = Some(session);
        }
        if let Some(epoch) = envelope.session_epoch {
            self.epoch = Some(epoch);
        }
    }

    fn observe_message(&mut self, message: &RuntimeMessage) -> bool {
        match message {
            RuntimeMessage::ServerHello(hello) => {
                self.session_id = Some(hello.session);
                self.epoch = Some(SessionEpoch(hello.epoch));
                false
            }
            RuntimeMessage::StateChanged(state) => {
                self.phase = state.phase;
                false
            }
            RuntimeMessage::PresentationSnapshot(snapshot) => {
                self.presentation.lines.clone_from(&snapshot.history.logical_lines);
                self.presentation.wait.clone_from(&snapshot.input_wait);
                self.presentation.resources.clone_from(&snapshot.resources);
                self.presentation.scene.clone_from(&snapshot.scene);
                true
            }
            RuntimeMessage::PresentationDelta(delta) => {
                super::super::apply_presentation_delta(
                    &mut self.presentation.lines,
                    &delta.operations,
                );
                for operation in &delta.operations {
                    match operation {
                        PresentationOperation::SetResources { resources } => {
                            self.presentation.resources.clone_from(resources);
                        }
                        PresentationOperation::ApplySceneDelta { delta } => {
                            apply_scene_delta(&mut self.presentation.scene, delta);
                        }
                        PresentationOperation::SetInputWait { input_wait } => {
                            self.presentation.wait.clone_from(input_wait);
                        }
                        _ => {}
                    }
                }
                true
            }
            RuntimeMessage::WaitChanged(WaitChange::Opened(wait) | WaitChange::Updated(wait)) => {
                self.presentation.wait = Some(wait.clone());
                true
            }
            RuntimeMessage::WaitChanged(WaitChange::Closed(wait_id)) => {
                if self
                    .presentation
                    .wait
                    .as_ref()
                    .is_some_and(|wait| wait.wait_id == *wait_id)
                {
                    self.presentation.wait = None;
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn refresh_presentation_cache(&mut self) {
        self.presentation.metrics = PresentationMetrics {
            logical_lines: self.presentation.lines.len(),
            runs: self
                .presentation
                .lines
                .iter()
                .map(|line| line.runs.iter().map(count_runs).sum::<usize>())
                .sum(),
            scene_layers: self.presentation.scene.layers.len(),
            sprites: self.presentation.resources.sprites.len(),
            canvases: self.presentation.resources.canvases.len(),
            canvas_commands: self
                .presentation
                .resources
                .canvases
                .iter()
                .map(|canvas| canvas.commands.len())
                .sum(),
        };
        self.presentation.text = None;
    }

    fn presentation_text(&mut self) -> &str {
        if self.presentation.text.is_none() {
            let text = self.presentation
                .lines
                .iter()
                .flat_map(|line| line.runs.iter())
                .map(super::super::display_text)
                .collect::<Vec<_>>()
                .join("\n");
            self.presentation.text = Some(text);
        }
        self.presentation.text.as_deref().unwrap_or_default()
    }

    fn debug_pause(&mut self) -> AuditResult<(GrantToken, StopToken)> {
        let messages = self.debug_exchange(DebugMessage::Hello(DebugHello {
            versions: VersionRange::exact(DEBUG_PROTOCOL_VERSION),
            requested_scopes: vec![DebugScope::ExecutionControl, DebugScope::VariablesRead],
        }))?;
        let grant = messages
            .iter()
            .find_map(|message| match message {
                DebugMessage::Grant(grant) => Some(grant.token),
                _ => None,
            })
            .ok_or("debug grant missing")?;
        let messages = self.debug_command(grant, DebugCommand::Pause)?;
        let stop = messages
            .iter()
            .find_map(|message| match message {
                DebugMessage::Stopped(stopped) => Some(stopped.stop),
                _ => None,
            })
            .ok_or("debug stop missing")?;
        Ok((grant, stop))
    }

    fn debug_command(
        &mut self,
        grant: GrantToken,
        command: DebugCommand,
    ) -> AuditResult<Vec<DebugMessage>> {
        let messages = self.debug_exchange(DebugMessage::Request(AuthorizedDebugRequest {
            grant,
            command,
        }))?;
        if let Some(error) = messages.iter().find_map(|message| match message {
            DebugMessage::Error(error) => Some(error),
            _ => None,
        }) {
            return Err(format!("debug request failed: {error:?}").into());
        }
        Ok(messages)
    }

    fn debug_exchange(&mut self, message: DebugMessage) -> AuditResult<Vec<DebugMessage>> {
        let envelope = message.envelope(
            self.session_id,
            self.epoch,
            self.debug_sequence,
            self.debug_sequence.saturating_add(1),
            None,
        )?;
        self.runtime.submit_envelope(&encode_envelope(
            &envelope,
            self.wire_limits,
        )?)?;
        self.debug_sequence = self.debug_sequence.saturating_add(1);
        for _ in 0..1_000 {
            self.runtime.drive(RuntimeDriveBudget {
                maximum_vm_instructions: 10_000,
                maximum_runtime_transitions: 128,
            })?;
            let mut debug = Vec::new();
            while let Some(bytes) = self.runtime.poll_envelope() {
                let envelope = decode_envelope(&bytes, self.wire_limits)?;
                self.observe_envelope(&envelope);
                if envelope.channel == Channel::Debug {
                    debug.push(DebugMessage::from_envelope(&envelope)?);
                } else {
                    let runtime = RuntimeMessage::from_envelope(&envelope)?;
                    if self.observe_message(&runtime) {
                        self.refresh_presentation_cache();
                    }
                }
            }
            if !debug.is_empty() {
                return Ok(debug);
            }
        }
        Err("debug exchange exceeded pump limit".into())
    }

    fn read_watch(
        &mut self,
        grant: GrantToken,
        stop: StopToken,
        source: &str,
    ) -> AuditResult<Value> {
        let mut parts = source.split(':');
        let name = parts
            .next()
            .filter(|name| !name.is_empty())
            .ok_or("watch has no name")?;
        let indices = parts.map(str::parse).collect::<Result<Vec<u64>, _>>()?;
        let mut cursor = None;
        let descriptor = loop {
            let messages = self.debug_command(
                grant,
                DebugCommand::ListVariables {
                    stop,
                    cursor,
                    limit: 256,
                },
            )?;
            let page = messages
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
        if !matches!(
            descriptor.storage,
            VariableStorage::Global | VariableStorage::FunctionStatic
        ) {
            return Err(
                format!("watch {name} requires an unsupported frame or character selector").into(),
            );
        }
        let messages = self.debug_command(
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
        let value = messages
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
            other => Err(format!("unsupported debug value {other:?}").into()),
        }
    }
}

pub(super) const fn drive_state_name(state: RuntimeDriveState) -> &'static str {
    match state {
        RuntimeDriveState::Idle => "idle",
        RuntimeDriveState::MoreWork => "more_work",
        RuntimeDriveState::OutputReady => "output_ready",
        RuntimeDriveState::Stopped => "stopped",
        RuntimeDriveState::Faulted => "faulted",
    }
}

fn unique_service_request<'a>(
    messages: &'a [RuntimeMessage],
    expected: &ServiceExpectation,
) -> AuditResult<&'a era_runtime_protocol::ServiceRequest> {
    let mut matches = messages.iter().filter_map(|message| match message {
        RuntimeMessage::ServiceRequest(request)
            if request.kind == expected.kind && request.operation == expected.operation =>
        {
            Some(request)
        }
        _ => None,
    });
    let request = matches.next().ok_or("matching service request is absent")?;
    if matches.next().is_some() {
        return Err("service response action matched multiple requests".into());
    }
    Ok(request)
}

fn unique_storage_request<'a>(
    messages: &'a [RuntimeMessage],
    expected: &StorageExpectation,
) -> AuditResult<&'a era_runtime_protocol::StorageRequest> {
    let mut matches = messages.iter().filter_map(|message| match message {
        RuntimeMessage::StorageRequest(request)
            if request.namespace == expected.namespace
                && request.relative_path == expected.relative_path =>
        {
            Some(request)
        }
        _ => None,
    });
    let request = matches.next().ok_or("matching storage request is absent")?;
    if matches.next().is_some() {
        return Err("storage response action matched multiple requests".into());
    }
    Ok(request)
}

fn apply_scene_delta(scene: &mut SceneStateV1, delta: &era_runtime_protocol::SceneDeltaV1) {
    for operation in &delta.operations {
        match operation {
            SceneOperationV1::UpsertLayer { layer } => {
                if let Some(existing) = scene
                    .layers
                    .iter_mut()
                    .find(|item| item.layer_id == layer.layer_id)
                {
                    existing.clone_from(layer.as_ref());
                } else {
                    scene.layers.push((**layer).clone());
                }
            }
            SceneOperationV1::RemoveLayer { layer_id } => {
                scene.layers.retain(|layer| layer.layer_id != *layer_id);
            }
            SceneOperationV1::ClearDepth { depth } => {
                scene.layers.retain(|layer| layer.depth != *depth);
            }
            SceneOperationV1::ClearAnchoredLine { line_id } => {
                scene.layers.retain(|layer| {
                    !matches!(
                        layer.anchor,
                        era_runtime_protocol::SceneAnchorV1::DisplayLine { line_id: anchor }
                            if anchor == *line_id
                    )
                });
            }
            SceneOperationV1::ReplaceScene { scene: replacement } => {
                scene.clone_from(replacement);
            }
        }
    }
    scene.revision = delta.new_revision;
}

fn count_runs(run: &DisplayRun) -> usize {
    match run {
        DisplayRun::Button { runs, .. } => 1 + runs.iter().map(count_runs).sum::<usize>(),
        DisplayRun::ColumnCell { content, .. } => {
            1 + content.iter().map(count_runs).sum::<usize>()
        }
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_state_names_are_explicit_and_stable() {
        assert_eq!(drive_state_name(RuntimeDriveState::Idle), "idle");
        assert_eq!(drive_state_name(RuntimeDriveState::MoreWork), "more_work");
        assert_eq!(drive_state_name(RuntimeDriveState::OutputReady), "output_ready");
        assert_eq!(drive_state_name(RuntimeDriveState::Stopped), "stopped");
        assert_eq!(drive_state_name(RuntimeDriveState::Faulted), "faulted");
    }
}
