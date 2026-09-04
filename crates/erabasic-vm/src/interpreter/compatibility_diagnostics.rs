//! One bounded warning queue and generation-scoped identity for runtime compatibility.

use super::{InstructionPosition, Vm, VmEvent};
use crate::state::user_calls::ResolvedUserCall;
use crate::{FiberId, VmDiagnosticNotification};
use erabasic_compat::{IntegerArithmeticWarning, UserCallArityDiagnostic};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompatibilityWarning {
    Arithmetic(IntegerArithmeticWarning),
    ExcessUserArguments,
}

impl CompatibilityWarning {
    fn descriptor(self) -> (u8, &'static str, &'static str) {
        match self {
            Self::Arithmetic(IntegerArithmeticWarning::Overflow) => (
                0,
                "compat.arithmetic.overflow",
                "integer arithmetic overflowed; snake saturation policy applied",
            ),
            Self::Arithmetic(IntegerArithmeticWarning::DivideByZero) => (
                1,
                "compat.arithmetic.divide_by_zero",
                "integer division or remainder by zero returned zero under snake policy",
            ),
            Self::ExcessUserArguments => (
                2,
                "compat.call.excess_arguments",
                "extra user-call arguments were ignored without evaluation under snake policy",
            ),
        }
    }
}

impl Vm {
    pub(super) fn queue_compatibility_warning(&mut self, warning: CompatibilityWarning) {
        // The queue contains at most the three distinct warning kinds, regardless of
        // nested expression size. Diagnostics themselves never enter game history.
        if !self.pending_compatibility_warnings.contains(&warning) {
            self.pending_compatibility_warnings.push(warning);
        }
    }

    pub(crate) fn queue_user_call_diagnostic(
        &mut self,
        call: &ResolvedUserCall,
        syntactic_arguments: usize,
    ) {
        let policy = self.generations[&call.generation]
            .artifact
            .call_compatibility;
        let decision = policy
            .user_argument_policy
            .decide(syntactic_arguments, call.bindings.len());
        if decision.diagnostic == Some(UserCallArityDiagnostic::Warning) {
            self.queue_compatibility_warning(CompatibilityWarning::ExcessUserArguments);
        }
    }

    pub(super) fn drain_compatibility_diagnostics(
        &mut self,
        fiber: FiberId,
        position: &InstructionPosition<'_>,
        events: &mut Vec<VmEvent>,
    ) {
        if self.pending_compatibility_warnings.is_empty() {
            return;
        }
        // Memo entries cannot retain notification effects, including duplicates at an
        // already reported site. Invalidate all enclosing memo candidates as in 2A.
        self.invalidate_path_memo(fiber);
        self.active_function_memos.clear();
        let command = self.command_for_position(position);
        let origin = self.execution_origin(position, &command);
        for warning in self.pending_compatibility_warnings.drain(..) {
            let (tag, code, message) = warning.descriptor();
            let site = (
                position.generation,
                position.function,
                position.instruction,
                tag,
            );
            if self.compatibility_warning_sites.insert(site) {
                events.push(VmEvent::Diagnostic {
                    fiber,
                    code: code.into(),
                    message: message.into(),
                    origin: origin.clone(),
                    notification: VmDiagnosticNotification::LogOnly,
                });
            }
        }
    }
}
