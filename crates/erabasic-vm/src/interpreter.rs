use std::sync::Arc;

use erabasic_bytecode::{
    BytecodeFunctionKind, BytecodeStorage, BytecodeType, ImportKind, Opcode, SymbolKey, opcode,
};

use crate::state::{
    EventDispatch, EventDispatchEntry, ForLoopState, ProgramGeneration, StructuredScopeKind,
};
use crate::{
    Fiber, FiberId, FiberState, HostReady, ImmediateHostCall, ImmediateHostCallResult,
    NativeCallRequest, NativePlaceView, NativeReady, NativeServiceRegistry, PlaceDescriptor,
    RunBudget, Vm, VmError, VmEvent, VmFault, VmFaultCode, VmHost, VmRunReport, VmRunStop, VmValue,
    bind_persistent_arguments, make_frame, validate_arguments,
};

mod arithmetic;
pub(crate) mod bit_calls;
mod character_ops;
pub(crate) mod compatibility_diagnostics;
mod dispatch;
pub(crate) mod dynamic_form;
mod event_dispatch;
pub(crate) mod existvar;
mod extended_ops;
mod fastpaths;
pub(crate) mod fault_hooks;
mod host_calls;
mod lookup;
pub(crate) mod map_calls;
pub(crate) mod matching;
mod native_ops;
mod operand;
mod recovery;
mod scheduler;
mod special_native;

use character_ops::{character_series, execute_character_mutation, execute_character_query};
use dynamic_form::{
    RuntimeFormStep, begin_runtime_call_text, begin_runtime_form, begin_runtime_form_check,
    resume_runtime_form,
};
use extended_ops::{
    array_snapshot_any_rank, execute_array_copy, execute_array_multi_sort,
    execute_array_multi_sort_ex, execute_random_place_transaction, execute_regex_match,
    global_unindexed_place, indexed_place,
};
use native_ops::{
    array_place, array_snapshot, execute_array_mutation, execute_array_query, execute_bit_mutation,
    execute_encode_to_uni_result, execute_erdname, execute_find_element, execute_get_var,
    execute_getnum, execute_index_by_name, execute_integer_mutation, execute_set_var,
    execute_split_transaction, execute_strjoin, execute_swap_transaction, execute_variable_fill,
    integer_argument, native_implicit_place_views, native_place_views, optional_index,
    validate_native_ready,
};
use operand::{
    assign_binary_tag, concat_strings, exact, map_vm_error, pop, pop_arguments, pop_indices,
    read_u16, read_u32,
};

enum StepOutcome {
    Continue,
    Diagnostic {
        code: &'static str,
        message: &'static str,
        notification: crate::VmDiagnosticNotification,
    },
    BulkProgress(u64),
    // Work completed before a failure still consumes this slice's budget. Keep
    // this carrier internal; the public failure and its catch category are unchanged.
    BulkFailure {
        additional_instructions: u64,
        error: StepError,
    },
    DeferredNative,
    Yielded,
    Blocked,
    Completed(Option<VmValue>),
}

const STRUCTURED_GOTO_DIAGNOSTIC_CODE: &str = "vm.control_flow.goto_into_structured_block";
const STRUCTURED_GOTO_DIAGNOSTIC_MESSAGE: &str = "GOTO entered a structured block without executing its opener; avoid jumping into FOR, REPEAT, or SELECTCASE blocks";

#[derive(Clone, Copy)]
struct ExecutionPolicy {
    allow_function_memo: bool,
    allow_immediate_host: bool,
    remaining_quantum: u32,
    remaining_instructions: u64,
}

type StepError = crate::ExecutionFailure;

struct InstructionPosition<'a> {
    generation: crate::GenerationId,
    function: SymbolKey,
    instruction: usize,
    variable: Option<&'a erabasic_bytecode::BytecodeGlobal>,
    encoded: DispatchInstruction<'a>,
}

#[derive(Clone)]
struct FunctionCursor {
    generation: crate::GenerationId,
    function: SymbolKey,
    index: usize,
    program: Arc<ProgramGeneration>,
}

struct DispatchInstruction<'a> {
    opcode: u16,
    payload: &'a [u8],
}

fn bypassed_select_value() -> VmValue {
    // SELECTCASE expressions cannot produce places, making this an unambiguous
    // snapshot-compatible marker for a scope entered through GOTO.
    VmValue::IntegerPlace(Box::default())
}

fn is_bypassed_select_value(value: &VmValue) -> bool {
    matches!(value, VmValue::IntegerPlace(place) if **place == PlaceDescriptor::default())
}

impl DispatchInstruction<'static> {
    fn trap() -> Self {
        Self {
            opcode: Opcode::Trap as u16,
            payload: &[],
        }
    }
}

impl Vm {
    fn execute_instruction(
        &mut self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        host: &mut impl VmHost,
        natives: &mut NativeServiceRegistry,
        host_calls: &mut u32,
        policy: ExecutionPolicy,
    ) -> Result<StepOutcome, StepError> {
        let opcode = Opcode::try_from(position.encoded.opcode).map_err(|opcode| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                format!("unknown opcode {opcode}"),
            )
        })?;
        if opcode == Opcode::PushString
            && let Some(additional_instructions) =
                self.try_literal_group_match(fiber, position, policy)
        {
            return Ok(StepOutcome::BulkProgress(additional_instructions));
        }
        let frame = fiber
            .frames
            .last_mut()
            .ok_or_else(|| StepError::new(VmFaultCode::InvalidInstruction, "missing frame"))?;
        frame.instruction = frame.instruction.saturating_add(1);
        if let Some(outcome) = self.dispatch_basic(fiber, position, opcode, policy)? {
            return self.finish_dispatch(fiber, outcome);
        }
        if let Some(outcome) = self.dispatch_map_calls(fiber, position, opcode, natives)? {
            return Ok(outcome);
        }
        if let Some(outcome) = self.dispatch_bit_calls(fiber, position, opcode, policy)? {
            return Ok(outcome);
        }
        if let Some(outcome) = self.dispatch_match(fiber, position, opcode, policy)? {
            return self.finish_dispatch(fiber, outcome);
        }
        if let Some(outcome) = self.dispatch_existvar(fiber, position, opcode)? {
            return Ok(outcome);
        }
        if let Some(outcome) = self.dispatch_methods(fiber, position, opcode)? {
            return self.finish_dispatch(fiber, outcome);
        }
        if let Some(outcome) =
            self.dispatch_calls(fiber, position, opcode, host, natives, host_calls, policy)?
        {
            return self.finish_dispatch(fiber, outcome);
        }
        if let Some(outcome) = self.dispatch_terminal(fiber, position, opcode, policy)? {
            return self.finish_dispatch(fiber, outcome);
        }
        Err(StepError::new(
            VmFaultCode::InvalidInstruction,
            format!("unhandled opcode {opcode:?}"),
        ))
    }

    fn finish_dispatch(
        &self,
        fiber: &Fiber,
        outcome: StepOutcome,
    ) -> Result<StepOutcome, StepError> {
        if matches!(
            &outcome,
            StepOutcome::Continue | StepOutcome::Diagnostic { .. }
        ) {
            let stack_len = fiber
                .frames
                .last()
                .map_or(Some(0), crate::state::Frame::operand_slots);
            if stack_len.is_none_or(|len| len > self.config.maximum_operand_stack) {
                return Err(StepError::new(
                    VmFaultCode::ResourceLimit,
                    "maximum operand stack exceeded",
                ));
            }
        }
        Ok(outcome)
    }
}
