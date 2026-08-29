use erabasic_ast::{AssignOp, BinaryOp, UnaryOp};
use erabasic_bytecode::{
    BytecodeFunction, BytecodeFunctionKind, BytecodeLabel, BytecodeParameter, BytecodeType, Digest,
    EncodedInstruction, FunctionImport, HostImport, ImportKind, NATIVE_ABI_VERSION, NativeImport,
    Opcode, RuntimeImport, SymbolKey, opcode,
};
use erabasic_hir::{
    CallTarget, ControlFlowKind, Function, FunctionKind, HirArgument, HirCallArgument, HirExpr,
    HirExprKind, HirFormPart, HirFormattedString, HirStatementKind, InstructionTarget, LineId,
    SemanticType, SourceLocation, Variable,
};

use crate::{
    CompilerDiagnostic, CompilerDiagnosticCode, ExecutionBinding, HostRegistry,
    compile::DenseIdIndex, registry::extension_binding,
};

mod builder;
mod encoding;
mod planning;

use builder::Builder;
use encoding::{
    assign_tag, binary_tag, compiler_native_contract, compiler_variable_mutation_contract,
    unary_tag,
};
use planning::{
    DataBlock, DataLine, TryListBlock, TryListLine, add_control_flow, argument_place,
    collect_data_blocks, collect_try_lists, formatted_constant, statement_fingerprint,
    structured_if_flow,
};

pub(crate) use encoding::bytecode_type;

pub(crate) struct LoweringContext<'a> {
    pub program: LoweringProgram<'a>,
    pub function_keys: &'a DenseIdIndex<SymbolKey>,
    pub functions_by_id: &'a DenseIdIndex<&'a FunctionSignature>,
    pub variable_keys: &'a DenseIdIndex<SymbolKey>,
    pub source_indices: &'a DenseIdIndex<u32>,
    pub host_registry: &'a HostRegistry,
}

pub(crate) struct LoweringProgram<'a> {
    pub variables: &'a [Variable],
    pub snake_input: bool,
    pub call_compatibility: erabasic_hir::CallCompatibility,
}

pub(crate) struct FunctionSignature {
    pub id: erabasic_hir::FunctionId,
    pub name: String,
    pub kind: FunctionKind,
    pub return_type: SemanticType,
    pub parameters: Vec<erabasic_hir::Parameter>,
}

impl From<&Function> for FunctionSignature {
    fn from(function: &Function) -> Self {
        Self {
            id: function.id,
            name: function.name.clone(),
            kind: function.kind,
            return_type: function.return_type,
            parameters: function.parameters.clone(),
        }
    }
}

pub(crate) struct LoweredFunction {
    pub cache_key: Digest,
    pub function: BytecodeFunction,
    pub source_entries: Vec<LoweredSourceMapEntry>,
    pub native_imports: Vec<NativeImport>,
    pub host_imports: Vec<HostImport>,
    pub diagnostics: Vec<CompilerDiagnostic>,
}

struct LineOffsets {
    start: usize,
    entry: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct LoweredSourceMapEntry {
    pub function: SymbolKey,
    pub code_start: u64,
    pub code_end: u64,
    pub source_index: u32,
    pub byte_start: u64,
    pub byte_end: u64,
    pub statement_fingerprint: Digest,
    // Match the compact serialized record; macro origins are rare, while every
    // ordinary statement benefits from the thin optional pointer.
    #[allow(clippy::box_collection)]
    pub origin_chain: Option<Box<Vec<(u32, u64, u64)>>>,
}

#[must_use]
#[allow(clippy::too_many_lines)]
pub(crate) fn lower_function(
    function: &Function,
    key: SymbolKey,
    cache_key: Digest,
    context: &LoweringContext<'_>,
) -> LoweredFunction {
    let mut builder = Builder::new(function, key, context);
    let structured = structured_if_flow(function);
    let data_lines = collect_data_blocks(function);
    let try_list_lines = collect_try_lists(function);
    let mut loop_closers = DenseIdIndex::new(function.lines.len());
    for edge in function
        .control_flow
        .iter()
        .filter(|edge| edge.kind == ControlFlowKind::LoopBack)
    {
        if let Some(opener) = edge.to {
            loop_closers.insert(opener.0, edge.from);
        }
    }
    let mut line_offsets = DenseIdIndex::new(function.lines.len());
    let mut pending_jumps = Vec::with_capacity(function.control_flow.len());
    let mut pending_function_end_jumps = Vec::new();
    for line in &function.lines {
        let start = builder.code.len();
        line_offsets.insert(
            line.id.0,
            LineOffsets {
                start,
                entry: start,
            },
        );
        if let Some(end) = structured.alternative_end(line.id) {
            let instruction = builder.code.len();
            builder.emit(opcode::jump(Opcode::Jump, 0), line.location);
            pending_jumps.push((instruction, *end, false));
        }
        line_offsets
            .get_mut(line.id.0)
            .expect("validated line IDs have bytecode offsets")
            .entry = builder.code.len();
        let before = builder.code.len();
        if let Some(DataLine::Opener(block)) = data_lines.get(line.id.0) {
            builder.lower_data_block(block);
        } else if let Some(TryListLine::Opener(block)) = try_list_lines.get(line.id.0) {
            builder.lower_try_list(block);
        } else if matches!(data_lines.get(line.id.0), Some(DataLine::Body))
            || matches!(try_list_lines.get(line.id.0), Some(TryListLine::Body))
        {
            // The opener emits the complete selection so unselected DATA expressions
            // are never evaluated. Body lines retain a NOP for source-map anchoring.
        } else {
            match &line.kind {
                HirStatementKind::Assignment { target, op, value } => {
                    for index in &target.indices {
                        builder.lower_expression(index, line.location);
                    }
                    builder.lower_expression(value, line.location);
                    let Some(key) = context.variable_keys.get(target.variable.0).copied() else {
                        builder.diagnostics.push(CompilerDiagnostic::at(
                            CompilerDiagnosticCode::InvalidHir,
                            line.location,
                            "assignment variable has no stable symbol key",
                        ));
                        builder.emit(
                            EncodedInstruction::new(Opcode::Trap, b"missing variable".to_vec()),
                            line.location,
                        );
                        continue;
                    };
                    builder.emit(
                        opcode::variable(
                            Opcode::StoreVariable,
                            key,
                            u16::try_from(target.indices.len()).unwrap_or(u16::MAX),
                            assign_tag(*op),
                        ),
                        line.location,
                    );
                }
                HirStatementKind::Instruction { target, arguments } => {
                    builder.lower_statement(target, arguments, line.id, line.location);
                }
                HirStatementKind::Label { .. } => {
                    builder.emit(
                        EncodedInstruction::new(Opcode::Nop, Vec::new()),
                        line.location,
                    );
                }
                HirStatementKind::Error => {
                    builder.diagnostics.push(CompilerDiagnostic::at(
                        CompilerDiagnosticCode::InvalidHir,
                        line.location,
                        "error HIR statement cannot be lowered",
                    ));
                    builder.emit(
                        EncodedInstruction::new(Opcode::Trap, b"invalid HIR".to_vec()),
                        line.location,
                    );
                }
            }
        }
        if builder.code.len() == before {
            builder.emit(
                EncodedInstruction::new(Opcode::Nop, Vec::new()),
                line.location,
            );
        }
        if data_lines.get(line.id.0).is_none() && try_list_lines.get(line.id.0).is_none() {
            let outgoing = builder.take_control_flow(line.id);
            let outgoing = outgoing.as_slice();
            let structural_name = match &line.kind {
                HirStatementKind::Instruction { target, .. } => Some(target.name()),
                _ => None,
            };
            if matches!(structural_name, Some("DO" | "SELECTCASE")) {
                // DO is an unconditional body entry. Its analyzer branch marks
                // the matching LOOP boundary, not a runtime condition.
            } else if structural_name == Some("LOOP") {
                if let Some(target) = outgoing
                    .iter()
                    .find(|edge| edge.kind == ControlFlowKind::LoopBack)
                    .and_then(|edge| edge.to)
                {
                    let instruction = builder.code.len();
                    builder.emit(opcode::jump(Opcode::JumpIfFalse, 0), line.location);
                    pending_jumps.push((instruction, target, true));
                }
            } else if matches!(structural_name, Some("FOR" | "REPEAT" | "WHILE")) {
                let after_closer = outgoing
                    .iter()
                    .find(|edge| edge.kind == ControlFlowKind::Branch)
                    .and_then(|edge| edge.to)
                    .and_then(|closer| function.lines.get(closer.0 as usize + 1))
                    .map(|line| line.id);
                let instruction = builder.code.len();
                builder.emit(opcode::jump(Opcode::JumpIfFalse, 0), line.location);
                if let Some(target) = after_closer {
                    pending_jumps.push((instruction, target, true));
                } else {
                    pending_function_end_jumps.push(instruction);
                }
            } else if matches!(structural_name, Some("NEXT" | "REND")) {
                if let Some(body) = outgoing
                    .iter()
                    .find(|edge| edge.kind == ControlFlowKind::LoopBack)
                    .and_then(|edge| edge.to)
                    .and_then(|opener| function.lines.get(opener.0 as usize + 1))
                    .map(|line| line.id)
                {
                    let instruction = builder.code.len();
                    builder.emit(opcode::jump(Opcode::JumpIfFalse, 0), line.location);
                    pending_jumps.push((instruction, body, true));
                }
            } else if matches!(structural_name, Some("CONTINUE" | "BREAK")) {
                let loop_edge = outgoing.iter().find(|edge| {
                    matches!(
                        edge.kind,
                        ControlFlowKind::Continue | ControlFlowKind::Break
                    )
                });
                let opener = loop_edge.and_then(|edge| edge.to);
                let closer = opener.and_then(|opener| loop_closers.get(opener.0).copied());
                let opener_name = opener.and_then(|opener| {
                    function
                        .lines
                        .get(opener.0 as usize)
                        .and_then(|line| match &line.kind {
                            HirStatementKind::Instruction { target, .. } => Some(target.name()),
                            _ => None,
                        })
                });
                if structural_name == Some("BREAK") && matches!(opener_name, Some("FOR" | "REPEAT"))
                {
                    builder.emit(
                        EncodedInstruction::new(Opcode::ForBreak, Vec::new()),
                        line.location,
                    );
                }
                let target = if structural_name == Some("CONTINUE") {
                    if opener_name == Some("WHILE") {
                        opener
                    } else {
                        closer
                    }
                } else {
                    closer.and_then(|closer| {
                        function
                            .lines
                            .get(closer.0 as usize + 1)
                            .map(|line| line.id)
                    })
                };
                let instruction = builder.code.len();
                builder.emit(opcode::jump(Opcode::Jump, 0), line.location);
                if let Some(target) = target {
                    // BREAK resumes at the following logical line. If that line is
                    // an ELSE/ELSEIF/CASE boundary, its synthetic entry jump must
                    // run so a selected branch cannot fall into the next branch.
                    // CONTINUE instead targets the executable entry of its loop
                    // boundary and deliberately bypasses such prologue metadata.
                    pending_jumps.push((instruction, target, structural_name == Some("CONTINUE")));
                } else {
                    pending_function_end_jumps.push(instruction);
                }
            } else if outgoing
                .iter()
                .any(|edge| edge.kind == ControlFlowKind::Branch && edge.to.is_none())
            {
                let instruction = builder.code.len();
                builder.emit(opcode::jump(Opcode::JumpIfFalse, 0), line.location);
                pending_function_end_jumps.push(instruction);
            } else {
                add_control_flow(
                    line.id,
                    line.location,
                    &mut builder,
                    &structured,
                    outgoing,
                    &mut pending_jumps,
                );
            }
        }
    }
    let function_end = builder.code.len();
    if builder.code.is_empty()
        || !pending_function_end_jumps.is_empty()
        || !matches!(
            builder
                .code
                .last()
                .and_then(|instruction| Opcode::try_from(instruction.opcode).ok()),
            Some(Opcode::Return | Opcode::Trap | Opcode::Jump)
        )
    {
        builder.lower_era_fallthrough(function.location);
    }
    for (instruction, target, use_entry) in pending_jumps {
        let Some(target_index) = line_offsets.get(target.0).map(|offsets| {
            if use_entry {
                offsets.entry
            } else {
                offsets.start
            }
        }) else {
            builder.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::InvalidHir,
                function.location,
                "control-flow target has no bytecode location",
            ));
            continue;
        };
        builder.patch_jump(instruction, target_index);
    }
    for instruction in pending_function_end_jumps {
        builder.patch_jump(instruction, function_end);
    }

    // Source-map construction used to search and serialize the whole statement
    // list for every emitted opcode. Large dialogue functions then became O(n²).
    // Cache each exact statement location once; expression sub-spans retain the
    // same deterministic function-level fallback as before. Validated function
    // lines normally arrive in source order, so retain that contiguous layout for
    // cache-friendly binary search instead of allocating one tree node per line.
    let mut fingerprints = function
        .lines
        .iter()
        .map(|line| {
            (
                (
                    line.location.source.0,
                    line.location.span.start,
                    line.location.span.end,
                ),
                statement_fingerprint(&line.kind),
            )
        })
        .collect::<Vec<_>>();
    if fingerprints.windows(2).any(|pair| pair[0].0 > pair[1].0) {
        // Stable ordering preserves BTreeMap collection semantics for invalid HIR
        // that repeats a location: the last statement at that key wins.
        fingerprints.sort_by_key(|entry| entry.0);
    }
    let mut unique_len = 0;
    for index in 0..fingerprints.len() {
        let entry = fingerprints[index];
        if unique_len != 0 && fingerprints[unique_len - 1].0 == entry.0 {
            fingerprints[unique_len - 1] = entry;
        } else {
            fingerprints[unique_len] = entry;
            unique_len += 1;
        }
    }
    fingerprints.truncate(unique_len);
    let mut fallback_fingerprint = Digest::hash(
        "rustyera.bytecode.source-statement.v1",
        &[function.name.as_bytes()],
    );
    fallback_fingerprint.0[16..].fill(0);
    let mut offset = 0u64;
    // Adjacent opcodes from one HIR line normally coalesce into one source entry.
    // Reserving for lines is therefore a tighter estimate than reserving for every
    // opcode, and avoids a per-function shrink allocation after construction.
    let mut source_entries = Vec::<LoweredSourceMapEntry>::with_capacity(function.lines.len());
    for (instruction, location) in builder.code.iter().zip(&builder.locations) {
        let end = offset + instruction.encoded_len();
        if let Some(source_index) = context.source_indices.get(location.source.0) {
            let entry = LoweredSourceMapEntry {
                function: key,
                code_start: offset,
                code_end: end,
                source_index: *source_index,
                byte_start: location.span.start as u64,
                byte_end: location.span.end as u64,
                statement_fingerprint: fingerprints
                    .binary_search_by_key(
                        &(location.source.0, location.span.start, location.span.end),
                        |entry| entry.0,
                    )
                    .ok()
                    .map_or(fallback_fingerprint, |index| fingerprints[index].1),
                origin_chain: None,
            };
            // One statement commonly lowers to several adjacent opcodes. A source
            // range describes all bytes in the half-open code interval, so merging
            // identical adjacent origins preserves every lookup while avoiding one
            // 112-byte metadata record per opcode in large projects.
            append_source_entry(&mut source_entries, entry);
        }
        offset = end;
    }
    let parameters = function
        .parameters
        .iter()
        .filter_map(|parameter| {
            let variable = context
                .program
                .variables
                .get(parameter.target.variable.0 as usize)?;
            let value_type = if variable.reference {
                match parameter.target.value_type {
                    SemanticType::Integer => BytecodeType::IntegerPlace,
                    SemanticType::String => BytecodeType::StringPlace,
                    SemanticType::Void | SemanticType::Error => return None,
                }
            } else {
                bytecode_type(parameter.target.value_type)?
            };
            Some(BytecodeParameter {
                key: *context.variable_keys.get(parameter.target.variable.0)?,
                indices: parameter
                    .target
                    .indices
                    .iter()
                    .map(|index| match index.constant {
                        Some(erabasic_hir::ConstantValue::Integer(value)) => {
                            u64::try_from(value).ok()
                        }
                        Some(erabasic_hir::ConstantValue::String(_)) | None => None,
                    })
                    .collect::<Option<Vec<_>>>()?,
                value_type,
                by_reference: variable.reference,
                default: parameter.default.as_ref().and_then(|value| {
                    value.constant.as_ref().map(|value| match value {
                        erabasic_hir::ConstantValue::Integer(value) => {
                            erabasic_bytecode::BytecodeConstant::Integer(*value)
                        }
                        erabasic_hir::ConstantValue::String(value) => {
                            erabasic_bytecode::BytecodeConstant::String(value.clone())
                        }
                    })
                }),
            })
        })
        .collect();
    let result = bytecode_type(function.return_type);
    let labels = function
        .labels
        .iter()
        .filter_map(|(_, name, line)| {
            Some(BytecodeLabel {
                name: name.clone(),
                instruction: u32::try_from(line_offsets.get(line.0)?.start).ok()?,
            })
        })
        .collect();
    LoweredFunction {
        cache_key,
        function: BytecodeFunction {
            key,
            name: function.name.clone(),
            kind: match function.kind {
                FunctionKind::Normal => BytecodeFunctionKind::Normal,
                FunctionKind::Event => BytecodeFunctionKind::Event,
                FunctionKind::System => BytecodeFunctionKind::System,
                FunctionKind::Method => BytecodeFunctionKind::Method,
            },
            parameters,
            result,
            labels,
            imports: builder.imports,
            max_stack: u32::try_from(builder.code.len().saturating_mul(2).saturating_add(16))
                .unwrap_or(u32::MAX),
            code: builder.code,
        },
        source_entries,
        native_imports: builder.native_imports,
        host_imports: builder.host_imports,
        diagnostics: builder.diagnostics,
    }
}

fn append_source_entry(entries: &mut Vec<LoweredSourceMapEntry>, entry: LoweredSourceMapEntry) {
    if let Some(previous) = entries.last_mut()
        && previous.code_end == entry.code_start
        && previous.function == entry.function
        && previous.source_index == entry.source_index
        && previous.byte_start == entry.byte_start
        && previous.byte_end == entry.byte_end
        && previous.statement_fingerprint == entry.statement_fingerprint
        && previous.origin_chain == entry.origin_chain
    {
        previous.code_end = entry.code_end;
    } else {
        entries.push(entry);
    }
}

pub(crate) use encoding::runtime_import;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_identical_source_origins_share_one_code_range() {
        let function = SymbolKey::derive("compiler-source-map-test", b"function");
        let fingerprint = Digest::hash("compiler-source-map-test", &[b"statement"]);
        let entry = |code_start, code_end, byte_start| LoweredSourceMapEntry {
            function,
            code_start,
            code_end,
            source_index: 2,
            byte_start,
            byte_end: byte_start + 4,
            statement_fingerprint: fingerprint,
            origin_chain: None,
        };
        let mut entries = Vec::new();
        append_source_entry(&mut entries, entry(0, 6, 10));
        append_source_entry(&mut entries, entry(6, 14, 10));
        append_source_entry(&mut entries, entry(14, 20, 30));

        assert_eq!(entries.len(), 2);
        assert_eq!((entries[0].code_start, entries[0].code_end), (0, 14));
        assert_eq!((entries[1].code_start, entries[1].code_end), (14, 20));
    }
}
