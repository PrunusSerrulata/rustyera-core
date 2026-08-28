//! Shared successful return transition for bytecode and Host `ReturnCurrent`.

use super::{Fiber, FiberState};
use crate::{Vm, VmError, VmValue, make_frame};
use erabasic_bytecode::BytecodeFunctionKind;

#[derive(Debug)]
pub(crate) enum FrameReturn {
    Continue,
    Completed(Option<VmValue>),
}

impl Vm {
    fn validate_return_value(&self, fiber: &Fiber, value: Option<&VmValue>) -> Result<(), VmError> {
        let active = fiber
            .frames
            .last()
            .ok_or_else(|| VmError::InvalidState("returning frame is missing".into()))?;
        let function = self
            .generations
            .get(&active.generation)
            .and_then(|generation| generation.function(active.function))
            .ok_or(VmError::MissingFunction(active.function))?;
        if function.kind == BytecodeFunctionKind::Method
            && value.map(VmValue::value_type) != function.result
        {
            return Err(VmError::InvalidState(
                "method returned an incompatible scalar type".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn return_frame(
        &mut self,
        fiber: &mut Fiber,
        value: Option<VmValue>,
        instruction: Option<usize>,
        allow_function_memo: bool,
    ) -> Result<FrameReturn, VmError> {
        self.validate_return_value(fiber, value.as_ref())?;
        let mut first = true;
        loop {
            let active = fiber.frames.last().expect("returning frame was checked");
            if let Some(call) = &active.user_call
                && fiber
                    .frames
                    .iter()
                    .rev()
                    .nth(1)
                    .is_none_or(|caller| caller.id != call.caller)
            {
                return Err(VmError::InvalidState(
                    "user-call return has lost its caller".into(),
                ));
            }
            let returned = fiber.frames.pop().expect("returning frame was checked");
            if first {
                if let Some(instruction) = instruction {
                    self.confirm_path_memo_result_read(fiber.id, returned.id, instruction);
                }
            } else {
                // A deferred JUMP completes the suspended caller without executing its
                // following instruction. Never fabricate a memo trace for that path.
                self.invalidate_path_memo(fiber.id);
            }
            if let Some(key) = self.active_function_memos.remove(&returned.id)
                && first
                && allow_function_memo
                && let Some(value) = value.as_ref()
                && let Some(entry) = self.capture_function_memo_entry(&key, value.clone())
            {
                self.cache_function_memo(key, entry);
            }
            self.complete_path_memo(fiber, returned.id, value.as_ref());
            if !fiber.frames.is_empty() {
                // Call completion is finite forward progress; a surrounding backward
                // branch retains its independent watchdog counter.
                fiber.consecutive_budget_exhaustions = 0;
            }
            if returned
                .user_call
                .as_ref()
                .is_some_and(|call| call.mode.unwinds_caller())
            {
                // The caller's LOCAL/REF cells stayed alive throughout the callee. Only
                // a successful return reaches this point; fault/cancel never use it.
                first = false;
                continue;
            }
            let Some(caller) = fiber.frames.last_mut() else {
                fiber.state = FiberState::Completed(value.clone());
                return Ok(FrameReturn::Completed(value));
            };
            if returned.return_value_to_caller
                && let Some(value) = value.clone()
            {
                caller.stack.push(value);
            }
            let next_event = caller.event_dispatch.as_mut().and_then(|dispatch| {
                if dispatch.active.single && value == Some(VmValue::Integer(1)) {
                    while dispatch
                        .pending
                        .front()
                        .is_some_and(|entry| entry.group == dispatch.active.group)
                    {
                        dispatch.pending.pop_front();
                    }
                }
                dispatch
                    .pending
                    .pop_front()
                    .inspect(|next| dispatch.active = next.clone())
            });
            if let Some(next) = next_event {
                let generation = caller.generation;
                let frame_id = self.allocate_frame_id();
                let program = self
                    .generations
                    .get(&generation)
                    .ok_or_else(|| VmError::InvalidState("event generation is missing".into()))?;
                let target = program
                    .function(next.function)
                    .ok_or(VmError::MissingFunction(next.function))?;
                self.memory.ensure_function_statics(
                    generation,
                    target.key,
                    program.function_statics(target.key),
                );
                fiber.frames.push(make_frame(
                    frame_id,
                    generation,
                    target,
                    program.function_locals(target.key),
                    Vec::new(),
                    false,
                    true,
                ));
            } else {
                caller.event_dispatch = None;
            }
            return Ok(FrameReturn::Continue);
        }
    }
}
