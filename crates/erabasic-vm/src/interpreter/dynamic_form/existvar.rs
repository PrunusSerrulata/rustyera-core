//! Form-side EXISTVAR uses the same staged order and existing checkpoint recovery.
use super::{
    Expr, Fiber, FormatCheckpoint, MAX_RUNTIME_FORM_NESTING, RuntimeFormContinuation,
    RuntimeFormTask, StepError, Vm, VmFaultCode, VmValue, frontend, owner_frame, resource_limit,
    support,
};

impl RuntimeFormContinuation {
    pub(super) fn schedule_existvar(
        &mut self,
        arguments: &[Option<Expr>],
    ) -> Result<(), StepError> {
        let source = arguments.first().cloned().flatten().ok_or_else(|| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                "validated EXISTVAR source missing",
            )
        })?;
        self.work.push(RuntimeFormTask::ExistVarFirst {
            source: source.clone(),
            mode: arguments.get(1).cloned().flatten(),
        });
        self.work.push(RuntimeFormTask::Evaluate(source));
        Ok(())
    }
    pub(super) fn existvar_first(
        &mut self,
        vm: &Vm,
        source: Expr,
        mode: Option<Expr>,
    ) -> Result<(), StepError> {
        let VmValue::String(name) = self.pop_value("EXISTVAR first source missing")? else {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "EXISTVAR first source type changed",
            ));
        };
        let flags =
            super::super::existvar::variable_name_flags(vm, self.generation, self.function, &name)?;
        self.values.push(VmValue::Integer(flags));
        if let Some(mode) = mode {
            self.work.push(RuntimeFormTask::ExistVarMode { source });
            self.work.push(RuntimeFormTask::Evaluate(mode));
        }
        Ok(())
    }
    pub(super) fn existvar_mode(
        &mut self,
        vm: &Vm,
        fiber: &Fiber,
        source: Expr,
    ) -> Result<(), StepError> {
        let mode = self.pop_integer("EXISTVAR mode missing")?;
        if mode == 0 {
            return Ok(());
        }
        self.pop_integer("EXISTVAR first lookup result missing")?;
        let program = vm.generations.get(&self.generation).ok_or_else(|| {
            StepError::new(VmFaultCode::MissingSymbol, "probe generation missing")
        })?;
        if !program
            .artifact
            .manifest
            .compatibility
            .supports_existvar_expression_probe()
        {
            return Err(support::permission_denied(
                "EXISTVAR expression mode unavailable",
            ));
        }
        if self.checkpoints.len() >= MAX_RUNTIME_FORM_NESTING {
            return Err(resource_limit("EXISTVAR checkpoint nesting limit"));
        }
        let owner = owner_frame(fiber, self.frame)?;
        let id = self.next_checkpoint;
        self.next_checkpoint = id
            .checked_add(1)
            .ok_or_else(|| resource_limit("probe identity exhausted"))?;
        self.checkpoints.push(FormatCheckpoint {
            id,
            expression_probe: true,
            work_depth: self.work.len(),
            value_depth: self.values.len(),
            output_depth: self.outputs.len(),
            owner_stack_depth: owner.stack.len(),
            owner_user_calls: owner.user_calls.len(),
        });
        self.work.push(RuntimeFormTask::FinishExpressionProbe(id));
        self.work.push(RuntimeFormTask::Evaluate(source));
        Ok(())
    }
    pub(super) fn finish_expression_probe(&mut self, vm: &Vm, id: u64) -> Result<(), StepError> {
        let Some(VmValue::String(source)) = self.values.last() else {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "EXISTVAR second source missing",
            ));
        };
        if source.len() > self.remaining_source_bytes {
            return Err(resource_limit(
                "EXISTVAR repeated sources exceed parser limit",
            ));
        }
        self.remaining_source_bytes -= source.len();
        // The dispatcher has popped this completion task. Keep its checkpoint
        // marker present while parsing so script-failure recovery can still select it.
        self.work.push(RuntimeFormTask::FinishExpressionProbe(id));
        frontend::probe_runtime_expression(vm, self.generation, self.function, source)?;
        self.work.pop();
        // The normal CHECK completion expects a String and emits 1. Its structural
        // checks apply unchanged once this specific probe's parse has succeeded.
        self.finish_checked_form(id)
    }
}
