//! MATCH uses the existing form work stack and the same VM token scanner as bytecode.
use super::super::matching::MatchState;
use super::call_plan::{RuntimeBoundCall, RuntimeCallSite};
use super::{
    BytecodeType, Deserialize, Expr, ExprKind, Fiber, RuntimeFormContinuation, RuntimeFormTask,
    Serialize, StepError, SymbolKey, Vm, VmFaultCode, VmValue, support, unsupported,
};
use erabasic_bytecode::{MatchCallSpec, MatchInput};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct FormMatch {
    pub site: RuntimeCallSite,
    pub arguments: Vec<Option<Expr>>,
    pub spec: MatchCallSpec,
    pub state: MatchState,
    pub value_depth: usize,
}
fn source_atom(mut expression: &Expr) -> &Expr {
    while let ExprKind::Group(inner) = &expression.kind {
        expression = inner;
    }
    expression
}
fn variable(expression: &Expr) -> Option<&str> {
    match &source_atom(expression).kind {
        ExprKind::Identifier(name) | ExprKind::Variable { name, .. } => Some(name),
        _ => None,
    }
}
pub(super) fn is_match(name: &str) -> bool {
    name.eq_ignore_ascii_case("MATCHALL") || name.eq_ignore_ascii_case("MATCHALLEX")
}

/// Consume types already derived by the caller; never recursively rewalk args here.
pub(super) fn match_spec(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    name: &str,
    arguments: &[Option<Expr>],
    actual_types: &[Option<BytecodeType>],
) -> Result<MatchCallSpec, StepError> {
    if !program
        .artifact
        .manifest
        .compatibility
        .supports_snake_data_apis()
    {
        return Err(support::permission_denied(
            "MATCH is unavailable in this identity",
        ));
    }
    if !is_match(name)
        || !(2..=5).contains(&arguments.len())
        || actual_types.len() != arguments.len()
    {
        return Err(unsupported("MATCH requires two to five arguments"));
    }
    let first = arguments[0]
        .as_ref()
        .ok_or_else(|| unsupported("MATCH input omitted"))?;
    let key = |expression: &Expr| {
        let name =
            variable(expression).ok_or_else(|| unsupported("MATCH requires a variable token"))?;
        program
            .scoped_variable(function, name)
            .map(|value| value.key)
            .ok_or_else(|| unsupported("MATCH variable token does not exist"))
    };
    let input = if name.eq_ignore_ascii_case("MATCHALLEX") {
        let ExprKind::String(value) = &source_atom(first).kind else {
            return Err(unsupported("MATCHALLEX requires a source string literal"));
        };
        MatchInput::Name(value.clone())
    } else {
        MatchInput::Variable(key(first)?)
    };
    let scalar = |index: usize| -> Result<Option<BytecodeType>, StepError> {
        let kind = actual_types.get(index).copied().flatten();
        if kind.is_some_and(|kind| !matches!(kind, BytecodeType::Integer | BytecodeType::String)) {
            return Err(unsupported("MATCH value is not scalar"));
        }
        Ok(kind)
    };
    let output = arguments
        .get(4)
        .map(|value| {
            value
                .as_ref()
                .ok_or_else(|| unsupported("MATCH output omitted"))
                .and_then(key)
        })
        .transpose()?;
    Ok(MatchCallSpec {
        input,
        input_restructured_to_scalar: false,
        output,
        needle: scalar(1)?.ok_or_else(|| unsupported("MATCH needle omitted"))?,
        begin_type: scalar(2)?.unwrap_or(BytecodeType::Integer),
        end_type: scalar(3)?,
    })
}

impl RuntimeFormContinuation {
    pub(super) fn schedule_match(
        &mut self,
        vm: &Vm,
        fiber: &Fiber,
        spec: MatchCallSpec,
        arguments: Vec<Option<Expr>>,
        site: RuntimeCallSite,
    ) -> Result<(), StepError> {
        let program = vm.generations.get(&self.generation).ok_or_else(|| {
            StepError::new(VmFaultCode::MissingSymbol, "MATCH generation missing")
        })?;
        if !self.valid_match_binding(program, &spec, site, &arguments) {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "MATCH plan binding differs",
            ));
        }
        let state =
            MatchState::capture(vm, fiber, self.generation, self.frame, self.function, &spec)?;
        let begin = arguments.get(2).cloned().flatten();
        let call = FormMatch {
            site,
            arguments,
            spec,
            state,
            value_depth: self.values.len(),
        };
        self.work.push(RuntimeFormTask::MatchBegin(call));
        if let Some(begin) = begin {
            self.work.push(RuntimeFormTask::Evaluate(begin));
        } else {
            self.values.push(VmValue::Integer(0));
        }
        Ok(())
    }
    pub(super) fn match_begin(
        &mut self,
        vm: &Vm,
        fiber: &Fiber,
        mut call: FormMatch,
    ) -> Result<(), StepError> {
        if self.values.len() != call.value_depth + 1 {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "MATCH begin value depth differs",
            ));
        }
        let value = self.pop_value("MATCH begin missing")?;
        call.state.set_begin(vm, fiber, &value)?;
        let end = call.arguments.get(3).cloned().flatten();
        self.work.push(RuntimeFormTask::MatchEnd(call));
        if let Some(end) = end {
            self.work.push(RuntimeFormTask::Evaluate(end));
        }
        Ok(())
    }
    pub(super) fn match_end(&mut self, mut call: FormMatch) -> Result<(), StepError> {
        let has_end = call.arguments.get(3).is_some_and(Option::is_some);
        if self.values.len() != call.value_depth + usize::from(has_end) {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "MATCH end value depth differs",
            ));
        }
        let value = if has_end {
            Some(self.pop_value("MATCH end missing")?)
        } else {
            None
        };
        call.state.set_end(value.as_ref())?;
        let needle = call.arguments[1].clone().ok_or_else(|| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                "MATCH needle expression missing",
            )
        })?;
        self.work.push(RuntimeFormTask::MatchNeedle(call));
        self.work.push(RuntimeFormTask::Evaluate(needle));
        Ok(())
    }
    pub(super) fn match_needle(&mut self, mut call: FormMatch) -> Result<(), StepError> {
        if self.values.len() != call.value_depth + 1 {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "MATCH needle value depth differs",
            ));
        }
        call.state
            .set_needle(self.pop_value("MATCH needle missing")?)?;
        self.work.push(RuntimeFormTask::MatchScan(call));
        Ok(())
    }
    pub(super) fn match_scan(
        &mut self,
        vm: &mut Vm,
        fiber: &mut Fiber,
        mut call: FormMatch,
    ) -> Result<(), StepError> {
        if self.values.len() != call.value_depth {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "MATCH scan value depth differs",
            ));
        }
        // One read/write transition per work step; normal form scheduling counts each against budgets.
        let (_, done) = call.state.scan(vm, fiber, 1)?;
        if done {
            self.values.push(VmValue::Integer(call.state.count));
        } else {
            self.work.push(RuntimeFormTask::MatchScan(call));
        }
        Ok(())
    }
    pub(crate) fn valid_match_tasks(&self, vm: &Vm, fiber: &Fiber) -> bool {
        let Some(program) = vm.generations.get(&self.generation) else {
            return false;
        };
        self.work.iter().enumerate().all(|(index, task)| {
            let (call, phase, needle) = match task {
                RuntimeFormTask::MatchBegin(call) => (call, 0, false),
                RuntimeFormTask::MatchEnd(call) => (call, 1, false),
                RuntimeFormTask::MatchNeedle(call) => (call, 2, false),
                RuntimeFormTask::MatchScan(call) => (call, 2, true),
                _ => return true,
            };
            if !self.valid_match_binding(program, &call.spec, call.site, &call.arguments) {
                return false;
            }
            let spec = &call.spec;
            let Ok(initial) =
                MatchState::capture(vm, fiber, self.generation, self.frame, self.function, spec)
            else {
                return false;
            };
            call.value_depth <= self.values.len()
                && call.state.valid(vm, fiber)
                && call.state.phase() == phase
                && call.state.needle.is_some() == needle
                && call.state.input == initial.input
                && call.state.output == initial.output
                && call.state.needle_type == spec.needle
                && (!needle
                    || (index + 1 == self.work.len()
                        && self.values.len() == call.value_depth
                        && self.awaiting_user_call.is_none()))
        })
    }
    fn valid_match_binding(
        &self,
        program: &crate::ProgramGeneration,
        spec: &MatchCallSpec,
        site: RuntimeCallSite,
        arguments: &[Option<Expr>],
    ) -> bool {
        self.lookup_bound_call(site) == Some(&RuntimeBoundCall::Match(spec.clone()))
            && self.validate_call_arguments(program, site, arguments)
    }
}
