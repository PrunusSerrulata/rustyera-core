use std::sync::Arc;

use erabasic_bytecode::{
    BytecodeFunctionKind, BytecodeStorage, BytecodeType, HostSnapshotCapability, ImportKind,
    Opcode, SymbolKey, opcode,
};

use crate::state::{EventDispatch, EventDispatchEntry, ForLoopState, ProgramGeneration};
use crate::{
    Fiber, FiberId, FiberState, HostCallRequest, HostCallResult, HostReady, HostWaitStability,
    NativeCallRequest, NativePlaceView, NativeReady, NativeServiceRegistry, PlaceDescriptor,
    RunBudget, Vm, VmError, VmEvent, VmFault, VmFaultCode, VmHost, VmRunReport, VmRunStop, VmValue,
    WaitingHost, bind_persistent_arguments, make_frame, prepare_dynamic_arguments,
    validate_arguments,
};

mod character_ops;
mod dispatch;
pub(crate) mod dynamic_form;
mod extended_ops;
mod fastpaths;
mod lookup;
mod native_ops;
mod operand;
mod scheduler;

use character_ops::{character_series, execute_character_mutation, execute_character_query};
use dynamic_form::{RuntimeFormStep, begin_runtime_form, resume_runtime_form};
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
    assign_binary_tag, binary_value, exact, map_vm_error, pop, pop_arguments, pop_indices,
    read_u16, read_u32, unary_value,
};

enum StepOutcome {
    Continue,
    BulkProgress(u64),
    DeferredNative,
    Yielded,
    Blocked,
    Completed(Option<VmValue>),
}

#[derive(Clone, Copy)]
struct ExecutionPolicy {
    allow_function_memo: bool,
    remaining_quantum: u32,
    remaining_instructions: u64,
}

struct StepError {
    code: VmFaultCode,
    message: String,
}

impl StepError {
    fn new(code: VmFaultCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

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
        if matches!(&outcome, StepOutcome::Continue) {
            let stack_len = fiber.frames.last().map_or(0, |frame| frame.stack.len());
            if stack_len > self.config.maximum_operand_stack {
                return Err(StepError::new(
                    VmFaultCode::ResourceLimit,
                    "maximum operand stack exceeded",
                ));
            }
        }
        Ok(outcome)
    }
}
