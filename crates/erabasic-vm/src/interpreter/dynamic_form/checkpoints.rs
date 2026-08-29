//! Script failure recovery plans are selected before normal frame/resource unwind.
//! This module only restores temporary form-evaluation state after the owner confirms unwind.
use super::{
    Deserialize, Fiber, FrameId, GenerationId, MAX_RUNTIME_FORM_NESTING, NativeServiceRegistry,
    RuntimeFormContinuation, RuntimeFormTask, Serialize, StepError, SymbolKey, Vm, VmFaultCode,
    VmValue, owner_frame, resource_limit, support,
};
use crate::{ExecutionFailure, FiberId};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) enum FormatCheckpointKind {
    CheckedForm,
    ExpressionProbe,
    CallTextArguments(erabasic_bytecode::CallTextSpec),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct FormatCheckpoint {
    pub id: u64,
    pub kind: FormatCheckpointKind,
    pub reference_bindings: bool,
    pub call_plan: Option<u64>,
    pub work_depth: usize,
    pub value_depth: usize,
    pub output_depth: usize,
    pub owner_stack_depth: usize,
    pub owner_user_calls: usize,
    pub host_scope_frontier: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeFormCatchTarget {
    pub fiber: FiberId,
    pub owner: FrameId,
    pub generation: GenerationId,
    pub function: SymbolKey,
    pub instruction: usize,
    pub checkpoint: u64,
    kind: FormatCheckpointKind,
    pub owner_stack_depth: usize,
    pub owner_user_calls: usize,
}

/// Read-only selection. The caller must unwind children via the normal lifecycle,
/// release their Host/pending/memo resources, and restore only the owner's temporary
/// operand suffix before calling `finish_runtime_form_catch`.
pub(crate) fn select_runtime_form_catch(
    fiber: FiberId,
    frame: &crate::state::Frame,
    failure: &ExecutionFailure,
) -> Option<RuntimeFormCatchTarget> {
    if !failure.is_script() {
        return None;
    }
    let form = frame.runtime_form.as_ref()?;
    let checkpoint = form.checkpoints.last()?;
    (form.frame == frame.id && form.checkpoints_valid()).then_some(RuntimeFormCatchTarget {
        fiber,
        owner: form.frame,
        generation: form.generation,
        function: form.function,
        instruction: form.instruction,
        checkpoint: checkpoint.id,
        kind: checkpoint.kind,
        owner_stack_depth: checkpoint.owner_stack_depth,
        owner_user_calls: checkpoint.owner_user_calls,
    })
}

/// Never pops VM frames, clears Host requests, rewrites REF aliases, or rolls back
/// committed memory/service effects. A stale or unprepared plan is an invariant fault.
pub(crate) fn finish_runtime_form_catch(
    fiber: &mut Fiber,
    target: RuntimeFormCatchTarget,
) -> Result<(), StepError> {
    if fiber.id != target.fiber {
        return Err(invalid_checkpoint("format catch belongs to another fiber"));
    }
    let owner = fiber
        .frames
        .last_mut()
        .filter(|frame| {
            frame.id == target.owner
                && frame.generation == target.generation
                && frame.function == target.function
        })
        .ok_or_else(|| {
            invalid_checkpoint("format catch owner is not the active frame after unwind")
        })?;
    if owner.stack.len() != target.owner_stack_depth
        || owner.user_calls.len() != target.owner_user_calls
    {
        return Err(invalid_checkpoint(
            "format catch owner temporary state has not been restored",
        ));
    }
    let form = owner
        .runtime_form
        .as_mut()
        .ok_or_else(|| invalid_checkpoint("format catch lost its continuation"))?;
    if form.instruction != target.instruction || !form.checkpoints_valid() {
        return Err(invalid_checkpoint(
            "format catch continuation identity is invalid",
        ));
    }
    let checkpoint = form
        .checkpoints
        .last()
        .filter(|checkpoint| {
            checkpoint.id == target.checkpoint
                && checkpoint.kind == target.kind
                && checkpoint.owner_stack_depth == target.owner_stack_depth
                && checkpoint.owner_user_calls == target.owner_user_calls
        })
        .cloned()
        .ok_or_else(|| invalid_checkpoint("format catch checkpoint is stale"))?;
    form.work.truncate(checkpoint.work_depth);
    form.values.truncate(checkpoint.value_depth);
    form.outputs.truncate(checkpoint.output_depth);
    form.checkpoints.pop();
    form.awaiting_user_call = None;
    form.abandon_host_scopes(checkpoint.host_scope_frontier);
    form.reference_bindings = checkpoint.reference_bindings;
    form.restore_call_plan(checkpoint.call_plan)?;
    // Do not refund parser/work budgets: failed checks still consumed resources.
    match target.kind {
        FormatCheckpointKind::CheckedForm | FormatCheckpointKind::ExpressionProbe => {
            form.values.push(VmValue::Integer(0));
        }
        FormatCheckpointKind::CallTextArguments(spec) => {
            if !matches!(form.completion, super::RuntimeFormRoot::Call { spec: root, .. } if root == spec)
            {
                return Err(invalid_checkpoint(
                    "CALLSTR argument catch root differs from its checkpoint",
                ));
            }
            form.completion = super::RuntimeFormRoot::Call { spec, failed: true };
        }
    }
    Ok(())
}

impl RuntimeFormContinuation {
    pub(super) fn begin_call_text_argument_checkpoint(
        &mut self,
        fiber: &Fiber,
        spec: erabasic_bytecode::CallTextSpec,
    ) -> Result<Option<u64>, StepError> {
        if !spec.mode.has_catch() {
            return Ok(None);
        }
        if self.checkpoints.len() >= MAX_RUNTIME_FORM_NESTING {
            return Err(resource_limit("CALLSTR checkpoint nesting limit"));
        }
        if self.awaiting_user_call.is_some() {
            return Err(invalid_checkpoint(
                "CALLSTR argument checkpoint began during a suspended user call",
            ));
        }
        let owner = owner_frame(fiber, self.frame)?;
        let id = self.next_checkpoint;
        self.next_checkpoint = id
            .checked_add(1)
            .ok_or_else(|| resource_limit("CALLSTR checkpoint identity exhausted"))?;
        self.checkpoints.push(FormatCheckpoint {
            id,
            kind: FormatCheckpointKind::CallTextArguments(spec),
            reference_bindings: self.reference_bindings,
            call_plan: self.current_call_plan,
            work_depth: self.work.len(),
            value_depth: self.values.len(),
            output_depth: self.outputs.len(),
            owner_stack_depth: owner.stack.len(),
            owner_user_calls: owner.user_calls.len(),
            host_scope_frontier: self.next_host_scope,
        });
        self.work
            .push(RuntimeFormTask::FinishCallTextArgumentCatch(id));
        Ok(Some(id))
    }

    pub(super) fn finish_call_text_argument_checkpoint(
        &mut self,
        id: u64,
    ) -> Result<(), StepError> {
        let checkpoint = self
            .checkpoints
            .last()
            .filter(|checkpoint| {
                checkpoint.id == id
                    && matches!(checkpoint.kind, FormatCheckpointKind::CallTextArguments(_))
            })
            .ok_or_else(|| invalid_checkpoint("CALLSTR argument completion has no checkpoint"))?;
        if self.work.len() != checkpoint.work_depth.saturating_add(1)
            || self.values.len() != checkpoint.value_depth
            || self.outputs.len() != checkpoint.output_depth
            || self.work.last() != Some(&RuntimeFormTask::FinishCallTextArgumentCatch(id))
        {
            return Err(invalid_checkpoint(
                "CALLSTR argument conversion has an invalid completion shape",
            ));
        }
        self.work.pop();
        self.checkpoints.pop();
        Ok(())
    }

    pub(super) fn begin_checked_form(
        &mut self,
        vm: &Vm,
        fiber: &Fiber,
        natives: &NativeServiceRegistry,
        source: &str,
    ) -> Result<(), StepError> {
        let program = vm
            .generations
            .get(&self.generation)
            .ok_or_else(|| invalid_checkpoint("STRFORMCHECK generation is missing"))?;
        if !program
            .artifact
            .manifest
            .compatibility
            .supports_checked_runtime_forms()
        {
            return Err(support::permission_denied(
                "STRFORMCHECK is unavailable in this compatibility identity",
            ));
        }
        if self.checkpoints.len() >= MAX_RUNTIME_FORM_NESTING {
            return Err(resource_limit("STRFORMCHECK checkpoint nesting limit"));
        }
        if self.awaiting_user_call.is_some() {
            return Err(invalid_checkpoint(
                "STRFORMCHECK cannot begin during a suspended user call",
            ));
        }
        let owner = owner_frame(fiber, self.frame)?;
        let id = self.next_checkpoint;
        self.next_checkpoint = id
            .checked_add(1)
            .ok_or_else(|| resource_limit("STRFORMCHECK checkpoint identity exhausted"))?;
        self.checkpoints.push(FormatCheckpoint {
            id,
            kind: FormatCheckpointKind::CheckedForm,
            reference_bindings: self.reference_bindings,
            call_plan: self.current_call_plan,
            work_depth: self.work.len(),
            value_depth: self.values.len(),
            output_depth: self.outputs.len(),
            owner_stack_depth: owner.stack.len(),
            owner_user_calls: owner.user_calls.len(),
            host_scope_frontier: self.next_host_scope,
        });
        self.work.push(RuntimeFormTask::FinishCheck(id));
        // Both parsing and actual expansion are inside the checkpoint. The outer
        // String argument has already been evaluated by the caller/work machine.
        self.schedule_form_source(vm, natives, source)
    }

    pub(super) fn finish_checked_form(&mut self, id: u64) -> Result<(), StepError> {
        let checkpoint = self
            .checkpoints
            .last()
            .filter(|checkpoint| checkpoint.id == id)
            .ok_or_else(|| {
                invalid_checkpoint("STRFORMCHECK completion marker has no checkpoint")
            })?;
        if self.work.len() != checkpoint.work_depth
            || self.outputs.len() != checkpoint.output_depth
            || self.values.len() != checkpoint.value_depth.saturating_add(1)
            || !matches!(self.values.last(), Some(VmValue::String(_)))
        {
            return Err(invalid_checkpoint(
                "STRFORMCHECK expansion has an invalid completion shape",
            ));
        }
        self.values.pop();
        self.values.push(VmValue::Integer(1));
        self.checkpoints.pop();
        Ok(())
    }

    pub(super) fn checkpoints_valid(&self) -> bool {
        if self.next_checkpoint == 0 || self.checkpoints.len() > MAX_RUNTIME_FORM_NESTING {
            return false;
        }
        let mut previous: Option<&FormatCheckpoint> = None;
        for checkpoint in &self.checkpoints {
            if checkpoint.host_scope_frontier == 0
                || checkpoint.host_scope_frontier > self.next_host_scope
                || checkpoint.id == 0
                || checkpoint.id >= self.next_checkpoint
                || checkpoint.value_depth > self.values.len()
                || checkpoint.output_depth > self.outputs.len()
                || self.work.get(checkpoint.work_depth)
                    != Some(&match checkpoint.kind {
                        FormatCheckpointKind::CheckedForm => {
                            RuntimeFormTask::FinishCheck(checkpoint.id)
                        }
                        FormatCheckpointKind::ExpressionProbe => {
                            RuntimeFormTask::FinishExpressionProbe(checkpoint.id)
                        }
                        FormatCheckpointKind::CallTextArguments(_) => {
                            RuntimeFormTask::FinishCallTextArgumentCatch(checkpoint.id)
                        }
                    })
            {
                return false;
            }
            if previous.is_some_and(|outer| {
                checkpoint.host_scope_frontier < outer.host_scope_frontier
                    || checkpoint.id <= outer.id
                    || checkpoint.work_depth <= outer.work_depth
                    || checkpoint.value_depth < outer.value_depth
                    || checkpoint.output_depth < outer.output_depth
                    || checkpoint.owner_stack_depth != outer.owner_stack_depth
                    || checkpoint.owner_user_calls != outer.owner_user_calls
            }) {
                return false;
            }
            previous = Some(checkpoint);
        }
        self.work
            .iter()
            .filter(|task| {
                matches!(
                    task,
                    RuntimeFormTask::FinishCheck(_)
                        | RuntimeFormTask::FinishExpressionProbe(_)
                        | RuntimeFormTask::FinishCallTextArgumentCatch(_)
                )
            })
            .count()
            == self.checkpoints.len()
    }
}

fn invalid_checkpoint(message: &str) -> StepError {
    StepError::new(VmFaultCode::InvalidInstruction, message)
}
