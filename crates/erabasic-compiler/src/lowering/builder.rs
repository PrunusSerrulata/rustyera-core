//! Stateful HIR-to-bytecode builder.

use std::collections::HashMap;

use super::{
    BytecodeType, CompilerDiagnostic, DenseIdIndex, EncodedInstruction, Function, FunctionImport,
    HostImport, LineId, LoweringContext, NativeImport, Opcode, SourceLocation, SymbolKey, opcode,
};

mod bit_arrays;
mod calls;
mod column_options;
mod data_blocks;
mod existvar;
mod expressions;
mod formatted;
mod html_queries;
mod imports;
mod map_calls;
mod matching;
mod methods;
mod statements;

pub(super) struct Builder<'a> {
    pub(super) hir_function: &'a Function,
    pub(super) context: &'a LoweringContext<'a>,
    pub(super) code: Vec<EncodedInstruction>,
    pub(super) locations: Vec<SourceLocation>,
    pub(super) imports: Vec<FunctionImport>,
    pub(super) import_indices: HashMap<(u8, SymbolKey), u32>,
    pub(super) native_imports: Vec<NativeImport>,
    pub(super) host_imports: Vec<HostImport>,
    pub(super) argument_types: Vec<BytecodeType>,
    control_flow_by_line: DenseIdIndex<Vec<&'a erabasic_hir::ControlFlowEdge>>,
    pub(super) diagnostics: Vec<CompilerDiagnostic>,
}

impl<'a> Builder<'a> {
    pub(super) fn new(
        hir_function: &'a Function,
        _function_key: SymbolKey,
        context: &'a LoweringContext<'a>,
    ) -> Self {
        let mut control_flow_by_line = DenseIdIndex::new(hir_function.lines.len());
        for edge in &hir_function.control_flow {
            control_flow_by_line
                .get_or_insert_with(edge.from.0, Vec::new)
                .expect("validated control-flow line IDs are in range")
                .push(edge);
        }
        let minimum_instructions = hir_function.lines.len().saturating_add(1);
        Self {
            hir_function,
            context,
            code: Vec::with_capacity(minimum_instructions),
            locations: Vec::with_capacity(minimum_instructions),
            imports: Vec::new(),
            import_indices: HashMap::with_capacity(8),
            native_imports: Vec::new(),
            host_imports: Vec::new(),
            argument_types: Vec::new(),
            control_flow_by_line,
            diagnostics: Vec::new(),
        }
    }

    pub(super) fn emit(&mut self, instruction: EncodedInstruction, location: SourceLocation) {
        self.code.push(instruction);
        self.locations.push(location);
    }

    pub(super) fn patch_jump(&mut self, instruction: usize, target: usize) {
        self.code[instruction].payload =
            opcode::jump(Opcode::Jump, u32::try_from(target).unwrap_or(u32::MAX)).payload;
    }

    pub(super) fn take_control_flow(
        &mut self,
        line: LineId,
    ) -> Vec<&'a erabasic_hir::ControlFlowEdge> {
        self.control_flow_by_line.take(line.0).unwrap_or_default()
    }
}
