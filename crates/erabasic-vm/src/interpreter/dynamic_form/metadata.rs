#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeFormContinuation {
    pub(crate) const fn origin(&self) -> (GenerationId, SymbolKey, usize) {
        (self.generation, self.function, self.instruction)
    }

    pub(crate) fn call_text_spec(&self) -> Option<erabasic_bytecode::CallTextSpec> {
        match self.completion {
            RuntimeFormRoot::Call { spec, .. } => Some(spec),
            RuntimeFormRoot::Value(_) => None,
        }
    }

    pub(crate) fn root_result_type(&self) -> Option<BytecodeType> {
        match self.completion {
            RuntimeFormRoot::Value(value_type) => Some(value_type),
            RuntimeFormRoot::Call { .. } => None,
        }
    }

    pub(crate) fn valid_for_frame(
        &self,
        generation: GenerationId,
        function: SymbolKey,
        frame: FrameId,
        maximum_stack: usize,
    ) -> bool {
        self.generation == generation
            && self.function == function
            && self.frame == frame
            && self.work.len() <= maximum_stack
            && self.values.len() <= maximum_stack
            && self.outputs.len() <= MAX_RUNTIME_FORM_NESTING
            && self.remaining_nodes <= maximum_stack
            && self.remaining_source_bytes <= MAX_RUNTIME_FORM_BYTES
            && self.host_scopes_valid()
            && self.checkpoints_valid()
            && self.host_resources().is_some_and(|(slots, bytes)| {
                slots <= maximum_stack && bytes <= MAX_RUNTIME_FORM_BYTES
            })
            && self.reference_arguments_valid()
            && self
                .reference_argument_resources()
                .is_some_and(|(slots, bytes)| {
                    slots <= maximum_stack && bytes <= MAX_RUNTIME_FORM_BYTES
                })
    }
}
