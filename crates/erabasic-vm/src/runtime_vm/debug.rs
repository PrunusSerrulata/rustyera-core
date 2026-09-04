#[allow(clippy::wildcard_imports)]
use super::*;
impl crate::VmDebugInspect for RuntimeVm {
    fn stop_token(&self) -> Option<crate::VmStopToken> {
        self.vm.stop_token()
    }

    fn fibers(
        &self,
        stop: crate::VmStopToken,
        cursor: Option<usize>,
        limit: usize,
    ) -> Result<crate::VmDebugPage<crate::VmDebugFiber>, VmError> {
        self.vm.fibers(stop, cursor, limit)
    }

    fn call_stack(
        &self,
        stop: crate::VmStopToken,
        fiber: FiberId,
    ) -> Result<Vec<crate::VmDebugFrame>, VmError> {
        self.vm.call_stack(stop, fiber)
    }

    fn operand_stack(
        &self,
        stop: crate::VmStopToken,
        fiber: FiberId,
        frame: crate::FrameId,
        cursor: Option<usize>,
        limit: usize,
    ) -> Result<crate::VmDebugPage<crate::VmDebugOperand>, VmError> {
        self.vm.operand_stack(stop, fiber, frame, cursor, limit)
    }

    fn variables(
        &self,
        stop: crate::VmStopToken,
        cursor: Option<usize>,
        limit: usize,
    ) -> Result<crate::VmDebugPage<crate::VmDebugVariable>, VmError> {
        self.vm.variables(stop, cursor, limit)
    }

    fn read_variable(
        &self,
        stop: crate::VmStopToken,
        target: &crate::VmDebugVariableRef,
    ) -> Result<crate::VmDebugVariable, VmError> {
        crate::VmDebugInspect::read_variable(&self.vm, stop, target)
    }
}

impl crate::VmDebugControl for RuntimeVm {
    fn request_pause(&mut self) -> Result<crate::VmDebugStop, VmError> {
        self.vm.request_pause()
    }

    fn continue_execution(&mut self, stop: crate::VmStopToken) -> Result<(), VmError> {
        self.vm.continue_execution(stop)
    }

    fn step(
        &mut self,
        stop: crate::VmStopToken,
        fiber: FiberId,
        kind: crate::VmStepKind,
    ) -> Result<(), VmError> {
        self.vm.step(stop, fiber, kind)
    }

    fn write_variables(
        &mut self,
        stop: crate::VmStopToken,
        writes: &[crate::VmDebugVariableWrite],
    ) -> Result<Vec<crate::VmDebugVariable>, VmError> {
        self.vm.write_variables(stop, writes)
    }

    fn update_breakpoints(
        &mut self,
        breakpoints: &[crate::VmBreakpoint],
        remove: &[u64],
    ) -> Result<Vec<crate::VmResolvedBreakpoint>, VmError> {
        self.vm.update_breakpoints(breakpoints, remove)
    }
}
