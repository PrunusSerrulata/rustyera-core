//! Compare saved leases with the immutable stack provenance from bytecode validation.
use crate::VmValue;
use crate::state::user_calls::UserCallOrigin;
use crate::state::{Fiber, FiberState, Frame};
use erabasic_bytecode::{BytecodeArtifact, ImportKind, Opcode, UserCallSpec};
use erabasic_validator::{ValidatedArtifact, ValidatedStackToken};

pub(super) fn valid_frame(artifact: &ValidatedArtifact, fiber: &Fiber, index: usize) -> bool {
    let Some(frame) = fiber.frames.get(index) else {
        return false;
    };
    if matches!(
        fiber.state,
        FiberState::Completed(_) | FiberState::Cancelled | FiberState::Faulted(_)
    ) {
        // Diagnostic frames may be stopped halfway through a failing instruction.
        // They cannot resume, and must not retain executable lease/checkpoint state.
        return frame.user_calls.is_empty()
            && frame.runtime_form.is_none();
    }
    let child = fiber.frames.get(index + 1);
    let summary = if let Some(call) = child.and_then(|child| child.user_call.as_ref())
        && call.mode.unwinds_caller()
        && let UserCallOrigin::Bytecode { invoke, .. } = call.origin
    {
        if invoke.checked_add(1) != Some(frame.instruction) {
            return false;
        }
        artifact
            .operand_stacks()
            .terminal_user_call(frame.function, invoke)
    } else {
        artifact
            .operand_stacks()
            .before(frame.function, frame.instruction)
    };
    let Some(summary) = summary else {
        return false;
    };
    let Some(missing_result) = unreturned_result(artifact.artifact(), fiber, frame, child) else {
        return false;
    };
    if summary
        .operand_count
        .checked_sub(usize::from(missing_result))
        != Some(frame.stack.len())
    {
        return false;
    }
    let mut user_calls = frame.user_calls.iter();
    for token in &summary.tokens {
        match *token {
            ValidatedStackToken::UserCall {
                stack_index,
                resolve,
                next_slot,
            } => {
                let Some(pending) = user_calls.next() else {
                    return false;
                };
                if pending.stack_index != stack_index
                    || pending.resolve != resolve as usize
                    || pending.next_slot != usize::from(next_slot)
                    || !matches!(frame.stack.get(stack_index), Some(VmValue::String(_)))
                {
                    return false;
                }
            }
        }
    }
    // Both directions matter: deleting a list and forging an extra lease are invalid.
    user_calls.next().is_none()
}

/// The CFG models completed instructions. A suspended call has consumed its inputs
/// but has not pushed its one optional scalar result. Derive that gap from the
/// actual instruction, never from the saved stack length or a caller-supplied count.
fn unreturned_result(
    artifact: &BytecodeArtifact,
    fiber: &Fiber,
    frame: &Frame,
    child: Option<&Frame>,
) -> Option<bool> {
    if frame.runtime_form.is_some() {
        // valid_origin separately binds this root to a String-returning STRFORM.
        return Some(true);
    }
    if child.is_none()
        && !matches!(
            fiber.state,
            FiberState::WaitingHost(_) | FiberState::WaitingResume(_)
        )
    {
        return Some(false);
    }
    let function = artifact
        .functions
        .iter()
        .find(|function| function.key == frame.function)?;
    let previous = frame.instruction.checked_sub(1)?;
    let instruction = function.code.get(previous)?;
    let opcode = Opcode::try_from(instruction.opcode).ok()?;
    if let Some(child) = child {
        return match opcode {
            Opcode::InvokeUserCall => {
                let resolve =
                    u32::from_le_bytes(instruction.payload.get(..4)?.try_into().ok()?) as usize;
                let origin = function.code.get(resolve)?;
                let spec = UserCallSpec::decode(&origin.payload).ok()?;
                // Full child/caller/generation/origin binding remains in user_calls/validation.
                Some(spec.mode.expected_result().is_some())
            }
            Opcode::Call => {
                let slot =
                    u32::from_le_bytes(instruction.payload.get(..4)?.try_into().ok()?) as usize;
                let import = function.imports.get(slot)?;
                if import.kind != ImportKind::Function || import.key != child.function {
                    return None;
                }
                let target = artifact
                    .functions
                    .iter()
                    .find(|target| target.key == import.key)?;
                if child.return_value_to_caller != target.result.is_some() {
                    return None;
                }
                Some(target.result.is_some())
            }
            Opcode::InvokeEvent
                if frame.event_dispatch.is_some() && !child.return_value_to_caller =>
            {
                Some(false)
            }
            _ => None,
        };
    }
    if matches!(fiber.state, FiberState::WaitingResume(_)) {
        return (opcode == Opcode::AwaitResume).then_some(true);
    }
    let FiberState::WaitingHost(wait) = &fiber.state else {
        return None;
    };
    if opcode != Opcode::CallHost
        || wait.origin.generation != frame.generation
        || wait.origin.function != frame.function
        || wait.origin.instruction as usize != previous
    {
        return None;
    }
    let slot = u32::from_le_bytes(instruction.payload.get(..4)?.try_into().ok()?) as usize;
    let import = function.imports.get(slot)?;
    if import.kind != ImportKind::Host || import.key != wait.import.key {
        return None;
    }
    let host = artifact
        .host_imports
        .iter()
        .find(|host| host.import.key == import.key)?;
    (host.import.result == wait.result).then_some(host.import.result.is_some())
}
