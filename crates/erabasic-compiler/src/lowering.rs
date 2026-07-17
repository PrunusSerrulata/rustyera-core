use std::collections::{BTreeMap, BTreeSet};

use erabasic_ast::{AssignOp, BinaryOp, PostfixOp, UnaryOp};
use erabasic_bytecode::{
    BytecodeFunction, BytecodeParameter, BytecodeType, Digest, EncodedInstruction, FunctionImport,
    HostImport, ImportKind, NATIVE_ABI_VERSION, NativeImport, Opcode, RuntimeImport,
    SourceMapEntry, SymbolKey, opcode,
};
use erabasic_hir::{
    CallTarget, ControlFlowKind, Function, FunctionId, HirArgument, HirCallArgument, HirExpr,
    HirExprKind, HirFormPart, HirFormattedString, HirStatementKind, InstructionTarget, LineId,
    Program, SemanticType, SourceLocation, VariableId,
};

use crate::{
    CompilerDiagnostic, CompilerDiagnosticCode, ExecutionBinding, HostRegistry,
    registry::extension_binding,
};

pub(crate) struct LoweringContext<'a> {
    pub program: &'a Program,
    pub function_keys: &'a BTreeMap<FunctionId, SymbolKey>,
    pub variable_keys: &'a BTreeMap<VariableId, SymbolKey>,
    pub source_indices: &'a BTreeMap<erabasic_hir::SourceId, u32>,
    pub host_registry: &'a HostRegistry,
}

pub(crate) struct LoweredFunction {
    pub cache_key: Digest,
    pub function: BytecodeFunction,
    pub source_entries: Vec<SourceMapEntry>,
    pub native_imports: Vec<NativeImport>,
    pub host_imports: Vec<HostImport>,
    pub diagnostics: Vec<CompilerDiagnostic>,
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
    let (data_blocks, data_body_lines) = collect_data_blocks(function);
    let mut line_starts = BTreeMap::new();
    let mut line_entries = BTreeMap::new();
    let mut pending_jumps = Vec::new();
    for line in &function.lines {
        line_starts.insert(line.id, builder.code.len());
        if let Some(end) = structured.alternative_ends.get(&line.id) {
            let instruction = builder.code.len();
            builder.emit(opcode::jump(Opcode::Jump, 0), line.location);
            pending_jumps.push((instruction, *end, false));
        }
        line_entries.insert(line.id, builder.code.len());
        let before = builder.code.len();
        if let Some(block) = data_blocks.get(&line.id) {
            builder.lower_data_block(block);
        } else if data_body_lines.contains(&line.id) {
            // The opener emits the complete selection so unselected DATA expressions
            // are never evaluated. Body lines retain a NOP for source-map anchoring.
        } else {
            match &line.kind {
                HirStatementKind::Assignment { target, op, value } => {
                    for index in &target.indices {
                        builder.lower_expression(index, line.location);
                    }
                    builder.lower_expression(value, line.location);
                    let Some(key) = context.variable_keys.get(&target.variable).copied() else {
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
        if !data_blocks.contains_key(&line.id) && !data_body_lines.contains(&line.id) {
            add_control_flow(
                function,
                line.id,
                line.location,
                &mut builder,
                &structured,
                &mut pending_jumps,
            );
        }
    }
    if builder.code.is_empty() {
        builder.emit(opcode::return_value(false), function.location);
    } else if !matches!(
        builder
            .code
            .last()
            .and_then(|instruction| Opcode::try_from(instruction.opcode).ok()),
        Some(Opcode::Return | Opcode::Trap | Opcode::Jump)
    ) {
        builder.emit(
            opcode::return_value(function.return_type != SemanticType::Void),
            function.location,
        );
    }
    for (instruction, target, use_entry) in pending_jumps {
        let locations = if use_entry {
            &line_entries
        } else {
            &line_starts
        };
        let Some(target_index) = locations.get(&target) else {
            builder.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::InvalidHir,
                function.location,
                "control-flow target has no bytecode location",
            ));
            continue;
        };
        builder.code[instruction].payload = u32::try_from(*target_index)
            .unwrap_or(u32::MAX)
            .to_le_bytes()
            .to_vec();
    }

    let mut offset = 0u64;
    let mut source_entries = Vec::with_capacity(builder.code.len());
    for (instruction, location) in builder.code.iter().zip(&builder.locations) {
        let end = offset + instruction.encoded_len();
        if let Some(source_index) = context.source_indices.get(&location.source) {
            source_entries.push(SourceMapEntry {
                function: key,
                code_start: offset,
                code_end: end,
                source_index: *source_index,
                byte_start: location.span.start as u64,
                byte_end: location.span.end as u64,
                statement_fingerprint: function
                    .lines
                    .iter()
                    .find(|line| line.location == *location)
                    .map_or_else(
                        || {
                            Digest::hash(
                                "rustyera.bytecode.source-statement.v1",
                                &[function.name.as_bytes()],
                            )
                        },
                        |line| statement_fingerprint(&line.kind),
                    ),
                origin_chain: Vec::new(),
            });
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
                key: *context.variable_keys.get(&parameter.target.variable)?,
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
    LoweredFunction {
        cache_key,
        function: BytecodeFunction {
            key,
            name: function.name.clone(),
            parameters,
            result,
            imports: builder.imports,
            max_stack: u32::try_from(builder.code.len().saturating_mul(2).saturating_add(16))
                .unwrap_or(u32::MAX),
            code: builder.code,
        },
        source_entries,
        native_imports: builder.native_imports.into_values().collect(),
        host_imports: builder.host_imports.into_values().collect(),
        diagnostics: builder.diagnostics,
    }
}

fn statement_fingerprint(kind: &HirStatementKind) -> Digest {
    let mut value = serde_json::to_value(kind).expect("typed statements are serializable");
    // Source locations are deliberately excluded: inserting unrelated lines must
    // not break a breakpoint anchor for an otherwise identical typed statement.
    strip_source_locations(&mut value);
    let bytes = serde_json::to_vec(&value).expect("normalized statements are serializable");
    Digest::hash("rustyera.bytecode.source-statement.v1", &[&bytes])
}

fn strip_source_locations(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            fields.remove("location");
            for value in fields.values_mut() {
                strip_source_locations(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                strip_source_locations(value);
            }
        }
        _ => {}
    }
}

struct DataBlock<'a> {
    opener: &'a erabasic_hir::HirStatement,
    choices: Vec<Vec<&'a erabasic_hir::HirStatement>>,
}

fn collect_data_blocks(function: &Function) -> (BTreeMap<LineId, DataBlock<'_>>, BTreeSet<LineId>) {
    let mut blocks = BTreeMap::new();
    let mut body = BTreeSet::new();
    let mut index = 0;
    while index < function.lines.len() {
        let line = &function.lines[index];
        let Some(name) = instruction_name(line) else {
            index += 1;
            continue;
        };
        if name != "STRDATA" && !name.starts_with("PRINTDATA") {
            index += 1;
            continue;
        }
        let mut choices = Vec::new();
        let mut cursor = index + 1;
        while cursor < function.lines.len() {
            let candidate = &function.lines[cursor];
            body.insert(candidate.id);
            match instruction_name(candidate) {
                Some("ENDDATA") => {
                    cursor += 1;
                    break;
                }
                Some("DATALIST") => {
                    let mut group = Vec::new();
                    cursor += 1;
                    while cursor < function.lines.len() {
                        let member = &function.lines[cursor];
                        body.insert(member.id);
                        if instruction_name(member) == Some("ENDLIST") {
                            break;
                        }
                        if matches!(instruction_name(member), Some("DATA" | "DATAFORM")) {
                            group.push(member);
                        }
                        cursor += 1;
                    }
                    choices.push(group);
                }
                Some("DATA" | "DATAFORM") => choices.push(vec![candidate]),
                _ => {}
            }
            cursor += 1;
        }
        blocks.insert(
            line.id,
            DataBlock {
                opener: line,
                choices,
            },
        );
        index = cursor;
    }
    (blocks, body)
}

fn instruction_name(line: &erabasic_hir::HirStatement) -> Option<&str> {
    match &line.kind {
        HirStatementKind::Instruction { target, .. } => Some(target.name()),
        _ => None,
    }
}

fn argument_place(argument: Option<&HirArgument>) -> Option<&erabasic_hir::HirPlace> {
    match argument? {
        HirArgument::Place(place)
        | HirArgument::Expression(HirExpr {
            kind: HirExprKind::Variable { place },
            ..
        }) => Some(place),
        HirArgument::Expression(_)
        | HirArgument::Formatted(_)
        | HirArgument::Raw(_)
        | HirArgument::Omitted => None,
    }
}

fn add_control_flow(
    function: &Function,
    line: LineId,
    location: SourceLocation,
    builder: &mut Builder<'_>,
    structured: &StructuredFlow,
    pending: &mut Vec<(usize, LineId, bool)>,
) {
    if let Some(target) = structured.false_targets.get(&line) {
        let instruction = builder.code.len();
        builder.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
        pending.push((instruction, *target, true));
        return;
    }
    let outgoing: Vec<_> = function
        .control_flow
        .iter()
        .filter(|edge| edge.from == line)
        .collect();
    if !structured.alternative_ends.contains_key(&line)
        && let Some(branch) = outgoing
            .iter()
            .find(|edge| edge.kind == ControlFlowKind::Branch)
        && let Some(target) = branch.to
    {
        let instruction = builder.code.len();
        builder.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
        pending.push((instruction, target, false));
        return;
    }
    if let Some(edge) = outgoing.iter().find(|edge| {
        matches!(
            edge.kind,
            ControlFlowKind::Goto
                | ControlFlowKind::Jump
                | ControlFlowKind::LoopBack
                | ControlFlowKind::Break
                | ControlFlowKind::Continue
        )
    }) && let Some(target) = edge.to
    {
        let instruction = builder.code.len();
        builder.emit(opcode::jump(Opcode::Jump, 0), location);
        pending.push((instruction, target, false));
    }
}

#[derive(Default)]
struct StructuredFlow {
    false_targets: BTreeMap<LineId, LineId>,
    alternative_ends: BTreeMap<LineId, LineId>,
}

struct OpenIf {
    opener: LineId,
    alternatives: Vec<(LineId, bool)>,
}

fn structured_if_flow(function: &Function) -> StructuredFlow {
    let mut result = StructuredFlow::default();
    let mut open = Vec::<OpenIf>::new();
    for line in &function.lines {
        let HirStatementKind::Instruction { target, .. } = &line.kind else {
            continue;
        };
        match target.name() {
            "IF" | "TRYCCALL" | "TRYCCALLFORM" | "TRYCJUMP" | "TRYCJUMPFORM" => open.push(OpenIf {
                opener: line.id,
                alternatives: Vec::new(),
            }),
            "ELSEIF" => {
                if let Some(frame) = open.last_mut() {
                    frame.alternatives.push((line.id, true));
                }
            }
            "ELSE" | "CATCH" => {
                if let Some(frame) = open.last_mut() {
                    frame.alternatives.push((line.id, false));
                }
            }
            "ENDIF" | "ENDCATCH" => {
                let Some(frame) = open.pop() else {
                    continue;
                };
                let mut previous_condition = Some(frame.opener);
                for (alternative, is_condition) in frame.alternatives {
                    if let Some(condition) = previous_condition {
                        result.false_targets.insert(condition, alternative);
                    }
                    result.alternative_ends.insert(alternative, line.id);
                    previous_condition = is_condition.then_some(alternative);
                }
                if let Some(condition) = previous_condition {
                    result.false_targets.insert(condition, line.id);
                }
            }
            _ => {}
        }
    }
    result
}

struct Builder<'a> {
    hir_function: &'a Function,
    context: &'a LoweringContext<'a>,
    code: Vec<EncodedInstruction>,
    locations: Vec<SourceLocation>,
    imports: Vec<FunctionImport>,
    native_imports: BTreeMap<SymbolKey, NativeImport>,
    host_imports: BTreeMap<SymbolKey, HostImport>,
    diagnostics: Vec<CompilerDiagnostic>,
}

impl<'a> Builder<'a> {
    fn new(
        hir_function: &'a Function,
        _function_key: SymbolKey,
        context: &'a LoweringContext<'a>,
    ) -> Self {
        Self {
            hir_function,
            context,
            code: Vec::new(),
            locations: Vec::new(),
            imports: Vec::new(),
            native_imports: BTreeMap::new(),
            host_imports: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }

    fn emit(&mut self, instruction: EncodedInstruction, location: SourceLocation) {
        self.code.push(instruction);
        self.locations.push(location);
    }

    fn lower_data_block(&mut self, block: &DataBlock<'_>) {
        let HirStatementKind::Instruction { target, arguments } = &block.opener.kind else {
            return;
        };
        if block.choices.is_empty() {
            return;
        }
        let name = target.name();
        let location = block.opener.location;
        let is_string = name == "STRDATA";
        let mut skip_jump = None;
        if !is_string {
            self.emit_runtime_call("ISSKIP", &[], Some(BytecodeType::Integer), false, location);
            self.emit(opcode::unary(2), location);
            skip_jump = Some(self.code.len());
            self.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
        }
        self.emit(
            opcode::push_integer(i64::try_from(block.choices.len()).unwrap_or(i64::MAX)),
            location,
        );
        self.emit_native_call(
            "RAND",
            &[BytecodeType::Integer],
            Some(BytecodeType::Integer),
            compiler_native_contract(false),
            location,
        );

        if !is_string && let Some(place) = argument_place(arguments.first()) {
            if place.indices.is_empty()
                && let Some(key) = self.context.variable_keys.get(&place.variable).copied()
            {
                self.emit(EncodedInstruction::new(Opcode::Dup, Vec::new()), location);
                self.emit(opcode::variable(Opcode::StoreVariable, key, 0, 0), location);
            } else {
                self.diagnostics.push(CompilerDiagnostic::at(
                    CompilerDiagnosticCode::UnsupportedConstruct,
                    location,
                    "PRINTDATA selected-index destination must currently be scalar",
                ));
            }
        }

        let mut end_jumps = Vec::new();
        for (index, choice) in block.choices.iter().enumerate() {
            let false_jump = if index + 1 < block.choices.len() {
                self.emit(EncodedInstruction::new(Opcode::Dup, Vec::new()), location);
                self.emit(
                    opcode::push_integer(i64::try_from(index).unwrap_or(i64::MAX)),
                    location,
                );
                self.emit(opcode::binary(11), location);
                let jump = self.code.len();
                self.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
                Some(jump)
            } else {
                None
            };
            self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
            if is_string {
                self.lower_strdata_choice(choice, arguments.first(), location);
            } else {
                self.lower_printdata_choice(choice, name);
            }
            let end = self.code.len();
            self.emit(opcode::jump(Opcode::Jump, 0), location);
            end_jumps.push(end);
            if let Some(jump) = false_jump {
                self.code[jump].payload = u32::try_from(self.code.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes()
                    .to_vec();
            }
        }
        let end = u32::try_from(self.code.len()).unwrap_or(u32::MAX);
        for jump in end_jumps {
            self.code[jump].payload = end.to_le_bytes().to_vec();
        }
        if !is_string {
            if name.ends_with('L') {
                self.emit_runtime_call("PRINTL", &[], None, false, location);
            } else if name.ends_with('W') {
                self.emit_runtime_call("PRINTW", &[], None, false, location);
            }
        }
        if let Some(jump) = skip_jump {
            self.code[jump].payload = u32::try_from(self.code.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes()
                .to_vec();
        }
    }

    fn lower_printdata_choice(&mut self, choice: &[&erabasic_hir::HirStatement], opener: &str) {
        for (index, line) in choice.iter().enumerate() {
            let HirStatementKind::Instruction { arguments, .. } = &line.kind else {
                continue;
            };
            if index != 0 {
                self.emit_runtime_call("PRINTL", &[], None, false, line.location);
            }
            let Some(argument) = arguments.first() else {
                continue;
            };
            let value_type = self.lower_argument(argument, line.location);
            let command = if opener.contains('K') {
                "PRINTK"
            } else if opener.contains('D') {
                "PRINTD"
            } else {
                "PRINT"
            };
            self.emit_runtime_call(command, &[value_type], None, false, line.location);
        }
    }

    fn lower_strdata_choice(
        &mut self,
        choice: &[&erabasic_hir::HirStatement],
        destination: Option<&HirArgument>,
        location: SourceLocation,
    ) {
        let mut parts = 0_u16;
        for (index, line) in choice.iter().enumerate() {
            if index != 0 {
                self.emit(opcode::push_string("\n"), line.location);
                parts = parts.saturating_add(1);
            }
            if let HirStatementKind::Instruction { arguments, .. } = &line.kind
                && let Some(argument) = arguments.first()
            {
                self.lower_argument(argument, line.location);
                parts = parts.saturating_add(1);
            }
        }
        if parts == 0 {
            self.emit(opcode::push_string(""), location);
        } else if parts > 1 {
            self.emit(opcode::concat(parts), location);
        }
        let Some(place) = argument_place(destination) else {
            self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
            return;
        };
        if place.indices.is_empty()
            && let Some(key) = self.context.variable_keys.get(&place.variable).copied()
        {
            self.emit(opcode::variable(Opcode::StoreVariable, key, 0, 0), location);
        } else {
            self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
            self.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::UnsupportedConstruct,
                location,
                "STRDATA destination must currently be scalar",
            ));
        }
    }

    fn lower_statement(
        &mut self,
        target: &InstructionTarget,
        arguments: &[HirArgument],
        line: LineId,
        location: SourceLocation,
    ) {
        let name = target.name();
        if matches!(name, "NOSKIP" | "ENDNOSKIP") {
            // The analyzer's block edge uses the ordinary conditional-branch shape.
            // NOSKIP therefore produces an internal true value while ENDNOSKIP is void.
            self.emit_runtime_call(
                name,
                &[],
                (name == "NOSKIP").then_some(BytecodeType::Integer),
                false,
                location,
            );
            return;
        }
        if name.starts_with("PRINTDATA")
            || matches!(
                name,
                "IF" | "ELSE"
                    | "ELSEIF"
                    | "ENDIF"
                    | "SIF"
                    | "WHILE"
                    | "WEND"
                    | "REPEAT"
                    | "REND"
                    | "FOR"
                    | "NEXT"
                    | "DO"
                    | "LOOP"
                    | "SELECTCASE"
                    | "CASE"
                    | "CASEELSE"
                    | "ENDSELECT"
                    | "TRYC"
                    | "CATCH"
                    | "ENDCATCH"
                    | "STRDATA"
                    | "DATALIST"
                    | "DATA"
                    | "DATAFORM"
                    | "ENDDATA"
                    | "ENDLIST"
                    | "TRYCALLLIST"
                    | "TRYJUMPLIST"
                    | "TRYGOTOLIST"
                    | "FUNC"
                    | "ENDFUNC"
            )
        {
            let parameter_types: Vec<_> = arguments
                .iter()
                .map(|argument| self.lower_argument(argument, location))
                .collect();
            let direct_condition = matches!(name, "IF" | "ELSEIF" | "WHILE");
            let has_branch = self
                .hir_function
                .control_flow
                .iter()
                .any(|edge| edge.from == line && edge.kind == ControlFlowKind::Branch);
            if name != "ELSE" && !direct_condition && (has_branch || !parameter_types.is_empty()) {
                self.emit_native_call(
                    &format!("control_{}", name.to_ascii_lowercase()),
                    &parameter_types,
                    has_branch.then_some(BytecodeType::Integer),
                    compiler_native_contract(false),
                    location,
                );
            }
            return;
        }
        if matches!(name, "RETURN" | "RETURNF" | "RETURNFORM") {
            for argument in arguments {
                self.lower_argument(argument, location);
            }
            self.emit(opcode::return_value(!arguments.is_empty()), location);
            return;
        }
        if matches!(
            name,
            "CALLFORM"
                | "CALLFORMF"
                | "JUMPFORM"
                | "TRYCALLFORM"
                | "TRYCALLFORMF"
                | "TRYJUMPFORM"
                | "TRYCCALL"
                | "TRYCCALLFORM"
                | "TRYCJUMP"
                | "TRYCJUMPFORM"
        ) {
            self.lower_dynamic_call(arguments, line, name, location);
            return;
        }
        if matches!(name, "CALL" | "CALLF" | "JUMP" | "TRYCALL" | "TRYJUMP") {
            self.lower_static_call(arguments, line, name, location);
            return;
        }
        let parameter_types: Vec<_> = arguments
            .iter()
            .map(|argument| self.lower_argument(argument, location))
            .collect();
        let extension = matches!(target, InstructionTarget::Extension(_));
        self.emit_runtime_call(name, &parameter_types, None, extension, location);
    }

    fn lower_static_call(
        &mut self,
        arguments: &[HirArgument],
        line: LineId,
        name: &str,
        location: SourceLocation,
    ) {
        let target = self
            .hir_function
            .control_flow
            .iter()
            .find(|edge| {
                edge.from == line
                    && matches!(edge.kind, ControlFlowKind::Call | ControlFlowKind::Jump)
            })
            .and_then(|edge| edge.function);
        let Some(target) = target.and_then(|id| self.context.function_keys.get(&id).copied())
        else {
            if name.starts_with("TRY") {
                // Reference TRY calls do not evaluate arguments when the target is absent.
                return;
            }
            self.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::MissingImport,
                location,
                format!("{name} target does not resolve to a function"),
            ));
            self.emit(
                EncodedInstruction::new(Opcode::Trap, b"missing function".to_vec()),
                location,
            );
            return;
        };
        let target_function = self
            .context
            .program
            .functions
            .iter()
            .find(|function| self.context.function_keys.get(&function.id) == Some(&target));
        let reference_parameters = target_function
            .map(|function| {
                function
                    .parameters
                    .iter()
                    .map(|parameter| {
                        self.context
                            .program
                            .variables
                            .get(parameter.target.variable.0 as usize)
                            .is_some_and(|variable| variable.reference)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let supplied = arguments.iter().skip(1).collect::<Vec<_>>();
        let mut parameter_types = Vec::new();
        if let Some(function) = target_function {
            for (index, parameter) in function.parameters.iter().enumerate() {
                let argument = supplied.get(index).copied();
                if matches!(argument, None | Some(HirArgument::Omitted)) {
                    if let Some(default) = &parameter.default {
                        parameter_types.push(self.lower_expression(default, location));
                    } else {
                        self.diagnostics.push(CompilerDiagnostic::at(
                            CompilerDiagnosticCode::InvalidHir,
                            location,
                            format!("{name} omits required argument {}", index + 1),
                        ));
                        let value_type = parameter.target.value_type;
                        match value_type {
                            SemanticType::String => self.emit(opcode::push_string(""), location),
                            _ => self.emit(opcode::push_integer(0), location),
                        }
                        parameter_types
                            .push(bytecode_type(value_type).unwrap_or(BytecodeType::Integer));
                    }
                    continue;
                }
                let argument = argument.expect("handled missing argument above");
                if reference_parameters.get(index) == Some(&true)
                    && let HirArgument::Expression(expression) = argument
                    && let HirExprKind::Variable { place } = &expression.kind
                {
                    parameter_types
                        .push(self.lower_argument(&HirArgument::Place(place.clone()), location));
                } else {
                    parameter_types.push(self.lower_argument(argument, location));
                }
            }
        }
        let result = target_function.and_then(|function| bytecode_type(function.return_type));
        let import = self.add_import(ImportKind::Function, target);
        self.emit(
            opcode::call(
                Opcode::Call,
                import,
                u16::try_from(parameter_types.len()).unwrap_or(u16::MAX),
                result,
            ),
            location,
        );
        if name.ends_with('F') && result.is_some() {
            self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
        }
        if name.contains("JUMP") {
            self.emit(opcode::return_value(result.is_some()), location);
        }
    }

    fn lower_dynamic_call(
        &mut self,
        arguments: &[HirArgument],
        line: LineId,
        name: &str,
        location: SourceLocation,
    ) {
        let Some(target) = arguments.first() else {
            self.emit(
                EncodedInstruction::new(Opcode::Trap, b"missing dynamic target".to_vec()),
                location,
            );
            return;
        };
        let target_type = self.lower_argument(target, location);
        if target_type != BytecodeType::String {
            self.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::InvalidHir,
                location,
                format!("{name} target is not a string"),
            ));
        }
        let allow_missing = name.starts_with("TRY");
        let has_catch = self
            .hir_function
            .control_flow
            .iter()
            .any(|edge| edge.from == line && edge.kind == ControlFlowKind::Branch);
        let resolve = self.code.len();
        self.emit(opcode::resolve_function(0, allow_missing), location);
        let parameter_types = arguments
            .iter()
            .skip(1)
            .map(|argument| self.lower_argument(argument, location))
            .collect::<Vec<_>>();
        self.emit(
            opcode::invoke_dynamic(
                u16::try_from(parameter_types.len()).unwrap_or(u16::MAX),
                name.contains("JUMP"),
            ),
            location,
        );
        if allow_missing {
            let success = self.code.len();
            if has_catch {
                self.emit(opcode::push_integer(1), location);
            }
            self.emit(opcode::jump(Opcode::Jump, 0), location);
            let missing = self.code.len();
            self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
            if has_catch {
                self.emit(opcode::push_integer(0), location);
            }
            let end = u32::try_from(self.code.len()).unwrap_or(u32::MAX);
            self.code[resolve].payload = {
                let mut payload = u32::try_from(missing)
                    .unwrap_or(u32::MAX)
                    .to_le_bytes()
                    .to_vec();
                payload.push(1);
                payload
            };
            self.code[success].payload = end.to_le_bytes().to_vec();
        }
    }

    fn lower_argument(&mut self, argument: &HirArgument, location: SourceLocation) -> BytecodeType {
        match argument {
            HirArgument::Expression(expression) => self.lower_expression(expression, location),
            HirArgument::Place(place) => {
                for index in &place.indices {
                    self.lower_expression(index, location);
                }
                let value_type = match place.value_type {
                    SemanticType::String => BytecodeType::StringPlace,
                    SemanticType::Integer | SemanticType::Void | SemanticType::Error => {
                        BytecodeType::IntegerPlace
                    }
                };
                if let Some(key) = self.context.variable_keys.get(&place.variable).copied() {
                    self.emit(
                        opcode::variable(
                            Opcode::MakePlace,
                            key,
                            u16::try_from(place.indices.len()).unwrap_or(u16::MAX),
                            0,
                        ),
                        location,
                    );
                } else {
                    self.emit(
                        EncodedInstruction::new(Opcode::Trap, b"missing variable place".to_vec()),
                        location,
                    );
                }
                value_type
            }
            HirArgument::Formatted(formatted) => self.lower_formatted(formatted, location),
            HirArgument::Raw(value) => {
                self.emit(opcode::push_string(value), location);
                BytecodeType::String
            }
            HirArgument::Omitted => {
                // EraBasic can distinguish an omitted operand from an explicit zero. The
                // internal call ABI reserves i64::MIN until bytecode gains a first-class
                // omitted value.
                self.emit(opcode::push_integer(i64::MIN), location);
                BytecodeType::Integer
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn lower_expression(&mut self, expression: &HirExpr, fallback: SourceLocation) -> BytecodeType {
        let location = expression.location;
        let result = bytecode_type(expression.value_type).unwrap_or(BytecodeType::Integer);
        match &expression.kind {
            HirExprKind::Integer { value } => self.emit(opcode::push_integer(*value), location),
            HirExprKind::String { value } => self.emit(opcode::push_string(value), location),
            HirExprKind::Variable { place } => {
                for index in &place.indices {
                    self.lower_expression(index, fallback);
                }
                if let Some(key) = self.context.variable_keys.get(&place.variable).copied() {
                    self.emit(
                        opcode::variable(
                            Opcode::LoadVariable,
                            key,
                            u16::try_from(place.indices.len()).unwrap_or(u16::MAX),
                            0,
                        ),
                        location,
                    );
                } else {
                    self.emit(
                        EncodedInstruction::new(Opcode::Trap, b"missing variable".to_vec()),
                        location,
                    );
                }
            }
            HirExprKind::Call { target, arguments } => {
                let builtin = matches!(target, CallTarget::Builtin { .. });
                let parameter_types: Vec<_> = arguments
                    .iter()
                    .filter_map(|argument| match argument {
                        HirCallArgument::Value(argument) => {
                            Some(self.lower_expression(argument, fallback))
                        }
                        HirCallArgument::Place(place) => Some(self.lower_argument(
                            &HirArgument::Place(place.clone()),
                            expression.location,
                        )),
                        HirCallArgument::Omitted if builtin => {
                            self.emit(opcode::push_integer(i64::MIN), expression.location);
                            Some(BytecodeType::Integer)
                        }
                        HirCallArgument::Omitted => None,
                    })
                    .collect();
                match target {
                    CallTarget::User { function } => {
                        if let Some(key) = self.context.function_keys.get(function).copied() {
                            let import = self.add_import(ImportKind::Function, key);
                            self.emit(
                                opcode::call(
                                    Opcode::Call,
                                    import,
                                    u16::try_from(parameter_types.len()).unwrap_or(u16::MAX),
                                    Some(result),
                                ),
                                location,
                            );
                        } else {
                            self.emit(
                                EncodedInstruction::new(Opcode::Trap, b"missing function".to_vec()),
                                location,
                            );
                        }
                    }
                    CallTarget::Builtin { name } => {
                        self.emit_runtime_call(
                            name,
                            &parameter_types,
                            Some(result),
                            false,
                            location,
                        );
                    }
                    CallTarget::Extension { name } => {
                        self.emit_runtime_call(
                            name,
                            &parameter_types,
                            Some(result),
                            true,
                            location,
                        );
                    }
                    CallTarget::Unresolved { name } => {
                        self.diagnostics.push(CompilerDiagnostic::at(
                            CompilerDiagnosticCode::MissingImport,
                            location,
                            format!("function {name} is unresolved"),
                        ));
                        self.emit(
                            EncodedInstruction::new(Opcode::Trap, b"unresolved call".to_vec()),
                            location,
                        );
                    }
                }
            }
            HirExprKind::Unary { op, operand } => {
                self.lower_expression(operand, fallback);
                self.emit(opcode::unary(unary_tag(*op)), location);
            }
            HirExprKind::Postfix { op, operand } => {
                self.lower_expression(operand, fallback);
                self.emit(opcode::unary(postfix_tag(*op)), location);
            }
            HirExprKind::Binary { op, left, right } => {
                self.lower_expression(left, fallback);
                self.lower_expression(right, fallback);
                self.emit(opcode::binary(binary_tag(*op)), location);
            }
            HirExprKind::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                self.lower_expression(condition, fallback);
                let false_jump = self.code.len();
                self.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
                self.lower_expression(then_expr, fallback);
                let end_jump = self.code.len();
                self.emit(opcode::jump(Opcode::Jump, 0), location);
                self.code[false_jump].payload = u32::try_from(self.code.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes()
                    .to_vec();
                self.lower_expression(else_expr, fallback);
                self.code[end_jump].payload = u32::try_from(self.code.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes()
                    .to_vec();
            }
            HirExprKind::Formatted { value } => {
                self.lower_formatted(value, fallback);
            }
            HirExprKind::Error => self.emit(
                EncodedInstruction::new(Opcode::Trap, b"invalid expression".to_vec()),
                fallback,
            ),
        }
        result
    }

    fn lower_formatted(
        &mut self,
        formatted: &HirFormattedString,
        fallback: SourceLocation,
    ) -> BytecodeType {
        let mut parts = 0u16;
        for part in &formatted.parts {
            match part {
                HirFormPart::Text { value } => {
                    self.emit(opcode::push_string(value), formatted.location);
                }
                HirFormPart::Interpolation {
                    expression,
                    width,
                    integer,
                    location,
                    ..
                } => {
                    let mut parameters = vec![self.lower_expression(expression, fallback)];
                    if let Some(width) = width {
                        parameters.push(self.lower_expression(width, fallback));
                    }
                    self.emit_native_call(
                        if *integer {
                            "format_integer"
                        } else {
                            "format_string"
                        },
                        &parameters,
                        Some(BytecodeType::String),
                        compiler_native_contract(true),
                        *location,
                    );
                }
                HirFormPart::Conditional {
                    condition,
                    then_value,
                    else_value,
                    location,
                } => {
                    self.lower_expression(condition, fallback);
                    let false_jump = self.code.len();
                    self.emit(opcode::jump(Opcode::JumpIfFalse, 0), *location);
                    self.lower_formatted(then_value, fallback);
                    let end_jump = self.code.len();
                    self.emit(opcode::jump(Opcode::Jump, 0), *location);
                    self.code[false_jump].payload = u32::try_from(self.code.len())
                        .unwrap_or(u32::MAX)
                        .to_le_bytes()
                        .to_vec();
                    if let Some(else_value) = else_value {
                        self.lower_formatted(else_value, fallback);
                    } else {
                        self.emit(opcode::push_string(""), *location);
                    }
                    self.code[end_jump].payload = u32::try_from(self.code.len())
                        .unwrap_or(u32::MAX)
                        .to_le_bytes()
                        .to_vec();
                }
                HirFormPart::Triple { symbol, location } => {
                    self.emit(opcode::push_string(&symbol.to_string()), *location);
                }
            }
            parts = parts.saturating_add(1);
        }
        if parts == 0 {
            self.emit(opcode::push_string(""), formatted.location);
        } else if parts > 1 {
            self.emit(opcode::concat(parts), formatted.location);
        }
        BytecodeType::String
    }

    fn emit_runtime_call(
        &mut self,
        name: &str,
        parameters: &[BytecodeType],
        result: Option<BytecodeType>,
        extension: bool,
        location: SourceLocation,
    ) {
        let classification = if extension {
            ExecutionBinding::Host(extension_binding(name))
        } else {
            self.context
                .host_registry
                .classification(name)
                .cloned()
                .unwrap_or(ExecutionBinding::Unsupported {
                    reason: "the callable has no execution catalog entry".into(),
                })
        };
        if let ExecutionBinding::Host(binding) = classification {
            let import = runtime_import(
                &binding.namespace,
                &binding.name,
                binding.abi_version,
                parameters,
                result,
            );
            let key = import.key;
            self.host_imports.entry(key).or_insert(HostImport {
                import,
                effect: binding.effect,
                capability: binding.capability,
                snapshot_capability: binding.snapshot_capability,
                contract: binding.contract,
            });
            let index = self.add_import(ImportKind::Host, key);
            self.emit(
                opcode::call(
                    Opcode::CallHost,
                    index,
                    u16::try_from(parameters.len()).unwrap_or(u16::MAX),
                    result,
                ),
                location,
            );
        } else if let ExecutionBinding::Native(contract) = classification {
            self.emit_native_call(name, parameters, result, contract, location);
        } else if let ExecutionBinding::Unsupported { reason } = classification {
            self.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::UnsupportedConstruct,
                location,
                format!("{name} is unsupported: {reason}"),
            ));
            self.emit(
                EncodedInstruction::new(Opcode::Trap, format!("unsupported {name}").into_bytes()),
                location,
            );
        }
    }

    fn emit_native_call(
        &mut self,
        name: &str,
        parameters: &[BytecodeType],
        result: Option<BytecodeType>,
        contract: erabasic_bytecode::OperationContract,
        location: SourceLocation,
    ) {
        let import = runtime_import(
            "rustyera.vm",
            &name.to_ascii_lowercase(),
            NATIVE_ABI_VERSION,
            parameters,
            result,
        );
        let key = import.key;
        self.native_imports.entry(key).or_insert(NativeImport {
            import,
            effect: contract.effect(),
            contract,
        });
        let index = self.add_import(ImportKind::Native, key);
        self.emit(
            opcode::call(
                Opcode::CallNative,
                index,
                u16::try_from(parameters.len()).unwrap_or(u16::MAX),
                result,
            ),
            location,
        );
    }

    fn add_import(&mut self, kind: ImportKind, key: SymbolKey) -> u32 {
        if let Some(index) = self
            .imports
            .iter()
            .position(|import| import.kind == kind && import.key == key)
        {
            return u32::try_from(index).unwrap_or(u32::MAX);
        }
        let index = self.imports.len();
        self.imports.push(FunctionImport { kind, key });
        u32::try_from(index).unwrap_or(u32::MAX)
    }
}

fn compiler_native_contract(pure: bool) -> erabasic_bytecode::OperationContract {
    use erabasic_bytecode::{
        CandidatePolicy, CapabilityFallback, OperationContract, OperationDebugPolicy,
        OperationHotReloadPolicy, OperationPersistence, OperationSnapshotPolicy, OperationState,
        OperationWaitPolicy, TransactionPolicy,
    };
    OperationContract {
        state: if pure {
            OperationState::Pure
        } else {
            OperationState::Vm
        },
        transaction: TransactionPolicy::ReadOnly,
        candidate: CandidatePolicy::ReadOnly,
        persistence: OperationPersistence::None,
        snapshot: OperationSnapshotPolicy::Included,
        hot_reload: OperationHotReloadPolicy::Preserve,
        wait: OperationWaitPolicy::Immediate,
        capability_fallback: CapabilityFallback::NotApplicable,
        debug: OperationDebugPolicy::Pure,
    }
}

fn runtime_import(
    namespace: &str,
    name: &str,
    abi_version: u32,
    parameters: &[BytecodeType],
    result: Option<BytecodeType>,
) -> RuntimeImport {
    let identity = serde_json::to_vec(&(namespace, name, abi_version, parameters, result))
        .expect("runtime import identity is serializable");
    RuntimeImport {
        key: SymbolKey::derive("rustyera.bytecode.runtime-import.v1", &identity),
        namespace: namespace.into(),
        name: name.into(),
        abi_version,
        parameters: parameters.to_vec(),
        result,
    }
}

pub(crate) fn bytecode_type(value: SemanticType) -> Option<BytecodeType> {
    match value {
        SemanticType::Integer => Some(BytecodeType::Integer),
        SemanticType::String => Some(BytecodeType::String),
        SemanticType::Void | SemanticType::Error => None,
    }
}

fn assign_tag(operation: AssignOp) -> u8 {
    match operation {
        AssignOp::Assign => 0,
        AssignOp::Add => 1,
        AssignOp::Subtract => 2,
        AssignOp::Multiply => 3,
        AssignOp::Divide => 4,
        AssignOp::Modulo => 5,
        AssignOp::BitAnd => 6,
        AssignOp::BitOr => 7,
        AssignOp::BitXor => 8,
        AssignOp::ShiftLeft => 9,
        AssignOp::ShiftRight => 10,
    }
}

fn unary_tag(operation: UnaryOp) -> u8 {
    match operation {
        UnaryOp::Plus => 0,
        UnaryOp::Minus => 1,
        UnaryOp::LogicalNot => 2,
        UnaryOp::BitNot => 3,
        UnaryOp::PreIncrement => 4,
        UnaryOp::PreDecrement => 5,
    }
}

fn postfix_tag(operation: PostfixOp) -> u8 {
    match operation {
        PostfixOp::Increment => 6,
        PostfixOp::Decrement => 7,
    }
}

fn binary_tag(operation: BinaryOp) -> u8 {
    match operation {
        BinaryOp::Multiply => 0,
        BinaryOp::Divide => 1,
        BinaryOp::Modulo => 2,
        BinaryOp::Add => 3,
        BinaryOp::Subtract => 4,
        BinaryOp::ShiftLeft => 5,
        BinaryOp::ShiftRight => 6,
        BinaryOp::Less => 7,
        BinaryOp::LessEqual => 8,
        BinaryOp::Greater => 9,
        BinaryOp::GreaterEqual => 10,
        BinaryOp::Equal => 11,
        BinaryOp::NotEqual => 12,
        BinaryOp::BitAnd => 13,
        BinaryOp::BitXor => 14,
        BinaryOp::BitOr => 15,
        BinaryOp::LogicalAnd => 16,
        BinaryOp::LogicalXor => 17,
        BinaryOp::LogicalOr => 18,
        BinaryOp::Nand => 19,
        BinaryOp::Nor => 20,
    }
}
