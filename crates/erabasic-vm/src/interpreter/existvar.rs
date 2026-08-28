//! Dedicated bytecode catch boundary for EXISTVAR's second source evaluation.
use super::{InstructionPosition, StepError, StepOutcome, dynamic_form, pop, read_u32};
use crate::{ExecutionFailure, Fiber, FrameId, GenerationId, Vm, VmFaultCode, VmValue};
use erabasic_bytecode::{Opcode, SymbolKey};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ExistVarCheckpoint {
    pub begin: usize,
    pub failure: usize,
    pub stack_index: usize,
    pub user_calls: usize,
    pub caught: bool,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct ExistVarCatchTarget {
    pub owner: FrameId,
    pub generation: GenerationId,
    pub function: SymbolKey,
    pub begin: usize,
    pub failure: usize,
    pub stack_index: usize,
    pub user_calls: usize,
}
fn invalid(message: &str) -> StepError {
    StepError::new(VmFaultCode::InvalidInstruction, message)
}

pub(crate) fn variable_name_flags(
    vm: &Vm,
    generation: GenerationId,
    function: SymbolKey,
    source: &str,
) -> Result<i64, StepError> {
    let program = vm
        .generations
        .get(&generation)
        .ok_or_else(|| invalid("EXISTVAR generation missing"))?;
    let Some(variable) = program.scoped_variable(function, source) else {
        return Ok(0);
    };
    let mut value = match variable.value_type {
        erabasic_bytecode::BytecodeType::Integer => 1,
        erabasic_bytecode::BytecodeType::String => 2,
        _ => return Err(invalid("variable declaration has a non-scalar type")),
    };
    if !variable.mutable {
        value |= 4;
    }
    match variable.dimensions.len() {
        2 => value |= 8,
        3 => value |= 16,
        _ => {}
    }
    Ok(value)
}

impl Vm {
    fn finish_existvar_probe(
        &self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
    ) -> Result<(), StepError> {
        let begin = read_u32(position.encoded.payload, 0)? as usize;
        let caught = *position
            .encoded
            .payload
            .get(4)
            .ok_or_else(|| invalid("probe phase missing"))?;
        let owner = fiber
            .frames
            .last()
            .ok_or_else(|| invalid("probe owner missing"))?;
        let checkpoint = owner
            .existvar_checks
            .last()
            .ok_or_else(|| invalid("probe checkpoint missing"))?;
        if checkpoint.begin != begin
            || checkpoint.caught != (caught == 1)
            || caught > 1
            || owner.stack.len() != checkpoint.stack_index + if caught == 1 { 1 } else { 2 }
            || owner.stack.get(checkpoint.stack_index) != Some(&probe_token(begin)?)
            || (caught == 1 && checkpoint.failure != position.instruction)
        {
            return Err(invalid("probe completion identity/phase/stack differs"));
        }
        if caught == 0 {
            let Some(VmValue::String(source)) = owner.stack.last() else {
                return Err(invalid("parse probe source is not String"));
            };
            // Keep the checkpoint active if lexical/name/type validation fails.
            dynamic_form::probe_runtime_expression(
                self,
                position.generation,
                position.function,
                source,
            )?;
        }
        let owner = fiber.frames.last_mut().unwrap();
        let checkpoint = owner.existvar_checks.pop().unwrap();
        owner.stack.truncate(checkpoint.stack_index);
        owner.stack.push(VmValue::Integer(i64::from(caught == 0)));
        Ok(())
    }

    pub(in crate::interpreter) fn dispatch_existvar(
        &mut self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        opcode: Opcode,
    ) -> Result<Option<StepOutcome>, StepError> {
        if !matches!(
            opcode,
            Opcode::ProbeVariableName | Opcode::BeginExistVarProbe | Opcode::FinishExistVarProbe
        ) {
            return Ok(None);
        }
        self.invalidate_path_memo(fiber.id);
        match opcode {
            Opcode::ProbeVariableName => {
                let VmValue::String(source) = pop(&mut fiber
                    .frames
                    .last_mut()
                    .ok_or_else(|| invalid("probe owner missing"))?
                    .stack)?
                else {
                    return Err(invalid("name probe source is not String"));
                };
                let value =
                    variable_name_flags(self, position.generation, position.function, &source)?;
                fiber
                    .frames
                    .last_mut()
                    .unwrap()
                    .stack
                    .push(VmValue::Integer(value));
            }
            Opcode::BeginExistVarProbe => {
                let generation = self
                    .generations
                    .get(&position.generation)
                    .ok_or_else(|| invalid("probe generation missing"))?;
                if !generation
                    .artifact
                    .manifest
                    .compatibility
                    .supports_existvar_expression_probe()
                {
                    return Err(invalid("EXISTVAR mode is unavailable in this identity"));
                }
                let owner = fiber
                    .frames
                    .last_mut()
                    .ok_or_else(|| invalid("probe owner missing"))?;
                if owner
                    .operand_slots()
                    .and_then(|n| n.checked_add(2))
                    .is_none_or(|n| n > self.config.maximum_operand_stack)
                {
                    return Err(StepError::new(
                        VmFaultCode::ResourceLimit,
                        "EXISTVAR checkpoint operand limit",
                    ));
                }
                let checkpoint = ExistVarCheckpoint {
                    begin: position.instruction,
                    failure: read_u32(position.encoded.payload, 0)? as usize,
                    stack_index: owner.stack.len(),
                    user_calls: owner.user_calls.len(),
                    caught: false,
                };
                owner.stack.push(probe_token(position.instruction)?);
                owner.existvar_checks.push(checkpoint);
            }
            Opcode::FinishExistVarProbe => {
                self.finish_existvar_probe(fiber, position)?;
            }
            _ => unreachable!(),
        }
        Ok(Some(StepOutcome::Continue))
    }
}

/// Called per frame by the common nearest-frame selector, after checking that same
/// frame's `runtime_form` (which was entered inside this bytecode boundary).
pub(crate) fn select_existvar_catch(
    frame: &crate::state::Frame,
    error: &ExecutionFailure,
) -> Option<ExistVarCatchTarget> {
    if !error.is_script() {
        return None;
    }
    let checkpoint = frame
        .existvar_checks
        .last()
        .filter(|checkpoint| !checkpoint.caught)?;
    Some(ExistVarCatchTarget {
        owner: frame.id,
        generation: frame.generation,
        function: frame.function,
        begin: checkpoint.begin,
        failure: checkpoint.failure,
        stack_index: checkpoint.stack_index,
        user_calls: checkpoint.user_calls,
    })
}

/// Root recovery first releases child frames and any `user_calls` created after the
/// watermark, and truncates owner.stack to `stack_index` + 1 (retaining its ticket).
pub(crate) fn finish_existvar_catch(
    fiber: &mut Fiber,
    target: ExistVarCatchTarget,
) -> Result<(), StepError> {
    let owner = fiber
        .frames
        .last_mut()
        .filter(|owner| {
            owner.id == target.owner
                && owner.generation == target.generation
                && owner.function == target.function
        })
        .ok_or_else(|| invalid("probe catch owner has not been restored"))?;
    if owner.runtime_form.is_some()
        || owner.user_calls.len() != target.user_calls
        || owner.stack.len() != target.stack_index + 1
        || owner.stack.last() != Some(&probe_token(target.begin)?)
    {
        return Err(invalid(
            "probe catch temporary resources have not been restored",
        ));
    }
    let checkpoint = owner
        .existvar_checks
        .last_mut()
        .filter(|check| {
            !check.caught
                && check.begin == target.begin
                && check.failure == target.failure
                && check.stack_index == target.stack_index
                && check.user_calls == target.user_calls
        })
        .ok_or_else(|| invalid("probe catch target is stale"))?;
    checkpoint.caught = true;
    owner.instruction = target.failure;
    Ok(())
}

pub(crate) fn valid_existvar_state(vm: &Vm, frame: &crate::state::Frame) -> bool {
    let Some(program) = vm.generations.get(&frame.generation) else {
        return false;
    };
    let Some(function) = program.function(frame.function) else {
        return false;
    };
    // Snapshot restoration has already matched the complete checkpoint list against
    // the validator's CFG provenance. Validate each retained checkpoint's runtime
    // phase and watermarks here; do not rescan every probe in the function.
    let mut previous: Option<&ExistVarCheckpoint> = None;
    for checkpoint in &frame.existvar_checks {
        let Some(begin) = function.code.get(checkpoint.begin) else {
            return false;
        };
        let Some(failure) = function.code.get(checkpoint.failure) else {
            return false;
        };
        if Opcode::try_from(begin.opcode) != Ok(Opcode::BeginExistVarProbe)
            || begin.payload.len() != 4
            || read_u32(&begin.payload, 0).ok().map(|v| v as usize) != Some(checkpoint.failure)
            || Opcode::try_from(failure.opcode) != Ok(Opcode::FinishExistVarProbe)
            || failure.payload.len() != 5
            || failure.payload[4] != 1
            || read_u32(&failure.payload, 0).ok().map(|v| v as usize) != Some(checkpoint.begin)
            || !probe_token(checkpoint.begin)
                .is_ok_and(|token| frame.stack.get(checkpoint.stack_index) == Some(&token))
            || checkpoint.user_calls > frame.user_calls.len()
            || (checkpoint.caught
                && (frame.instruction != checkpoint.failure
                    || frame.runtime_form.is_some()
                    || frame.stack.len() != checkpoint.stack_index + 1
                    || frame.user_calls.len() != checkpoint.user_calls))
            || !(checkpoint.caught
                || checkpoint.begin < frame.instruction && frame.instruction < checkpoint.failure)
            || previous.is_some_and(|outer| {
                outer.caught
                    || checkpoint.begin <= outer.begin
                    || checkpoint.failure >= outer.failure
                    || checkpoint.stack_index <= outer.stack_index
            })
        {
            return false;
        }
        previous = Some(checkpoint);
    }
    true
}

fn probe_token(begin: usize) -> Result<VmValue, StepError> {
    i64::try_from(begin)
        .map(VmValue::Integer)
        .map_err(|_| invalid("probe origin exceeds i64"))
}
