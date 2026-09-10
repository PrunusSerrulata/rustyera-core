use super::{ExecutionPolicy, Fiber, InstructionPosition, Vm};

impl Vm {
    pub(in crate::interpreter) fn try_literal_select(
        &self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        policy: ExecutionPolicy,
    ) -> Option<u64> {
        if !policy.allow_function_memo {
            return None;
        }
        // Consult the sparse directory only at SELECTCASE, never on each dispatch.
        let (program, index) = position.resolved_program.or_else(|| {
            let program = self.generations.get(&position.generation)?;
            Some((
                program.as_ref(),
                *program.function_index(position.function)?,
            ))
        })?;
        let plan = program.literal_select_plan(index, position.instruction)?;
        let frame = fiber.frames.last_mut()?;
        if frame.for_loops.len() != plan.loops || frame.select_values.len() != plan.outer_selects {
            return None;
        }
        let target = plan.target(frame.stack.last()?)?;
        let logical = target.additional_instructions.checked_add(1)?;
        if logical > policy.remaining_instructions || logical > u64::from(policy.remaining_quantum)
        {
            return None;
        }
        // Include implicit operand contexts and every failed CASE group's peak.
        // Fall back before mutation so resource faults keep their original PC.
        if frame.operand_slots()?.checked_add(target.peak_extra)?
            > self.config.maximum_operand_stack
        {
            return None;
        }
        let selector = frame.stack.pop()?;
        frame.select_values.push(selector);
        frame.instruction = target.instruction;
        Some(target.additional_instructions)
    }
}
