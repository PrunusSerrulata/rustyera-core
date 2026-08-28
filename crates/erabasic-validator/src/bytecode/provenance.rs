//! Non-serialized summaries of the validator's existing CFG stack analysis.
use super::instructions::StackValue;
use erabasic_bytecode::SymbolKey;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ValidatedStackToken {
    UserCall {
        stack_index: usize,
        resolve: u32,
        next_slot: u16,
    },
    ExistVarProbe {
        stack_index: usize,
        begin: u32,
    },
}

/// Ordinary operands need no duplicated type storage for lease validation.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ValidatedStackState {
    pub operand_count: usize,
    pub tokens: Vec<ValidatedStackToken>,
}

impl ValidatedStackState {
    pub(super) fn from_stack(stack: &[StackValue]) -> Self {
        let tokens = stack
            .iter()
            .enumerate()
            .filter_map(|(stack_index, value)| match value {
                StackValue::Value(_) => None,
                StackValue::UserCallToken { resolve, next_slot } => {
                    Some(ValidatedStackToken::UserCall {
                        stack_index,
                        resolve: *resolve,
                        next_slot: *next_slot,
                    })
                }
                StackValue::ExistVarProbeToken { begin } => {
                    Some(ValidatedStackToken::ExistVarProbe {
                        stack_index,
                        begin: *begin,
                    })
                }
            })
            .collect();
        Self {
            operand_count: stack.len(),
            tokens,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct FunctionStackProvenance {
    // Intern equal shapes: most bytecode IPs have one of a few scalar-only stacks.
    // Retain every reachable IP without a Vec header and duplicate token list per IP.
    before: Vec<usize>,
    states: Vec<ValidatedStackState>,
    terminal_user_calls: BTreeMap<usize, ValidatedStackState>,
}

impl FunctionStackProvenance {
    pub(super) fn new(
        states: Vec<Option<Vec<StackValue>>>,
        terminal_user_calls: BTreeMap<usize, ValidatedStackState>,
    ) -> Self {
        let mut indices = BTreeMap::new();
        let mut unique = Vec::new();
        let before = states
            .into_iter()
            .map(|stack| {
                let Some(stack) = stack else {
                    return usize::MAX;
                };
                let state = ValidatedStackState::from_stack(&stack);
                if let Some(index) = indices.get(&state) {
                    return *index;
                }
                let index = unique.len();
                unique.push(state.clone());
                indices.insert(state, index);
                index
            })
            .collect();
        Self {
            before,
            states: unique,
            terminal_user_calls,
        }
    }
}

/// Only successful bytecode validation can construct this provenance container.
/// Keys are stable function identities, so manifest canonicalization cannot detach it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedOperandStacks(BTreeMap<SymbolKey, FunctionStackProvenance>);

impl ValidatedOperandStacks {
    pub(super) fn new(functions: BTreeMap<SymbolKey, FunctionStackProvenance>) -> Self {
        Self(functions)
    }

    /// None means an unknown function, an unreachable IP, or an out-of-range IP.
    #[must_use]
    pub fn before(&self, function: SymbolKey, instruction: usize) -> Option<&ValidatedStackState> {
        let function = self.0.get(&function)?;
        function.states.get(*function.before.get(instruction)?)
    }

    /// A JUMP caller remains suspended after an `InvokeUserCall` with no CFG successor.
    #[must_use]
    pub fn terminal_user_call(
        &self,
        function: SymbolKey,
        invoke: usize,
    ) -> Option<&ValidatedStackState> {
        self.0.get(&function)?.terminal_user_calls.get(&invoke)
    }
}
