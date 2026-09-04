//! Script-check recovery only releases evaluation resources; committed game state remains.

use super::dynamic_form::{
    RuntimeFormCatchTarget, finish_runtime_form_catch, select_runtime_form_catch,
};
use super::existvar::{ExistVarCatchTarget, finish_existvar_catch, select_existvar_catch};
use super::{StepError, Vm};
use crate::{Fiber, FiberState, HostRequestId, VmError, VmFault, VmFaultCode};

#[derive(Clone, Copy)]
enum CatchTarget {
    Form(RuntimeFormCatchTarget),
    Expression(ExistVarCatchTarget),
}

impl Vm {
    pub(crate) fn recover_runtime_form_failure(
        &mut self,
        fiber: &mut Fiber,
        error: &crate::ExecutionFailure,
    ) -> Result<bool, StepError> {
        // A callee's bytecode probe is nearer than its caller's format check.
        // Within one frame, form evaluation started inside the bytecode probe.
        let Some((owner_index, target)) =
            fiber
                .frames
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, frame)| {
                    select_runtime_form_catch(fiber.id, frame, error)
                        .map(CatchTarget::Form)
                        .or_else(|| {
                            select_existvar_catch(frame, error).map(CatchTarget::Expression)
                        })
                        .map(|target| (index, target))
                })
        else {
            return Ok(false);
        };
        let (generation, function, stack_depth, user_calls) = match target {
            CatchTarget::Form(target) => (
                target.generation,
                target.function,
                target.owner_stack_depth,
                target.owner_user_calls,
            ),
            CatchTarget::Expression(target) => (
                target.generation,
                target.function,
                target.stack_index.checked_add(1).ok_or_else(|| {
                    StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "probe stack watermark overflow",
                    )
                })?,
                target.user_calls,
            ),
        };
        let owner = &fiber.frames[owner_index];
        if owner.generation != generation
            || owner.function != function
            || owner.stack.len() < stack_depth
            || owner.user_calls.len() < user_calls
        {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "script-check recovery identity differs",
            ));
        }
        self.invalidate_path_memo(fiber.id);
        for frame in &fiber.frames[owner_index..] {
            self.active_function_memos.remove(&frame.id);
        }
        // A fiber owns at most its top Host wait. Host completion resolves that same
        // token before recovery; no frontend requests or unrelated fibers are cancelled.
        fiber.frames.truncate(owner_index + 1);
        let owner = fiber.frames.last_mut().expect("owner retained");
        owner.stack.truncate(stack_depth);
        owner
            .map_calls
            .retain(|call| call.stack_index < stack_depth);
        owner.user_calls.truncate(user_calls);
        owner
            .bit_calls
            .retain(|call| call.stack_index < stack_depth);
        owner
            .match_calls
            .retain(|call| call.stack_index < stack_depth);
        match target {
            CatchTarget::Form(target) => finish_runtime_form_catch(fiber, target)?,
            CatchTarget::Expression(target) => {
                owner.runtime_form = None;
                finish_existvar_catch(fiber, target)?;
            }
        }
        fiber.state = FiberState::Runnable;
        // Do not reset either watchdog: repeated caught script failures are not an
        // escape from instruction, loop or resource limits.
        Ok(true)
    }

    pub(crate) fn fail_waiting_host(
        &mut self,
        request: HostRequestId,
        error: crate::ExecutionFailure,
    ) -> Result<(crate::FiberId, Option<VmFault>), VmError> {
        let fiber_id = self
            .fibers
            .iter()
            .find_map(|(id, fiber)| match &fiber.state {
                FiberState::WaitingHost(wait) if wait.request == request => Some(*id),
                _ => None,
            })
            .ok_or(VmError::StaleHostRequest(request))?;
        let mut fiber = self
            .fibers
            .remove(&fiber_id)
            .ok_or(VmError::UnknownFiber(fiber_id))?;
        let FiberState::WaitingHost(wait) = &fiber.state else {
            unreachable!("wait matched above");
        };
        let origin = wait.origin.clone();
        let error = match self.recover_runtime_form_failure(&mut fiber, &error) {
            Ok(true) => {
                self.fibers.insert(fiber_id, fiber);
                self.runnable.push_back(fiber_id);
                self.prune_bit_leases();
                return Ok((fiber_id, None));
            }
            Ok(false) => error,
            Err(internal) => internal,
        };
        let fault = VmFault::from_origin(fiber_id, origin, error);
        let published = match self.transition_fault(&mut fiber, fault) {
            super::fault_hooks::FaultTransition::HookStarted => {
                self.runnable.push_back(fiber_id);
                None
            }
            super::fault_hooks::FaultTransition::Published(fault) => Some(*fault),
        };
        self.fibers.insert(fiber_id, fiber);
        self.prune_bit_leases();
        Ok((fiber_id, published))
    }
}
