//! Script-check recovery only releases evaluation resources; committed game state remains.

use super::dynamic_form::{
    RuntimeFormCatchTarget, finish_runtime_form_catch, select_runtime_form_catch,
};
use super::{StepError, Vm};
use crate::{Fiber, FiberState, HostRequestId, VmError, VmFault, VmFaultCode};

#[derive(Clone, Copy)]
enum CatchTarget {
    Form(RuntimeFormCatchTarget),
}

impl Vm {
    pub(crate) fn recover_runtime_form_failure(
        &mut self,
        fiber: &mut Fiber,
        error: &crate::ExecutionFailure,
    ) -> Result<bool, StepError> {
        // Recover the nearest active checked form in the actual frame chain.
        let Some((owner_index, target)) =
            fiber
                .frames
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, frame)| {
                    select_runtime_form_catch(fiber.id, frame, error)
                        .map(CatchTarget::Form)
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
        owner.user_calls.truncate(user_calls);
        match target {
            CatchTarget::Form(target) => finish_runtime_form_catch(fiber, target)?,
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
                return Ok((fiber_id, None));
            }
            Ok(false) => error,
            Err(internal) => internal,
        };
        self.abort_path_memo(fiber_id);
        for frame in &fiber.frames {
            self.active_function_memos.remove(&frame.id);
        }
        fiber.clear_runtime_forms();
        let fault = VmFault {
            category: error.category,
            code: error.code,
            message: error.message,
            fiber: fiber_id,
            generation: origin.generation,
            function: origin.function,
            function_name: origin.function_name,
            instruction: origin.instruction,
            command: origin.command,
            source: origin.source,
        };
        fiber.state = FiberState::Faulted(fault.clone());
        self.fibers.insert(fiber_id, fiber);
        Ok((fiber_id, Some(fault)))
    }
}
