//! HTML query services retain one VM wait across all core-owned measurement rounds.

mod flows;
mod plan;
mod wire;

pub(crate) use flows::HtmlLineFlows;

use era_runtime_protocol::{HtmlMeasureRequestV2, HtmlMeasureResponseV2, HtmlQueryStyleV2};
use erabasic_html::{HtmlQueryError, HtmlQueryErrorKind, HtmlStringLengthSettings};
use erabasic_vm::{FiberId, FrameId, HostRequestId, VmExecutionOrigin};

#[allow(clippy::wildcard_imports)]
use super::*;
use plan::{
    PlanPoll, QueryBudget, QueryPlan, failure, reference_length_unit, reference_split_pixels,
};
use wire::ProbeTransfer;

const MAXIMUM_RESPONSE_BYTES: usize = 64 * 1024;
const MAXIMUM_REQUEST_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum QueryOperation {
    Length,
    Substring,
    Lines,
}

impl QueryOperation {
    fn wire(self) -> (&'static str, ProtocolVersion) {
        match self {
            Self::Length => (HTML_STRING_LEN_OPERATION, HTML_STRING_LEN_OPERATION_VERSION),
            Self::Substring => (HTML_SUBSTRING_OPERATION, HTML_SUBSTRING_OPERATION_VERSION),
            Self::Lines => (
                HTML_STRING_LINES_OPERATION,
                HTML_STRING_LINES_OPERATION_VERSION,
            ),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct HtmlQueryContinuation {
    request: HostRequestId,
    fiber: FiberId,
    frame: FrameId,
    depth: usize,
    origin: VmExecutionOrigin,
    epoch: u64,
    context: ProjectionQueryContext,
    style: HtmlQueryStyleV2,
    operation: QueryOperation,
    plan: QueryPlan,
    budget: QueryBudget,
    transfer: Option<ProbeTransfer>,
    next_probe: u32,
    line_ticket: Option<String>,
}

enum Advance {
    Request(HtmlMeasureRequestV2),
    Complete(PlanPoll),
}

impl HtmlQueryContinuation {
    fn advance(&mut self) -> Result<Advance, HtmlQueryError> {
        if self.transfer.is_none() {
            match self
                .plan
                .poll(query_settings(&self.style)?, &mut self.budget)?
            {
                PlanPoll::Measure(probe) => {
                    self.transfer = Some(ProbeTransfer::new(probe));
                }
                complete => return Ok(Advance::Complete(complete)),
            }
        }
        let next = self.next_probe.checked_add(1).ok_or_else(|| {
            failure(
                HtmlQueryErrorKind::ResourceLimit,
                "HTML request identifier limit exceeded",
            )
        })?;
        let probe = self
            .transfer
            .as_mut()
            .expect("initialized transfer")
            .request(next, &mut self.budget)?;
        self.next_probe = next;
        Ok(Advance::Request(HtmlMeasureRequestV2 {
            context: self.context,
            style: self.style.clone(),
            probes: vec![probe],
        }))
    }

    fn receive(&mut self, response: HtmlMeasureResponseV2) -> Result<(), HtmlQueryError> {
        let transfer = self.transfer.as_mut().ok_or_else(|| {
            failure(
                HtmlQueryErrorKind::InvalidMeasurement,
                "HTML response has no pending probe",
            )
        })?;
        if let Some(measurement) = transfer.resume(response)? {
            self.plan.resume(measurement)?;
            self.transfer = None;
        }
        Ok(())
    }

    fn owns_vm_frame(&self, vm: &RuntimeVm) -> bool {
        vm.host_frame_identity(self.fiber, self.depth)
            == Some((self.frame, self.origin.generation, self.origin.function))
    }
}

impl RuntimeSession {
    pub(in crate::session) fn dispatch_html_query(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
    ) -> Result<(), RuntimeError> {
        let operation = if name.starts_with("HTML__LINES_") || name == "HTML_STRINGLINES" {
            QueryOperation::Lines
        } else if name == "HTML_SUBSTRING" {
            QueryOperation::Substring
        } else {
            QueryOperation::Length
        };
        let (wire_operation, version) = operation.wire();
        if !self.require_host_service(
            request,
            ServiceKind::PresentationQuery,
            wire_operation,
            version,
        )? {
            return Ok(());
        }
        // Capture after argument side effects and publish all pending output before observing.
        let context = self.presentation_observation_context()?;
        let style = self.presentation.html_query_style();
        let result = self.prepare_html_query(vm, request, name, operation, context, style);
        let prepared = match result {
            Ok(prepared) => prepared,
            Err(error) => {
                let outcome = self.complete_html_failure(vm, request.id, &error, &request.origin);
                if outcome.is_ok()
                    && error.origin() == erabasic_html::HtmlQueryErrorOrigin::ScriptInput
                    && name == "HTML__LINES_STEP"
                {
                    // Source parsing occurs only after prepare_html_query validates this ticket.
                    if let Some(VmValue::String(ticket)) = request.arguments.first() {
                        self.operations.html_lines.discard_failed_step(ticket);
                    }
                }
                return outcome;
            }
        };
        match prepared {
            PreparedQuery::Ready(value) => commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(value),
                    writes: Vec::new(),
                }),
            ),
            PreparedQuery::Plan(mut continuation) => {
                let advance = match continuation.advance() {
                    Ok(advance) => advance,
                    Err(error) => {
                        let outcome =
                            self.complete_html_failure(vm, request.id, &error, &request.origin);
                        if outcome.is_ok()
                            && error.origin() == erabasic_html::HtmlQueryErrorOrigin::ScriptInput
                            && let Some(ticket) = &continuation.line_ticket
                        {
                            self.operations.html_lines.discard_failed_step(ticket);
                        }
                        return outcome;
                    }
                };
                match advance {
                    Advance::Complete(value) => {
                        let ready = match self.finish_html_query(vm, &continuation, value) {
                            Ok(ready) => ready,
                            Err(error) => {
                                return self.complete_html_failure(
                                    vm,
                                    request.id,
                                    &error,
                                    &request.origin,
                                );
                            }
                        };
                        commit_completion(vm, request.id, VmHostCompletion::Ready(ready))
                    }
                    Advance::Request(payload) => {
                        let encoded = match encode_html_request(&payload, &mut continuation.budget)
                        {
                            Ok(encoded) => encoded,
                            Err(error) => {
                                return self.complete_html_failure(
                                    vm,
                                    request.id,
                                    &error,
                                    &request.origin,
                                );
                            }
                        };
                        // Only the first service round changes the VM host wait.
                        commit_completion(
                            vm,
                            request.id,
                            VmHostCompletion::Pending {
                                stability: HostWaitStability::Transient,
                                rebind_payload: Vec::new(),
                            },
                        )?;
                        self.emit_html_measurement(continuation, encoded)
                    }
                }
            }
        }
    }

    fn prepare_html_query(
        &mut self,
        vm: &RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        operation: QueryOperation,
        context: ProjectionQueryContext,
        style: HtmlQueryStyleV2,
    ) -> Result<PreparedQuery, HtmlQueryError> {
        let settings = query_settings(&style)?;
        if name == "HTML__LENGTH_UNIT" {
            return reference_length_unit(
                integer(request, 0)?,
                integer(request, 1)?,
                settings.font_size_pixels,
            )
            .map(|value| PreparedQuery::Ready(VmValue::Integer(value)));
        }
        if name == "HTML__LINES_BEGIN" {
            let ticket = self.operations.html_lines.begin(
                self.epoch.0,
                vm,
                request,
                string(request, 0)?.into(),
            )?;
            return Ok(PreparedQuery::Ready(VmValue::String(ticket)));
        }
        let mut budget = QueryBudget::default();
        let mut line_ticket = None;
        let plan = if name.starts_with("HTML__LINES_") {
            let ticket = string(request, 0)?;
            let flow = self
                .operations
                .html_lines
                .get(ticket, self.epoch.0, vm, request)?;
            match name {
                "HTML__LINES_MORE" => {
                    return Ok(PreparedQuery::Ready(VmValue::Integer(i64::from(
                        !flow.tail.is_empty(),
                    ))));
                }
                "HTML__LINES_END" => {
                    return self
                        .operations
                        .html_lines
                        .end(ticket)
                        .map(|value| PreparedQuery::Ready(VmValue::Integer(value)));
                }
                "HTML__LINES_STEP" => {
                    budget = flow.budget;
                    let source = flow.tail.clone();
                    let plan = QueryPlan::substring(
                        &source,
                        reference_split_pixels(integer(request, 1)?, settings.font_size_pixels),
                        &mut budget,
                    )?;
                    self.operations.html_lines.start_step(ticket)?;
                    line_ticket = Some(ticket.into());
                    plan
                }
                _ => return Err(invalid_arguments()),
            }
        } else {
            initial_html_plan(request, name, settings, &mut budget)?
        };
        let depth = vm
            .fiber_frame_count(request.fiber)
            .and_then(|count| count.checked_sub(1))
            .ok_or_else(invalid_arguments)?;
        let (frame, generation, function) = vm
            .host_frame_identity(request.fiber, depth)
            .ok_or_else(invalid_arguments)?;
        if generation != request.origin.generation || function != request.origin.function {
            return Err(invalid_arguments());
        }
        Ok(PreparedQuery::Plan(Box::new(HtmlQueryContinuation {
            request: request.id,
            fiber: request.fiber,
            frame,
            depth,
            origin: request.origin.clone(),
            epoch: self.epoch.0,
            context,
            style,
            operation,
            plan,
            budget,
            transfer: None,
            next_probe: 0,
            line_ticket,
        })))
    }

    pub(in crate::session) fn complete_html_query(
        &mut self,
        mut continuation: Box<HtmlQueryContinuation>,
        result: ServiceResult,
    ) -> Result<(), RuntimeError> {
        if !matches!(
            self.phase,
            RuntimePhase::Running
                | RuntimePhase::WaitingInput
                | RuntimePhase::WaitingExternal
                | RuntimePhase::DebugPaused
        ) {
            return self.reject(
                0,
                CommandErrorCode::StaleRequest,
                "HTML query timeline is no longer running",
            );
        }
        if continuation.epoch != self.epoch.0
            || continuation.context != self.projection_query_context()
            || self
                .vm
                .as_ref()
                .is_none_or(|vm| !continuation.owns_vm_frame(vm))
        {
            return self.html_query_stale(&continuation.origin);
        }
        let payload = match result {
            ServiceResult::Ready { payload } => payload,
            ServiceResult::Error { error } => {
                return self.html_backend_failure(
                    &error.code,
                    &error.message,
                    &continuation.origin,
                );
            }
        };
        if payload.as_slice().len() > MAXIMUM_RESPONSE_BYTES {
            return self.html_query_failure(
                &failure(
                    HtmlQueryErrorKind::ResourceLimit,
                    "HTML measurement response exceeds its byte limit",
                ),
                Some(continuation.origin.clone()),
            );
        }
        let response: HtmlMeasureResponseV2 = match decode_canonical(payload.as_slice()) {
            Ok(response) => response,
            Err(error) => {
                return self.html_backend_failure(
                    "invalid_request",
                    &format!("malformed HTML v2 response: {error}"),
                    &continuation.origin,
                );
            }
        };
        if encode_canonical(&response).ok().as_deref() != Some(payload.as_slice()) {
            return self.html_backend_failure(
                "invalid_request",
                "HTML v2 response contains unknown fields",
                &continuation.origin,
            );
        }
        if response.context != continuation.context {
            return self.html_query_stale(&continuation.origin);
        }
        let identity = continuation
            .transfer
            .as_ref()
            .ok_or_else(invalid_arguments)
            .and_then(|transfer| transfer.validate_identity(&response));
        if let Err(error) = identity {
            return self.html_query_failure(&error, Some(continuation.origin.clone()));
        }
        // Per-probe errors use the same classification as whole-service failures,
        // but only after validating the exact requested probe identity.
        if let era_runtime_protocol::HtmlProbeResultV2::Error { error } = &response.probes[0].result
        {
            return self.html_backend_failure(&error.code, &error.message, &continuation.origin);
        }
        let received = continuation
            .budget
            .charge(payload.as_slice().len(), 0)
            .and_then(|()| continuation.receive(response));
        if let Err(error) = received {
            // The response side is a provider contract, even if future inner code
            // accidentally propagates a source-looking error from a measurement.
            return self.html_query_failure(&error, Some(continuation.origin.clone()));
        }
        match continuation.advance() {
            Err(error) => self.complete_pending_html_failure(&continuation, &error),
            Ok(Advance::Request(payload)) => {
                let encoded = match encode_html_request(&payload, &mut continuation.budget) {
                    Ok(encoded) => encoded,
                    Err(error) => {
                        return self.html_query_failure(&error, Some(continuation.origin.clone()));
                    }
                };
                self.emit_html_measurement(continuation, encoded)
            }
            Ok(Advance::Complete(value)) => self.commit_html_query_completion(&continuation, value),
        }
    }

    fn commit_html_query_completion(
        &mut self,
        continuation: &HtmlQueryContinuation,
        value: PlanPoll,
    ) -> Result<(), RuntimeError> {
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("HTML completion has no VM".into()))?;
        let ready = self.finish_html_query(&vm, continuation, value);
        let outcome = match ready {
            Ok(ready) => commit_completion(
                &mut vm,
                continuation.request,
                VmHostCompletion::Ready(ready),
            ),
            Err(error) => self.complete_html_failure(
                &mut vm,
                continuation.request,
                &error,
                &continuation.origin,
            ),
        };
        self.vm = Some(vm);
        outcome?;
        if !matches!(
            self.phase,
            RuntimePhase::Faulted | RuntimePhase::DebugPaused
        ) {
            self.set_phase(RuntimePhase::Running)?;
        }
        Ok(())
    }

    fn emit_html_measurement(
        &mut self,
        continuation: Box<HtmlQueryContinuation>,
        payload: Vec<u8>,
    ) -> Result<(), RuntimeError> {
        let (operation, operation_version) = continuation.operation.wire();
        let request_id = self.allocate_request()?;
        self.operations.insert_service(
            request_id,
            PendingService::Host(ExternalCompletion::HtmlQuery { continuation }),
        );
        self.emit(
            RuntimeMessage::ServiceRequest(ServiceRequest {
                request_id,
                kind: ServiceKind::PresentationQuery,
                operation: operation.into(),
                operation_version,
                payload: ProtocolBytes::new(payload),
                deadline_ns: None,
            }),
            None,
        )
    }

    fn finish_html_query(
        &mut self,
        vm: &RuntimeVm,
        continuation: &HtmlQueryContinuation,
        value: PlanPoll,
    ) -> Result<HostReady, HtmlQueryError> {
        let mut writes = Vec::new();
        let value = match value {
            PlanPoll::Integer(value) if continuation.line_ticket.is_none() => {
                VmValue::Integer(value)
            }
            PlanPoll::Substring(result) => {
                if let Some(ticket) = &continuation.line_ticket {
                    VmValue::Integer(self.operations.html_lines.finish_step(
                        ticket,
                        self.epoch.0,
                        vm,
                        result,
                        continuation.budget,
                    )?)
                } else {
                    for (index, text) in [&result.head, &result.tail].into_iter().enumerate() {
                        if let Some(target) = global_place_at(vm, "RESULTS", index) {
                            writes.push(HostWrite {
                                target,
                                value: VmValue::String(text.clone()),
                            });
                        }
                    }
                    VmValue::String(result.head)
                }
            }
            _ => return Err(invalid_arguments()),
        };
        // Both RESULTS writes and the function return share one VM-validated commit.
        Ok(HostReady {
            value: Some(value),
            writes,
        })
    }

    fn complete_html_failure(
        &mut self,
        vm: &mut RuntimeVm,
        request: HostRequestId,
        error: &HtmlQueryError,
        origin: &VmExecutionOrigin,
    ) -> Result<(), RuntimeError> {
        if error.origin() == erabasic_html::HtmlQueryErrorOrigin::ScriptInput {
            complete_script_fault_request(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Parse,
                format!("html.query.{:?}: {error}", error.kind),
            )
        } else {
            self.html_query_failure(error, Some(origin.clone()))
        }
    }

    fn complete_pending_html_failure(
        &mut self,
        continuation: &HtmlQueryContinuation,
        error: &HtmlQueryError,
    ) -> Result<(), RuntimeError> {
        if error.origin() != erabasic_html::HtmlQueryErrorOrigin::ScriptInput {
            return self.html_query_failure(error, Some(continuation.origin.clone()));
        }
        // complete_service has consumed exactly this pending request and the caller
        // has validated epoch, projection, frame and measurement identity above.
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("HTML completion has no VM".into()))?;
        let result =
            self.complete_html_failure(&mut vm, continuation.request, error, &continuation.origin);
        self.vm = Some(vm);
        result?;
        if let Some(ticket) = &continuation.line_ticket {
            self.operations.html_lines.discard_failed_step(ticket);
        }
        if self.phase != RuntimePhase::DebugPaused {
            // A local catcher is runnable; an uncaught fault is a queued VM event.
            // Neither case should leave the session waiting for the consumed service.
            self.set_phase(RuntimePhase::Running)?;
        }
        Ok(())
    }

    fn html_query_failure(
        &mut self,
        error: &HtmlQueryError,
        origin: Option<VmExecutionOrigin>,
    ) -> Result<(), RuntimeError> {
        let code = if error.kind == HtmlQueryErrorKind::ResourceLimit {
            FaultCode::ResourceLimit
        } else if error.kind == HtmlQueryErrorKind::InvalidMeasurement {
            FaultCode::ServiceFailure
        } else {
            FaultCode::VmFault
        };
        self.fault(
            code,
            &format!("html.query.{:?}: {error}", error.kind),
            origin,
        )
    }

    fn html_query_stale(&mut self, origin: &VmExecutionOrigin) -> Result<(), RuntimeError> {
        self.html_backend_failure(
            "stale_projection",
            "HTML response does not match the current epoch, frame or projection",
            origin,
        )
    }

    fn html_backend_failure(
        &mut self,
        category: &str,
        message: &str,
        origin: &VmExecutionOrigin,
    ) -> Result<(), RuntimeError> {
        let (code, normalized) = projection_service_failure(category);
        self.fault(
            code,
            &format!("html.service.{normalized} ({category}): {message}"),
            Some(origin.clone()),
        )
    }
}

enum PreparedQuery {
    Ready(VmValue),
    Plan(Box<HtmlQueryContinuation>),
}

fn initial_html_plan(
    request: &VmHostRequest,
    name: &str,
    settings: HtmlStringLengthSettings,
    budget: &mut QueryBudget,
) -> Result<QueryPlan, HtmlQueryError> {
    let source = string(request, 0)?;
    match name {
        "HTML__MEASURE_LENGTH" => QueryPlan::length(source, settings, 1, budget),
        "HTML_STRINGLEN" => QueryPlan::length(
            source,
            settings,
            if request.arguments.len() == 1 {
                0
            } else {
                integer(request, 1)?
            },
            budget,
        ),
        "HTML_SUBSTRING" => QueryPlan::substring(
            source,
            reference_split_pixels(integer(request, 1)?, settings.font_size_pixels),
            budget,
        ),
        // Public STRINGLINES must have been lowered to the private lazy protocol.
        // An eager raw host import cannot reproduce per-iteration argument evaluation.
        _ => Err(failure(
            HtmlQueryErrorKind::InvalidMeasurement,
            "HTML_STRINGLINES requires the compiler's lazy host sequence",
        )),
    }
}

fn query_settings(style: &HtmlQueryStyleV2) -> Result<HtmlStringLengthSettings, HtmlQueryError> {
    let font_size_pixels =
        i32::try_from(style.base.font_millipixels / 1000).map_err(|_| invalid_arguments())?;
    let drawable_width_pixels =
        i32::try_from(style.settings.drawable_width.0 / 1000).map_err(|_| invalid_arguments())?;
    if font_size_pixels <= 0 || drawable_width_pixels <= 0 {
        return Err(invalid_arguments());
    }
    let rgb = |color: &era_runtime_protocol::Color| {
        u32::from(color.red) * 65536 + u32::from(color.green) * 256 + u32::from(color.blue)
    };
    Ok(HtmlStringLengthSettings {
        font_size_pixels,
        drawable_width_pixels,
        prevent_button_wrap: style.settings.prevent_button_wrap,
        legacy_nonbutton_wrap: style.settings.legacy_nonbutton_wrap,
        foreground_rgb: rgb(&style.base.foreground),
        focus_rgb: rgb(&style.settings.button_focus_foreground),
    })
}

fn encode_html_request(
    payload: &HtmlMeasureRequestV2,
    budget: &mut QueryBudget,
) -> Result<Vec<u8>, HtmlQueryError> {
    let bytes = encode_canonical(payload).map_err(|_| invalid_arguments())?;
    if bytes.len() > MAXIMUM_REQUEST_BYTES {
        return Err(failure(
            HtmlQueryErrorKind::ResourceLimit,
            "HTML request exceeds its byte limit",
        ));
    }
    budget.charge(bytes.len(), 0)?;
    Ok(bytes)
}

fn integer(request: &VmHostRequest, index: usize) -> Result<i64, HtmlQueryError> {
    match request.arguments.get(index) {
        Some(VmValue::Integer(value)) => Ok(*value),
        _ => Err(invalid_arguments()),
    }
}
fn string(request: &VmHostRequest, index: usize) -> Result<&str, HtmlQueryError> {
    match request.arguments.get(index) {
        Some(VmValue::String(value)) => Ok(value),
        _ => Err(invalid_arguments()),
    }
}
fn invalid_arguments() -> HtmlQueryError {
    failure(
        HtmlQueryErrorKind::InvalidMeasurement,
        "invalid HTML host arguments or query settings",
    )
}

/// Keep the frontend's existing qualified codes while retaining S04 failure categories.
pub(super) fn projection_service_failure(code: &str) -> (FaultCode, &'static str) {
    match code.strip_prefix("frontend.").unwrap_or(code) {
        "unsupported" | "unsupported_service" => {
            (FaultCode::UnsupportedRuntimeFeature, "unsupported")
        }
        "resource_limit" => (FaultCode::ResourceLimit, "resource_limit"),
        "invalid_request" => (FaultCode::ServiceFailure, "invalid_request"),
        "stale_projection" => (FaultCode::ServiceFailure, "stale_projection"),
        _ => (FaultCode::ServiceFailure, "backend_failure"),
    }
}
