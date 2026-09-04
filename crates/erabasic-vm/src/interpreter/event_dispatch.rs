use std::collections::VecDeque;

#[allow(clippy::wildcard_imports)]
use super::*;

impl Vm {
    /// Start the canonical event-group order on the current caller frame.
    /// Missing groups are a normal `Ok(false)` result; malformed or resource-bound
    /// groups are classified failures for the caller to publish or attach.
    pub(super) fn start_event_dispatch(
        &mut self,
        fiber: &mut Fiber,
        generation: crate::GenerationId,
        name: &str,
    ) -> Result<bool, StepError> {
        let program = self.generations.get(&generation).cloned().ok_or_else(|| {
            StepError::new(VmFaultCode::MissingSymbol, "event generation is missing")
        })?;
        let Some(group) = program
            .artifact
            .event_groups
            .iter()
            .find(|group| group.name.eq_ignore_ascii_case(name))
        else {
            return Ok(false);
        };
        let groups: &[(&[erabasic_bytecode::BytecodeEventEntry], u8)] = if group.only.is_empty() {
            &[(&group.priority, 1), (&group.normal, 2), (&group.later, 3)]
        } else {
            &[(&group.only, 0)]
        };
        let mut pending = VecDeque::new();
        for (entries, group_id) in groups {
            pending.extend(entries.iter().map(|entry| EventDispatchEntry {
                function: entry.function,
                single: entry.single,
                group: *group_id,
            }));
        }
        let Some(active) = pending.pop_front() else {
            return Ok(false);
        };
        if fiber.frames.len() >= self.config.maximum_call_depth {
            return Err(StepError::classified(
                crate::FaultCategory::ResourceLimit,
                VmFaultCode::ResourceLimit,
                "maximum call depth exceeded while starting event dispatch",
            ));
        }
        let target = program.function(active.function).ok_or_else(|| {
            StepError::new(VmFaultCode::MissingSymbol, "event function is missing")
        })?;
        if target.kind != BytecodeFunctionKind::Event || !target.parameters.is_empty() {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "event group target is not a parameterless event",
            ));
        }
        self.memory.ensure_function_statics(
            generation,
            target.key,
            program.function_statics(target.key),
        );
        fiber
            .frames
            .last_mut()
            .ok_or_else(|| StepError::new(VmFaultCode::InvalidInstruction, "missing event caller"))?
            .event_dispatch = Some(EventDispatch { active, pending });
        fiber.frames.push(make_frame(
            self.allocate_frame_id(),
            generation,
            target,
            program.function_locals(target.key),
            Vec::new(),
            false,
            true,
        ));
        Ok(true)
    }
}
