#[allow(clippy::wildcard_imports)]
use super::*;
use crate::state::PendingFaultHook;

pub(crate) enum FaultTransition {
    HookStarted,
    Published(Box<VmFault>),
}

impl Vm {
    /// Move a fiber through the single terminal-fault boundary. Script faults may
    /// start one final hook; every failure after that point is attached as the
    /// secondary fault and can never recursively dispatch another hook.
    pub(crate) fn transition_fault(
        &mut self,
        fiber: &mut Fiber,
        fault: VmFault,
    ) -> FaultTransition {
        self.abort_path_memo(fiber.id);
        for frame in &fiber.frames {
            self.active_function_memos.remove(&frame.id);
        }
        fiber.clear_runtime_forms();

        if let FiberState::Faulted(primary) = &fiber.state {
            let mut primary = primary.clone();
            primary.attach_secondary(fault);
            return FaultTransition::Published(Box::new(Self::publish_fault(fiber, primary)));
        }
        if fiber.fault_hook.is_some() {
            return FaultTransition::Published(Box::new(Self::publish_pending_fault_hook(
                fiber,
                Some(fault),
            )));
        }
        let crate::FaultCategory::Script(kind) = fault.category else {
            return FaultTransition::Published(Box::new(Self::publish_fault(fiber, fault)));
        };
        let hook_name = if kind == crate::ScriptFaultKind::ExplicitThrow {
            "BEFORE_THROW"
        } else {
            "BEFORE_ERROR"
        };
        let enabled = self
            .generations
            .get(&fault.generation)
            .is_some_and(|program| program.artifact.call_compatibility.before_error_throw_hooks);
        if !enabled || fiber.frames.is_empty() {
            return FaultTransition::Published(Box::new(Self::publish_fault(fiber, fault)));
        }

        let original_frame_depth = fiber.frames.len();
        match self.start_event_dispatch(fiber, fault.generation, hook_name) {
            Ok(true) => {
                fiber.fault_hook = Some(PendingFaultHook {
                    original: fault,
                    original_frame_depth,
                });
                fiber.state = FiberState::Runnable;
                FaultTransition::HookStarted
            }
            Ok(false) => FaultTransition::Published(Box::new(Self::publish_fault(fiber, fault))),
            Err(error) => {
                let mut hook_origin = fault.origin();
                hook_origin.command = hook_name.into();
                let secondary = VmFault::from_origin(fiber.id, hook_origin, error);
                let mut fault = fault;
                fault.attach_secondary(secondary);
                FaultTransition::Published(Box::new(Self::publish_fault(fiber, fault)))
            }
        }
    }

    pub(super) fn finish_fault_hook_success(fiber: &mut Fiber) -> Option<VmFault> {
        let hook = fiber.fault_hook.as_ref()?;
        if fiber.frames.len() != hook.original_frame_depth
            || fiber
                .frames
                .last()
                .is_some_and(|frame| frame.event_dispatch.is_some())
        {
            return None;
        }
        Some(Self::publish_pending_fault_hook(fiber, None))
    }

    pub(super) fn finish_fault_hook_terminal(fiber: &mut Fiber) -> VmFault {
        Self::publish_pending_fault_hook(fiber, None)
    }

    fn publish_pending_fault_hook(fiber: &mut Fiber, secondary: Option<VmFault>) -> VmFault {
        let mut hook = fiber
            .fault_hook
            .take()
            .expect("final-fault publication requires a pending hook");
        fiber.frames.truncate(hook.original_frame_depth);
        if let Some(frame) = fiber.frames.last_mut() {
            frame.event_dispatch = None;
        }
        if let Some(secondary) = secondary {
            hook.original.attach_secondary(secondary);
        }
        Self::publish_fault(fiber, hook.original)
    }

    fn publish_fault(fiber: &mut Fiber, fault: VmFault) -> VmFault {
        fiber.state = FiberState::Faulted(fault.clone());
        fault
    }
}
