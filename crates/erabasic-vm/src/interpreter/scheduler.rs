#[allow(clippy::wildcard_imports)]
use super::*;

impl Vm {
    #[allow(clippy::too_many_lines)]
    pub fn run_slice(
        &mut self,
        host: &mut impl VmHost,
        natives: &mut NativeServiceRegistry,
        budget: RunBudget,
    ) -> VmRunReport {
        let mut report = VmRunReport {
            stop: VmRunStop::Idle,
            instructions: 0,
            host_calls: 0,
            events: Vec::new(),
        };
        if self.debug_is_paused() {
            return report;
        }
        if let Some(selected) = self.debug_step_fiber()
            && let Some(index) = self.runnable.iter().position(|fiber| *fiber == selected)
            && let Some(selected) = self.runnable.remove(index)
        {
            self.runnable.push_front(selected);
        }
        let base_quantum = budget.fiber_quantum.max(1);
        let mut budget_exhausted = false;
        let mut function_cursor = None;
        // Debug controls cannot be installed concurrently while this mutable VM
        // slice is running. Once active, keep checking until the slice ends so
        // resume-skip and step-plan transitions retain their existing behavior.
        let debug_checks_active = self.debug_checks_active();
        while let Some(fiber_id) = self.runnable.pop_front() {
            if self
                .debug_step_fiber()
                .is_some_and(|selected| selected != fiber_id)
            {
                self.runnable.push_front(fiber_id);
                break;
            }
            if report.instructions >= budget.maximum_instructions {
                self.runnable.push_front(fiber_id);
                budget_exhausted = true;
                break;
            }
            // Round-robin preemption matters only while another fiber is ready. Let a sole
            // runnable fiber consume the caller-visible slice directly: this removes repeated
            // queue churn and lets deterministic function traces span an otherwise artificial
            // quantum boundary without weakening the total instruction budget.
            let quantum = if self.runnable.is_empty() {
                u32::try_from(
                    budget
                        .maximum_instructions
                        .saturating_sub(report.instructions),
                )
                .unwrap_or(u32::MAX)
                .max(base_quantum)
            } else {
                base_quantum
            };
            let Some(mut fiber) = self.fibers.remove(&fiber_id) else {
                continue;
            };
            if !matches!(fiber.state, FiberState::Runnable) {
                self.fibers.insert(fiber_id, fiber);
                continue;
            }
            let mut used = 0u32;
            let mut yielded = false;
            while used < quantum && matches!(fiber.state, FiberState::Runnable) {
                if report.instructions >= budget.maximum_instructions {
                    budget_exhausted = true;
                    break;
                }
                let continuation_origin = fiber
                    .frames
                    .last()
                    .and_then(|frame| frame.runtime_form.as_ref())
                    .map(dynamic_form::RuntimeFormContinuation::origin);
                if continuation_origin.is_none()
                    && debug_checks_active
                    && let Some(stop) = self.debug_stop_before(&fiber)
                {
                    report.events.push(VmEvent::DebugStopped(stop));
                    break;
                }
                let position_result =
                    if let Some((generation, function, instruction)) = continuation_origin {
                        self.instruction_position_at(
                            generation,
                            function,
                            instruction,
                            &mut function_cursor,
                        )
                    } else {
                        self.instruction_position(&fiber, &mut function_cursor)
                    };
                let position = match position_result {
                    Ok(position) => position,
                    Err(error) => {
                        let fallback = fiber.frames.last().map_or(
                            InstructionPosition {
                                generation: self.current_generation,
                                function: SymbolKey::default(),
                                instruction: 0,
                                variable: None,
                                encoded: DispatchInstruction::trap(),
                            },
                            |frame| InstructionPosition {
                                generation: frame.generation,
                                function: frame.function,
                                instruction: frame.instruction,
                                variable: None,
                                encoded: DispatchInstruction::trap(),
                            },
                        );
                        let fault = self.make_fault(
                            fiber.id,
                            &fallback,
                            VmFaultCode::InvalidInstruction,
                            error.to_string(),
                        );
                        fiber.clear_runtime_forms();
                        fiber.state = FiberState::Faulted(fault.clone());
                        report.events.push(VmEvent::FiberFaulted {
                            fiber: fiber.id,
                            fault,
                        });
                        break;
                    }
                };
                if continuation_origin.is_none()
                    && position.encoded.opcode == Opcode::CallHost as u16
                    && report.host_calls >= budget.maximum_host_calls
                {
                    budget_exhausted = true;
                    break;
                }
                let host_before = report.host_calls;
                let policy = ExecutionPolicy {
                    allow_function_memo: !debug_checks_active,
                    // A queued diagnostic/debug event is an ordering barrier. Fall back to
                    // the ordinary Host port so the caller observes it before more runtime
                    // state is mutated.
                    allow_immediate_host: !debug_checks_active && report.events.is_empty(),
                    remaining_quantum: quantum.saturating_sub(used),
                    remaining_instructions: budget
                        .maximum_instructions
                        .saturating_sub(report.instructions),
                };
                let outcome = if continuation_origin.is_some() {
                    self.invalidate_path_memo(fiber.id);
                    resume_runtime_form(self, &mut fiber, natives).and_then(|step| match step {
                        RuntimeFormStep::Pending => Ok(StepOutcome::DeferredNative),
                        RuntimeFormStep::CompleteCall => Ok(StepOutcome::Continue),
                        RuntimeFormStep::Complete(value) => {
                            let frame = fiber.frames.last_mut().ok_or_else(|| {
                                StepError::new(
                                    VmFaultCode::InvalidInstruction,
                                    "STRFORM owner frame disappeared before completion",
                                )
                            })?;
                            frame.stack.push(value);
                            Ok(StepOutcome::Continue)
                        }
                    })
                } else {
                    if let Ok(opcode) = Opcode::try_from(position.encoded.opcode) {
                        self.observe_path_memo_opcode(fiber.id, opcode);
                    } else {
                        self.invalidate_path_memo(fiber.id);
                    }
                    self.execute_instruction(
                        &mut fiber,
                        &position,
                        host,
                        natives,
                        &mut report.host_calls,
                        policy,
                    )
                };
                self.drain_compatibility_diagnostics(fiber.id, &position, &mut report.events);
                let additional_instructions = match &outcome {
                    Ok(StepOutcome::BulkProgress(instructions)) => *instructions,
                    _ => 0,
                };
                self.observe_path_memo_instruction(
                    fiber.id,
                    1_u64.saturating_add(additional_instructions),
                );
                report.instructions = report
                    .instructions
                    .saturating_add(1)
                    .saturating_add(additional_instructions);
                used = used
                    .saturating_add(1)
                    .saturating_add(u32::try_from(additional_instructions).unwrap_or(u32::MAX));
                if report.host_calls != host_before {
                    fiber.mark_progress();
                }
                match outcome {
                    Ok(StepOutcome::Continue | StepOutcome::BulkProgress(_)) => {
                        if debug_checks_active
                            && let Some(stop) = self.debug_stop_after(&fiber, false, false)
                        {
                            report.events.push(VmEvent::DebugStopped(stop));
                            break;
                        }
                    }
                    Ok(StepOutcome::Diagnostic {
                        code,
                        message,
                        notification,
                    }) => {
                        self.invalidate_path_memo(fiber.id);
                        let command = self.command_for_position(&position);
                        report.events.push(VmEvent::Diagnostic {
                            fiber: fiber.id,
                            code: code.into(),
                            message: message.into(),
                            origin: self.execution_origin(&position, &command),
                            notification,
                        });
                        if debug_checks_active
                            && let Some(stop) = self.debug_stop_after(&fiber, false, false)
                        {
                            report.events.push(VmEvent::DebugStopped(stop));
                            break;
                        }
                    }
                    Ok(StepOutcome::DeferredNative) => self.invalidate_path_memo(fiber.id),
                    Ok(StepOutcome::Yielded) => {
                        self.invalidate_path_memo(fiber.id);
                        fiber.mark_progress();
                        yielded = true;
                        report
                            .events
                            .push(VmEvent::FiberYielded { fiber: fiber.id });
                        if debug_checks_active
                            && let Some(stop) = self.debug_stop_after(&fiber, false, false)
                        {
                            report.events.push(VmEvent::DebugStopped(stop));
                        }
                        break;
                    }
                    Ok(StepOutcome::Blocked) => {
                        self.invalidate_path_memo(fiber.id);
                        fiber.mark_progress();
                        if let FiberState::WaitingHost(wait) = &fiber.state {
                            report.events.push(VmEvent::HostPending {
                                fiber: fiber.id,
                                request: wait.request,
                            });
                        }
                        if debug_checks_active
                            && let Some(stop) = self.debug_stop_after(&fiber, true, false)
                        {
                            report.events.push(VmEvent::DebugStopped(stop));
                        }
                        break;
                    }
                    Ok(StepOutcome::Completed(value)) => {
                        report.events.push(VmEvent::FiberCompleted {
                            fiber: fiber.id,
                            value,
                        });
                        if debug_checks_active
                            && let Some(stop) = self.debug_stop_after(&fiber, false, true)
                        {
                            report.events.push(VmEvent::DebugStopped(stop));
                        }
                        break;
                    }
                    Err(error) => {
                        self.abort_path_memo(fiber.id);
                        fiber.clear_runtime_forms();
                        let fault = self.make_classified_fault(fiber.id, &position, error);
                        fiber.state = FiberState::Faulted(fault.clone());
                        report.events.push(VmEvent::FiberFaulted {
                            fiber: fiber.id,
                            fault,
                        });
                        break;
                    }
                }
                if fiber.backward_branches_without_progress
                    > self.config.maximum_backward_branches_without_progress
                {
                    let fault = self.make_fault(
                        fiber.id,
                        &position,
                        VmFaultCode::RunawayExecution,
                        "backward-branch watchdog detected execution without host progress",
                    );
                    fiber.clear_runtime_forms();
                    fiber.state = FiberState::Faulted(fault.clone());
                    report.events.push(VmEvent::FiberFaulted {
                        fiber: fiber.id,
                        fault,
                    });
                    break;
                }
            }

            if matches!(fiber.state, FiberState::Runnable) {
                if used >= quantum || budget_exhausted {
                    self.invalidate_path_memo(fiber.id);
                }
                // A fiber quantum is scheduler preemption, not evidence that the caller's
                // instruction budget was exhausted. Large finite EraBasic routines can span
                // many quanta in one run slice (for example, the eraTW all-items scan). Count
                // only slices that actually consume the caller-visible budget so such work is
                // not mistaken for persistent runaway execution.
                if report.instructions >= budget.maximum_instructions && !yielded {
                    fiber.consecutive_budget_exhaustions =
                        fiber.consecutive_budget_exhaustions.saturating_add(1);
                    if fiber.consecutive_budget_exhaustions
                        > self.config.maximum_consecutive_budget_exhaustions
                    {
                        let position = self
                            .instruction_position(&fiber, &mut function_cursor)
                            .unwrap_or(InstructionPosition {
                                generation: self.current_generation,
                                function: SymbolKey::default(),
                                instruction: 0,
                                variable: None,
                                encoded: DispatchInstruction::trap(),
                            });
                        let fault = self.make_fault(
                            fiber.id,
                            &position,
                            VmFaultCode::RunawayExecution,
                            "instruction-budget watchdog detected persistent execution without progress",
                        );
                        fiber.clear_runtime_forms();
                        fiber.state = FiberState::Faulted(fault.clone());
                        report.events.push(VmEvent::FiberFaulted {
                            fiber: fiber.id,
                            fault,
                        });
                    }
                }
                if matches!(fiber.state, FiberState::Runnable) {
                    self.runnable.push_back(fiber_id);
                }
            }
            if matches!(fiber.state, FiberState::Faulted(_) | FiberState::Cancelled) {
                self.abort_path_memo(fiber.id);
                for frame in &fiber.frames {
                    self.active_function_memos.remove(&frame.id);
                }
            }
            self.fibers.insert(fiber_id, fiber);
            if budget_exhausted || self.debug_is_paused() {
                break;
            }
        }
        self.reclaim_generations();
        report.stop = if budget_exhausted || !self.runnable.is_empty() {
            VmRunStop::BudgetExhausted
        } else {
            VmRunStop::Idle
        };
        report
    }
}
