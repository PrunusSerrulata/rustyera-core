//! Compiler-owned LINES tickets can span arbitrary width expressions and stable INPUT.

use erabasic_html::{HtmlQueryError, HtmlQueryErrorKind, HtmlSubstringResult};
use erabasic_vm::{FiberId, FrameId, GenerationId, RuntimeVm, VmHostRequest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::plan::{QueryBudget, failure, limits};

const MAXIMUM_FLOWS: usize = 16;
const MAXIMUM_FLOW_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct HtmlLineFlow {
    epoch: u64,
    fiber: FiberId,
    frame: FrameId,
    depth: usize,
    generation: GenerationId,
    function: erabasic_bytecode::SymbolKey,
    form_scope: Option<erabasic_vm::RuntimeHostScope>,
    pub(super) tail: String,
    pub(super) count: u64,
    pub(super) budget: QueryBudget,
    in_flight: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct HtmlLineFlows {
    next_id: u64,
    entries: BTreeMap<String, HtmlLineFlow>,
}

impl HtmlLineFlows {
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn begin(
        &mut self,
        epoch: u64,
        vm: &RuntimeVm,
        request: &VmHostRequest,
        source: String,
    ) -> Result<String, HtmlQueryError> {
        self.retain_live(vm);
        if self.entries.len() >= MAXIMUM_FLOWS
            || source.len() > limits().maximum_source_bytes
            || self.bytes().saturating_add(source.len()) > MAXIMUM_FLOW_BYTES
        {
            return Err(limit());
        }
        let depth = vm
            .fiber_frame_count(request.fiber)
            .and_then(|count| count.checked_sub(1))
            .ok_or_else(invalid)?;
        let (frame, generation, function) = vm
            .host_frame_identity(request.fiber, depth)
            .ok_or_else(invalid)?;
        if generation != request.origin.generation || function != request.origin.function {
            return Err(invalid());
        }
        let next = self.next_id.checked_add(1).ok_or_else(limit)?;
        let ticket = format!("html-lines:{next}");
        self.next_id = next;
        self.entries.insert(
            ticket.clone(),
            HtmlLineFlow {
                epoch,
                fiber: request.fiber,
                frame,
                depth,
                generation,
                function,
                form_scope: vm.host_request_scope(request.id),
                tail: source,
                count: 0,
                budget: QueryBudget::default(),
                in_flight: false,
            },
        );
        Ok(ticket)
    }

    pub(super) fn get(
        &self,
        ticket: &str,
        epoch: u64,
        vm: &RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<&HtmlLineFlow, HtmlQueryError> {
        let flow = self.entries.get(ticket).ok_or_else(invalid)?;
        if flow.epoch != epoch
            || flow.fiber != request.fiber
            || flow.generation != request.origin.generation
            || flow.form_scope != vm.host_request_scope(request.id)
            || flow.function != request.origin.function
            || !flow.live(vm)
            || vm.fiber_frame_count(request.fiber) != Some(flow.depth + 1)
            || flow.in_flight
        {
            return Err(invalid());
        }
        flow.validate()?;
        Ok(flow)
    }

    pub(super) fn start_step(&mut self, ticket: &str) -> Result<(), HtmlQueryError> {
        let flow = self.entries.get_mut(ticket).ok_or_else(invalid)?;
        if flow.in_flight || flow.tail.is_empty() {
            return Err(invalid());
        }
        flow.in_flight = true;
        Ok(())
    }

    pub(super) fn finish_step(
        &mut self,
        ticket: &str,
        epoch: u64,
        vm: &RuntimeVm,
        result: HtmlSubstringResult,
        budget: QueryBudget,
    ) -> Result<i64, HtmlQueryError> {
        let total = self.bytes();
        let flow = self.entries.get_mut(ticket).ok_or_else(invalid)?;
        if !flow.in_flight || flow.epoch != epoch || !flow.live(vm) {
            return Err(invalid());
        }
        budget.validate()?;
        // A reference do/while with no consumed source could otherwise spin forever.
        if result.consumed_working_bytes == 0 && !result.tail.is_empty() {
            return Err(failure(
                HtmlQueryErrorKind::NoProgress,
                "HTML_STRINGLINES made no progress",
            ));
        }
        if result.tail == flow.tail && !result.tail.is_empty() {
            return Err(failure(
                HtmlQueryErrorKind::NoProgress,
                "HTML_STRINGLINES repeated its tail",
            ));
        }
        if flow.count >= u64::try_from(limits().maximum_lines).unwrap_or(u64::MAX)
            || total
                .saturating_sub(flow.tail.len())
                .saturating_add(result.tail.len())
                > MAXIMUM_FLOW_BYTES
        {
            return Err(limit());
        }
        flow.tail = result.tail;
        flow.count += 1;
        flow.budget = budget;
        flow.in_flight = false;
        i64::try_from(flow.count).map_err(|_| limit())
    }

    // Called only after this runtime has validated and failed its own active request.
    pub(super) fn discard_failed_step(&mut self, ticket: &str) {
        self.entries.remove(ticket);
    }

    pub(super) fn end(&mut self, ticket: &str) -> Result<i64, HtmlQueryError> {
        let flow = self.entries.get(ticket).ok_or_else(invalid)?;
        if flow.in_flight || !flow.tail.is_empty() {
            return Err(invalid());
        }
        let count = i64::try_from(flow.count).map_err(|_| limit())?;
        self.entries.remove(ticket);
        Ok(count)
    }

    pub(crate) fn validate_snapshot(&self, vm: &RuntimeVm, epoch: u64) -> Result<(), String> {
        if self.entries.len() > MAXIMUM_FLOWS || self.bytes() > MAXIMUM_FLOW_BYTES {
            return Err("HTML flow snapshot exceeds its resource limit".into());
        }
        for (ticket, flow) in &self.entries {
            let id = ticket
                .strip_prefix("html-lines:")
                .and_then(|id| id.parse::<u64>().ok());
            if id.is_none_or(|id| {
                id == 0 || id > self.next_id || ticket != &format!("html-lines:{id}")
            }) || flow.epoch != epoch
                || flow.in_flight
                || !flow.live(vm)
                || flow
                    .form_scope
                    .is_some_and(|scope| !vm.host_scope_has_html_ticket(scope, ticket))
            {
                return Err(
                    "HTML flow snapshot has invalid owner, identity, epoch or progress".into(),
                );
            }
            flow.validate().map_err(|error| error.to_string())?;
        }
        for (scope, ticket) in vm.active_html_line_scopes() {
            if !self
                .entries
                .get(&ticket)
                .is_some_and(|flow| flow.form_scope == Some(scope))
            {
                return Err("VM HTML scope has no matching runtime line flow".into());
            }
        }
        Ok(())
    }

    /// Only restore invokes this after validating the exact retained VM frames.
    /// Live hot reload rejects active flows instead of rebinding them to new code.
    pub(crate) fn rebind_epoch(&mut self, epoch: u64) {
        for flow in self.entries.values_mut() {
            flow.epoch = epoch;
        }
    }

    pub(crate) fn retain_live(&mut self, vm: &RuntimeVm) {
        self.entries.retain(|_, flow| flow.live(vm));
    }
    fn bytes(&self) -> usize {
        self.entries
            .values()
            .fold(0usize, |sum, flow| sum.saturating_add(flow.tail.len()))
    }
}

impl HtmlLineFlow {
    fn live(&self, vm: &RuntimeVm) -> bool {
        vm.host_frame_identity(self.fiber, self.depth)
            == Some((self.frame, self.generation, self.function))
            && self
                .form_scope
                .is_none_or(|scope| vm.host_scope_is_live(scope))
    }
    fn validate(&self) -> Result<(), HtmlQueryError> {
        self.budget.validate()?;
        if self.tail.len() > limits().maximum_output_bytes
            || self.count > u64::try_from(limits().maximum_lines).unwrap_or(u64::MAX)
        {
            return Err(limit());
        }
        Ok(())
    }
}

fn invalid() -> HtmlQueryError {
    failure(
        HtmlQueryErrorKind::InvalidMeasurement,
        "invalid or stale HTML line-flow ticket",
    )
}
fn limit() -> HtmlQueryError {
    failure(
        HtmlQueryErrorKind::ResourceLimit,
        "HTML line-flow resource limit exceeded",
    )
}
