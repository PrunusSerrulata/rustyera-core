//! Runtime-owned handling for the debugger's intentionally restricted console.
//!
//! Parsing and pure-expression evaluation remain in the sibling `console`
//! module. This module owns the stateful VM read/write and response sequence.

use era_debug_protocol::{
    ConsoleCommand, ConsoleOutcome, DebugDiagnostic, DebugMessage, DebugResponse, DebugValue,
    StopToken, VariableValue,
};
use erabasic_vm::{VmDebugControl, VmDebugInspect, VmDebugVariableWrite};

use super::{
    RuntimeError, RuntimeSession, console_diagnostic, parse_console_expression, protocol_value,
    protocol_variable_value,
};

impl RuntimeSession {
    pub(super) fn debug_console(
        &mut self,
        message_id: u64,
        stop: StopToken,
        command: ConsoleCommand,
    ) -> Result<(), RuntimeError> {
        let vm_stop = self.validate_stop(stop, message_id)?;
        let (source, execute) = match command {
            ConsoleCommand::Evaluate { source } => (source, false),
            ConsoleCommand::ExecuteSafe { source } => (source, true),
        };
        let trimmed = source.trim();
        let variables = match self.debug_vm(message_id)?.variables(vm_stop, None, 1024) {
            Ok(page) => page.values,
            Err(error) => return self.emit_vm_debug_error(error, Some(message_id)),
        };
        let mut value = None;
        let mut changed_variables = Vec::new();
        let mut diagnostics = Vec::new();
        if execute {
            let Some((target_name, expression)) = trimmed.split_once('=') else {
                diagnostics.push(console_diagnostic(
                    "debug.console.unsafe_statement",
                    "only a single EraBasic assignment is accepted by the safe console",
                ));
                return self.emit_console_outcome(
                    message_id,
                    stop,
                    value,
                    changed_variables,
                    diagnostics,
                );
            };
            let target_name = target_name.trim();
            let Some(target) = variables
                .iter()
                .find(|item| item.name.eq_ignore_ascii_case(target_name))
            else {
                diagnostics.push(console_diagnostic(
                    "debug.console.unknown_variable",
                    "assignment target is not a visible scalar variable",
                ));
                return self.emit_console_outcome(
                    message_id,
                    stop,
                    value,
                    changed_variables,
                    diagnostics,
                );
            };
            let parsed = match parse_console_expression(expression.trim(), &variables) {
                Ok(value) => value,
                Err((code, message)) => {
                    diagnostics.push(console_diagnostic(code, &message));
                    return self.emit_console_outcome(
                        message_id,
                        stop,
                        value,
                        changed_variables,
                        diagnostics,
                    );
                }
            };
            let writes = [VmDebugVariableWrite {
                target: target.target.clone(),
                value: parsed,
                expected_revision: target.revision,
            }];
            match self
                .debug_vm_mut(message_id)?
                .write_variables(vm_stop, &writes)
            {
                Ok(values) => {
                    self.revision = self.revision.saturating_add(1);
                    changed_variables = values.into_iter().map(protocol_variable_value).collect();
                }
                Err(error) => return self.emit_vm_debug_error(error, Some(message_id)),
            }
        } else {
            match parse_console_expression(trimmed, &variables) {
                Ok(parsed) => value = Some(protocol_value(parsed)),
                Err((code, message)) => diagnostics.push(console_diagnostic(code, &message)),
            }
        }
        self.emit_console_outcome(message_id, stop, value, changed_variables, diagnostics)
    }

    fn emit_console_outcome(
        &mut self,
        message_id: u64,
        stop: StopToken,
        value: Option<DebugValue>,
        changed_variables: Vec<VariableValue>,
        diagnostics: Vec<DebugDiagnostic>,
    ) -> Result<(), RuntimeError> {
        let stop = self.refreshed_stop(stop);
        self.emit_debug(
            DebugMessage::Response(DebugResponse::Console(ConsoleOutcome {
                stop,
                value,
                output: Vec::new(),
                changed_variables,
                changed_game_fields: Vec::new(),
                diagnostics,
            })),
            Some(message_id),
        )
    }
}
