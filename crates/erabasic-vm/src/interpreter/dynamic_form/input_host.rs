//! Finite runtime-owned input APIs use the existing Host wait/ready path.
//! No arbitrary Host import or external service becomes callable from a form.
use super::{
    BytecodeType, Expr, RuntimeFormContinuation, RuntimeFormTask, StepError, owner_frame,
    owner_frame_mut, support,
};
use crate::state::{FiberState, WaitingHost};
use crate::{
    Fiber, HostCallRequest, HostCallResult, HostWaitStability, Vm, VmFaultCode, VmHost, VmValue,
};

pub(super) struct InputHostInvocation<'a> {
    pub(super) name: &'a str,
    pub(super) arguments: Vec<VmValue>,
    pub(super) gate: Option<(Expr, bool)>,
}

pub(super) fn allowed(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "GETKEY"
            | "GETKEYTRIGGERED"
            | "SEQUENCEINPUT"
            | "DISABLE_INPUT_MACRO"
            | "ENABLE_INPUT_MACRO"
            | "ENV_HAS_CAPABILITY"
            | "GETPLATFORM"
    )
}
fn invalid(message: &str) -> StepError {
    StepError::new(VmFaultCode::InvalidInstruction, message)
}
impl RuntimeFormContinuation {
    pub(crate) fn next_step_calls_input_host(&self) -> bool {
        self.awaiting_user_call.is_none()
            && matches!(
                self.work.last(),
                Some(
                    RuntimeFormTask::GateInputHost { .. } | RuntimeFormTask::FinishInputHost { .. }
                )
            )
    }
    pub(super) fn schedule_input_host(
        &mut self,
        vm: &Vm,
        name: &str,
        arguments: &[Option<Expr>],
    ) -> Result<bool, StepError> {
        let snake_input = vm.generations.get(&self.generation).is_some_and(|program| {
            program
                .artifact
                .manifest
                .compatibility
                .supports_snake_input()
        });
        if !snake_input || !allowed(name) {
            return Ok(false);
        }
        if matches!(
            name.to_ascii_uppercase().as_str(),
            "GETKEY" | "GETKEYTRIGGERED"
        ) {
            let [Some(key)] = arguments else {
                return Err(invalid("GETKEY requires one key expression"));
            };
            self.work.push(RuntimeFormTask::GateInputHost {
                plan: self
                    .current_call_plan
                    .ok_or_else(|| invalid("input query lacks its source plan"))?,
                key: key.clone(),
                triggered: name.eq_ignore_ascii_case("GETKEYTRIGGERED"),
            });
        } else {
            if arguments.iter().any(Option::is_none) {
                return Err(invalid(
                    "input API cannot contain an interior omitted argument",
                ));
            }
            self.work.push(RuntimeFormTask::FinishInputHost {
                name: name.to_ascii_uppercase(),
                count: arguments.len(),
            });
            self.work.extend(
                arguments
                    .iter()
                    .rev()
                    .flatten()
                    .cloned()
                    .map(RuntimeFormTask::Evaluate),
            );
        }
        Ok(true)
    }
    pub(super) fn call_input_host(
        &mut self,
        vm: &mut Vm,
        fiber: &mut Fiber,
        host: &mut impl VmHost,
        host_calls: &mut u32,
        invocation: InputHostInvocation<'_>,
    ) -> Result<(), StepError> {
        let InputHostInvocation {
            name,
            arguments,
            gate,
        } = invocation;
        let program = std::sync::Arc::clone(
            vm.generations
                .get(&self.generation)
                .ok_or_else(|| invalid("input form generation missing"))?,
        );
        if !program
            .artifact
            .manifest
            .compatibility
            .supports_snake_input()
        {
            return Err(support::permission_denied(
                "snake input form API is unavailable",
            ));
        }
        if !allowed(name) && name != "__GETKEY_ACTIVE" {
            return Err(invalid("form attempted an undeclared input Host operation"));
        }
        let target = program
            .artifact
            .host_imports
            .iter()
            .find(|target| {
                target.import.namespace == "rustyera.input"
                    && target.import.name.eq_ignore_ascii_case(name)
                    && target.import.result == Some(BytecodeType::Integer)
                    && target.import.parameters.len() == arguments.len()
                    && target
                        .import
                        .parameters
                        .iter()
                        .zip(&arguments)
                        .all(|(kind, value)| *kind == value.value_type())
            })
            .ok_or_else(|| invalid("input form Host signature is absent"))?
            .clone();
        let opening = program
            .function(self.function)
            .and_then(|function| function.code.get(self.instruction))
            .ok_or_else(|| invalid("input form root instruction missing"))?;
        let position = crate::interpreter::InstructionPosition {
            generation: self.generation,
            function: self.function,
            instruction: self.instruction,
            variable: None,
            encoded: crate::interpreter::DispatchInstruction {
                opcode: opening.opcode,
                payload: &opening.payload,
            },
        };
        let origin = vm.execution_origin(&position, name);
        let depth = owner_frame(fiber, self.frame)?.stack.len();
        self.work
            .push(RuntimeFormTask::ReadInputHost { depth, gate });
        vm.invalidate_path_memo(fiber.id);
        let request = vm.allocate_request_id();
        *host_calls = host_calls.saturating_add(1);
        match host.call(HostCallRequest {
            id: request,
            fiber: fiber.id,
            import: target.import.clone(),
            arguments,
            omitted_arguments: Vec::new(),
            origin: origin.clone(),
        }) {
            HostCallResult::Ready(ready) => vm
                .apply_host_ready(fiber, Some(BytecodeType::Integer), ready)
                .map_err(|error| {
                    StepError::classified(
                        crate::FaultCategory::HostContract,
                        VmFaultCode::Host,
                        error.to_string(),
                    )
                })?,
            HostCallResult::Deferred => {
                fiber.state = FiberState::WaitingHost(WaitingHost {
                    request,
                    import: target.import,
                    result: Some(BytecodeType::Integer),
                    stability: HostWaitStability::Transient,
                    rebind_payload: Vec::new(),
                    origin,
                    form_scope: None,
                });
            }
            HostCallResult::Pending { .. } => {
                return Err(StepError::classified(
                    crate::FaultCategory::HostContract,
                    VmFaultCode::Host,
                    "input controller query cannot create an external or stable Host wait",
                ));
            }
            HostCallResult::Error(error) => return Err(error),
        }
        Ok(())
    }
    pub(super) fn read_input_host(
        &mut self,
        fiber: &mut Fiber,
        depth: usize,
        gate: Option<(Expr, bool)>,
    ) -> Result<(), StepError> {
        let owner = owner_frame_mut(fiber, self.frame)?;
        if owner.stack.len() != depth + 1 {
            return Err(invalid("input Host result stack differs"));
        }
        let Some(VmValue::Integer(value)) = owner.stack.pop() else {
            return Err(invalid("input Host result is not Integer"));
        };
        if let Some((key, triggered)) = gate {
            if value == 0 {
                self.values.push(VmValue::Integer(0));
            } else if value == 1 {
                self.work.push(RuntimeFormTask::FinishInputHost {
                    name: if triggered {
                        "GETKEYTRIGGERED"
                    } else {
                        "GETKEY"
                    }
                    .into(),
                    count: 1,
                });
                self.work.push(RuntimeFormTask::Evaluate(key));
            } else {
                return Err(invalid("input active gate is not boolean"));
            }
        } else {
            self.values.push(VmValue::Integer(value));
        }
        Ok(())
    }
}
