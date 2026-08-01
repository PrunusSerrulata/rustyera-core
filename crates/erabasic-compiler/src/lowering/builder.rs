//! Stateful HIR-to-bytecode builder.

use super::{
    BTreeMap, CompilerDiagnostic, EncodedInstruction, Function, FunctionImport, HostImport, LineId,
    LoweringContext, NativeImport, SourceLocation, SymbolKey,
};

mod calls;
mod data_blocks;
mod expressions;
mod formatted;
mod imports;
mod statements;

pub(super) struct Builder<'a> {
    pub(super) hir_function: &'a Function,
    pub(super) context: &'a LoweringContext<'a>,
    pub(super) code: Vec<EncodedInstruction>,
    pub(super) locations: Vec<SourceLocation>,
    pub(super) imports: Vec<FunctionImport>,
    pub(super) native_imports: BTreeMap<SymbolKey, NativeImport>,
    pub(super) host_imports: BTreeMap<SymbolKey, HostImport>,
    control_flow_by_line: BTreeMap<LineId, Vec<&'a erabasic_hir::ControlFlowEdge>>,
    pub(super) diagnostics: Vec<CompilerDiagnostic>,
}

impl<'a> Builder<'a> {
    pub(super) fn new(
        hir_function: &'a Function,
        _function_key: SymbolKey,
        context: &'a LoweringContext<'a>,
    ) -> Self {
        let mut control_flow_by_line = BTreeMap::new();
        for edge in &hir_function.control_flow {
            control_flow_by_line
                .entry(edge.from)
                .or_insert_with(Vec::new)
                .push(edge);
        }
        Self {
            hir_function,
            context,
            code: Vec::new(),
            locations: Vec::new(),
            imports: Vec::new(),
            native_imports: BTreeMap::new(),
            host_imports: BTreeMap::new(),
            control_flow_by_line,
            diagnostics: Vec::new(),
        }
    }

    pub(super) fn emit(&mut self, instruction: EncodedInstruction, location: SourceLocation) {
        self.code.push(instruction);
        self.locations.push(location);
    }
}
