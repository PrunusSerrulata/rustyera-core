use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use erabasic_bytecode::{
    BytecodeArtifact, BytecodeConstant, BytecodeFunction, BytecodeStorage, BytecodeType,
    COMPILER_ABI_VERSION, CONTAINER_VERSION, FormatVersion, HOST_ABI_VERSION, HostCapability,
    HostImport, ISA_VERSION, ImportKind, NATIVE_ABI_VERSION, NativeImport, Opcode, SymbolKey,
    UnvalidatedArtifact, VM_ABI_VERSION, opcode,
};
use rayon::prelude::*;

use crate::{ValidationCode, ValidationDiagnostic, ValidationLimits, ValidationReport};

#[derive(Clone, Debug)]
pub struct ValidationContext {
    pub container_version: FormatVersion,
    pub isa_version: FormatVersion,
    pub compiler_abi: u32,
    pub native_abi: u32,
    pub host_abi: u32,
    pub vm_abi: u32,
    pub supported_features: BTreeSet<String>,
    pub native_imports: BTreeMap<SymbolKey, NativeImport>,
    pub host_imports: BTreeMap<SymbolKey, HostImport>,
    pub host_capabilities: BTreeSet<HostCapability>,
    pub limits: ValidationLimits,
}

impl Default for ValidationContext {
    fn default() -> Self {
        Self {
            container_version: CONTAINER_VERSION,
            isa_version: ISA_VERSION,
            compiler_abi: COMPILER_ABI_VERSION,
            native_abi: NATIVE_ABI_VERSION,
            host_abi: HOST_ABI_VERSION,
            vm_abi: VM_ABI_VERSION,
            supported_features: BTreeSet::new(),
            native_imports: BTreeMap::new(),
            host_imports: BTreeMap::new(),
            host_capabilities: BTreeSet::new(),
            limits: ValidationLimits::default(),
        }
    }
}

impl ValidationContext {
    #[must_use]
    pub fn for_artifact(artifact: &BytecodeArtifact) -> Self {
        Self {
            supported_features: artifact
                .manifest
                .required_features
                .iter()
                .cloned()
                .collect(),
            native_imports: artifact
                .native_imports
                .iter()
                .cloned()
                .map(|import| (import.import.key, import))
                .collect(),
            host_imports: artifact
                .host_imports
                .iter()
                .cloned()
                .map(|import| (import.import.key, import))
                .collect(),
            host_capabilities: artifact
                .host_imports
                .iter()
                .map(|import| import.capability)
                .collect(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedArtifact(Arc<BytecodeArtifact>);

impl ValidatedArtifact {
    #[must_use]
    pub fn artifact(&self) -> &BytecodeArtifact {
        &self.0
    }

    #[must_use]
    pub fn into_inner(self) -> BytecodeArtifact {
        Arc::unwrap_or_clone(self.0)
    }

    /// Return shared ownership of the immutable, already-validated artifact.
    ///
    /// Runtime and VM layers commonly retain the same large project artifact. Sharing it
    /// avoids cloning all bytecode and source-map records when a VM generation is created.
    #[must_use]
    pub fn into_shared(self) -> Arc<BytecodeArtifact> {
        self.0
    }
}

#[must_use]
pub fn validate_bytecode(
    artifact: UnvalidatedArtifact,
    context: &ValidationContext,
) -> ValidationReport<ValidatedArtifact> {
    validate_artifact(artifact.into_inner(), context, true)
}

/// Validate an in-process compiler artifact without recomputing its IDs.
///
/// The compiler uses this before assigning IDs and the runtime may use it again
/// while accepting that same compiler-owned value. Serialized, externally
/// supplied, or otherwise untrusted artifacts must use [`validate_bytecode`],
/// which recomputes and checks both content identities.
#[must_use]
pub fn validate_compiler_output(
    mut artifact: BytecodeArtifact,
    context: &ValidationContext,
) -> ValidationReport<ValidatedArtifact> {
    // Identity generation canonicalizes the artifact too. Do that ordering step
    // here so structural validation observes exactly the artifact that will be
    // hashed after validation succeeds.
    artifact.canonicalize();
    validate_artifact(artifact, context, false)
}

fn validate_artifact(
    mut artifact: BytecodeArtifact,
    context: &ValidationContext,
    validate_identity: bool,
) -> ValidationReport<ValidatedArtifact> {
    let mut diagnostics = Vec::new();
    validate_versions(&artifact, context, &mut diagnostics);
    validate_limits(&artifact, context, &mut diagnostics);
    if validate_identity {
        validate_identities(&mut artifact, &mut diagnostics);
    }
    validate_symbols(&artifact, context, &mut diagnostics);
    validate_source_map(&artifact, &mut diagnostics);
    validate_functions(&artifact, context, &mut diagnostics);
    ValidationReport {
        value: diagnostics
            .is_empty()
            .then_some(ValidatedArtifact(Arc::new(artifact))),
        diagnostics,
    }
}

fn validate_versions(
    artifact: &BytecodeArtifact,
    context: &ValidationContext,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let manifest = &artifact.manifest;
    if manifest.container_version.major != context.container_version.major
        || manifest.container_version.minor > context.container_version.minor
        || manifest.isa_version.major != context.isa_version.major
        || manifest.isa_version.minor > context.isa_version.minor
        || manifest.compiler_abi != context.compiler_abi
        || manifest.native_abi != context.native_abi
        || manifest.program_version.host_abi != context.host_abi
        || manifest.program_version.vm_abi != context.vm_abi
    {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::UnsupportedVersion,
            "artifact version is not supported by this VM contract",
        ));
    }
    for feature in &manifest.required_features {
        if !context.supported_features.contains(feature) {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::UnsupportedFeature,
                format!("required bytecode feature {feature} is not supported"),
            ));
        }
    }
}

fn validate_limits(
    artifact: &BytecodeArtifact,
    context: &ValidationContext,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let limits = context.limits;
    if artifact.functions.len() > limits.maximum_functions
        || artifact.globals.len() > limits.maximum_globals
        || artifact.source_map.entries.len() > limits.maximum_source_map_entries
    {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::ResourceLimit,
            "artifact exceeds a project resource limit",
        ));
    }
    let mut resident_elements = 0u64;
    let mut function_elements = BTreeMap::<SymbolKey, u64>::new();
    for global in &artifact.globals {
        let elements = global
            .dimensions
            .iter()
            .try_fold(1u64, |total, dimension| total.checked_mul(*dimension));
        if global.dimensions.len() > limits.maximum_dimensions_per_variable
            || elements.is_none_or(|elements| elements > limits.maximum_elements_per_variable)
        {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::ResourceLimit,
                format!("variable {} exceeds a storage resource limit", global.name),
            ));
        }
        let elements = elements.unwrap_or(u64::MAX);
        if matches!(
            global.storage,
            BytecodeStorage::FunctionLocal
                | BytecodeStorage::FunctionStatic
                | BytecodeStorage::FunctionPersistent
        ) {
            if let Some(owner) = global.owner {
                let total = function_elements.entry(owner).or_default();
                *total = total.saturating_add(elements);
            }
        } else {
            resident_elements = resident_elements.saturating_add(elements);
        }
    }
    if resident_elements > limits.maximum_total_variable_elements {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::ResourceLimit,
            "artifact exceeds the total variable storage limit",
        ));
    } else {
        for (owner, elements) in function_elements {
            if resident_elements.saturating_add(elements) > limits.maximum_total_variable_elements {
                let function = artifact
                    .functions
                    .iter()
                    .find(|function| function.key == owner)
                    .map_or("<unknown>", |function| function.name.as_str());
                diagnostics.push(ValidationDiagnostic::project(
                    ValidationCode::ResourceLimit,
                    format!(
                        "function {function} variable storage exceeds the total variable storage limit"
                    ),
                ));
            }
        }
    }
    for function in &artifact.functions {
        if function.code.len() > limits.maximum_instructions_per_function
            || function.imports.len() > limits.maximum_imports_per_function
            || function.max_stack > limits.maximum_stack
        {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::ResourceLimit,
                format!("function {} exceeds a resource limit", function.name),
            ));
        }
    }
}

fn validate_identities(
    artifact: &mut BytecodeArtifact,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let execution_id = artifact.manifest.program_version.execution_id;
    let artifact_id = artifact.manifest.artifact_id;
    if artifact.refresh_ids().is_err()
        || artifact.manifest.program_version.execution_id != execution_id
        || artifact.manifest.artifact_id != artifact_id
    {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::InvalidOperand,
            "artifact identity does not match its canonical contents",
        ));
    }
}

fn validate_symbols(
    artifact: &BytecodeArtifact,
    context: &ValidationContext,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    ensure_unique(
        artifact.globals.iter().map(|global| global.key),
        "global",
        diagnostics,
    );
    validate_runtime_layout(artifact, diagnostics);
    for import in &artifact.native_imports {
        validate_operation_contract(
            &import.import.namespace,
            &import.import.name,
            import.effect,
            None,
            import.contract,
            diagnostics,
        );
        if context.native_imports.get(&import.import.key) != Some(import) {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::HostAbiMismatch,
                format!(
                    "native import {}.{} is not bound with the required ABI",
                    import.import.namespace, import.import.name
                ),
            ));
        }
    }
    for import in &artifact.host_imports {
        validate_operation_contract(
            &import.import.namespace,
            &import.import.name,
            import.effect,
            Some(import.snapshot_capability),
            import.contract,
            diagnostics,
        );
        if context.host_imports.get(&import.import.key) != Some(import) {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::HostAbiMismatch,
                format!(
                    "host import {}.{} is not bound with the required ABI",
                    import.import.namespace, import.import.name
                ),
            ));
        }
        if !context.host_capabilities.contains(&import.capability) {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::MissingCapability,
                format!("host capability {:?} is not available", import.capability),
            ));
        }
    }
}

fn validate_operation_contract(
    namespace: &str,
    name: &str,
    effect: erabasic_bytecode::HostEffect,
    snapshot_capability: Option<erabasic_bytecode::HostSnapshotCapability>,
    contract: erabasic_bytecode::OperationContract,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    if !contract.is_coherent()
        || effect != contract.effect()
        || snapshot_capability.is_some_and(|value| value != contract.snapshot_capability())
    {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::InvalidOperationContract,
            format!("operation {namespace}.{name} has a contradictory execution contract"),
        ));
    }
}

fn validate_runtime_layout(
    artifact: &BytecodeArtifact,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let function_keys: BTreeSet<_> = artifact
        .functions
        .iter()
        .map(|function| function.key)
        .collect();
    let reference_parameters: BTreeSet<_> = artifact
        .functions
        .iter()
        .flat_map(|function| &function.parameters)
        .filter(|parameter| parameter.by_reference)
        .map(|parameter| parameter.key)
        .collect();
    for global in &artifact.globals {
        let function_storage = matches!(
            global.storage,
            BytecodeStorage::FunctionLocal
                | BytecodeStorage::FunctionStatic
                | BytecodeStorage::FunctionPersistent
        );
        let unbound_reference = global.storage == BytecodeStorage::FunctionLocal
            && reference_parameters.contains(&global.key);
        let disabled_builtin = global.dimensions.contains(&0)
            && artifact
                .project_data
                .schema
                .variable(&global.name)
                .is_some_and(|schema| {
                    schema.can_forbid
                        && schema.dimensions.len() == global.dimensions.len()
                        && schema
                            .dimensions
                            .iter()
                            .zip(&global.dimensions)
                            .all(|(schema, bytecode)| u64::try_from(*schema) == Ok(*bytecode))
                });
        if function_storage != global.owner.is_some()
            || global
                .owner
                .is_some_and(|owner| !function_keys.contains(&owner))
            || (global.storage != BytecodeStorage::Calculated
                && global.dimensions.contains(&0)
                && !unbound_reference
                && !disabled_builtin)
        {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::InvalidOperand,
                format!("variable {} has an invalid runtime layout", global.name),
            ));
        }
        let element_count = global
            .dimensions
            .iter()
            .try_fold(1u64, |total, dimension| total.checked_mul(*dimension))
            .unwrap_or(0);
        let initial_types_match = global.initial_values.iter().all(|value| {
            matches!(
                (global.value_type, value),
                (BytecodeType::Integer, BytecodeConstant::Integer(_))
                    | (BytecodeType::String, BytecodeConstant::String(_))
            )
        });
        if !initial_types_match || global.initial_values.len() as u64 > element_count {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::TypeMismatch,
                format!("variable {} has invalid initial values", global.name),
            ));
        }
    }
    validate_function_parameters(artifact, diagnostics);
    ensure_unique(
        artifact.functions.iter().map(|function| function.key),
        "function",
        diagnostics,
    );
    validate_event_groups(artifact, &function_keys, diagnostics);
    ensure_unique(
        artifact
            .native_imports
            .iter()
            .map(|import| import.import.key),
        "native import",
        diagnostics,
    );
    ensure_unique(
        artifact.host_imports.iter().map(|import| import.import.key),
        "host import",
        diagnostics,
    );
}

fn validate_function_parameters(
    artifact: &BytecodeArtifact,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let globals: BTreeMap<_, _> = artifact
        .globals
        .iter()
        .map(|global| (global.key, global))
        .collect();
    let functions: BTreeMap<_, _> = artifact
        .functions
        .iter()
        .map(|function| (function.key, function))
        .collect();
    for function in &artifact.functions {
        for parameter in &function.parameters {
            let default_valid = match (&parameter.default, parameter.value_type) {
                (None, _) => true,
                (Some(erabasic_bytecode::BytecodeConstant::Integer(_)), BytecodeType::Integer)
                | (Some(erabasic_bytecode::BytecodeConstant::String(_)), BytecodeType::String) => {
                    !parameter.by_reference
                }
                _ => false,
            };
            let valid = default_valid
                && globals.get(&parameter.key).is_some_and(|global| {
                    let function_storage = matches!(
                        global.storage,
                        BytecodeStorage::FunctionLocal
                            | BytecodeStorage::FunctionStatic
                            | BytecodeStorage::FunctionPersistent
                    );
                    let owner_matches = !function_storage
                        || global.owner == Some(function.key)
                        || (global.storage == BytecodeStorage::FunctionPersistent
                            && global.owner.is_some_and(|owner| {
                                functions.get(&owner).is_some_and(|candidate| {
                                    candidate.name.eq_ignore_ascii_case(&function.name)
                                })
                            }));
                    let type_matches = match parameter.value_type {
                        BytecodeType::IntegerPlace => global.value_type == BytecodeType::Integer,
                        BytecodeType::StringPlace => global.value_type == BytecodeType::String,
                        BytecodeType::Integer | BytecodeType::String => {
                            global.value_type == parameter.value_type
                        }
                    };
                    let maximum_indices = global.dimensions.len()
                        + usize::from(global.storage == BytecodeStorage::Character);
                    global.key == parameter.key
                        && owner_matches
                        && global.mutable
                        && global.storage != BytecodeStorage::Calculated
                        && parameter.indices.len() <= maximum_indices
                        && type_matches
                });
            if !valid {
                diagnostics.push(ValidationDiagnostic::project(
                    ValidationCode::InvalidOperand,
                    format!(
                        "function {} has an invalid parameter binding",
                        function.name
                    ),
                ));
            }
        }
    }
}

fn validate_event_groups(
    artifact: &BytecodeArtifact,
    function_keys: &BTreeSet<SymbolKey>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let mut event_names = BTreeSet::new();
    for group in &artifact.event_groups {
        let valid_name =
            !group.name.is_empty() && event_names.insert(group.name.to_ascii_uppercase());
        let members_valid = group
            .only
            .iter()
            .chain(&group.priority)
            .chain(&group.normal)
            .chain(&group.later)
            .all(|entry| function_keys.contains(&entry.function));
        if !valid_name || !members_valid {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::InvalidOperand,
                format!("event group {} has an invalid dispatch table", group.name),
            ));
        }
    }
}

fn ensure_unique(
    identities: impl IntoIterator<Item = SymbolKey>,
    category: &str,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let mut seen = BTreeSet::new();
    for identity in identities {
        if !seen.insert(identity) {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::DuplicateIdentity,
                format!("duplicate {category} identity {identity:?}"),
            ));
        }
    }
}

fn validate_source_map(artifact: &BytecodeArtifact, diagnostics: &mut Vec<ValidationDiagnostic>) {
    for source in &artifact.source_map.sources {
        if source.line_starts.first() != Some(&0)
            || !source.line_starts.windows(2).all(|pair| pair[0] < pair[1])
            || source
                .line_starts
                .last()
                .is_some_and(|offset| *offset > source.byte_len)
        {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::InvalidSourceMap,
                format!("source {} has an invalid line table", source.relative_path),
            ));
        }
    }
    let functions: BTreeMap<_, _> = artifact
        .functions
        .iter()
        .map(|function| {
            (
                function.key,
                function
                    .code
                    .iter()
                    .map(erabasic_bytecode::EncodedInstruction::encoded_len)
                    .sum::<u64>(),
            )
        })
        .collect();
    let entry_diagnostics = artifact
        .source_map
        .entries
        .par_chunks(65_536)
        .map(|entries| {
            let mut chunk_diagnostics = Vec::new();
            for entry in entries {
                let valid = functions.get(&entry.function).is_some_and(|length| {
                    entry.code_start < entry.code_end && entry.code_end <= *length
                }) && artifact
                    .source_map
                    .sources
                    .get(entry.source_index as usize)
                    .is_some_and(|source| {
                        entry.byte_start <= entry.byte_end && entry.byte_end <= source.byte_len
                    })
                    && artifact
                        .source_map
                        .statement_fingerprints
                        .get(entry.statement_fingerprint as usize)
                        .is_some();
                if !valid {
                    chunk_diagnostics.push(ValidationDiagnostic::project(
                        ValidationCode::InvalidSourceMap,
                        format!(
                            "source-map entry is outside its function or source \
                             (function={:?}, code={}..{} of {:?}, source={}, bytes={}..{} of {:?}, \
                             fingerprint={} of {})",
                            entry.function,
                            entry.code_start,
                            entry.code_end,
                            functions.get(&entry.function),
                            entry.source_index,
                            entry.byte_start,
                            entry.byte_end,
                            artifact
                                .source_map
                                .sources
                                .get(entry.source_index as usize)
                                .map(|source| source.byte_len),
                            entry.statement_fingerprint,
                            artifact.source_map.statement_fingerprints.len(),
                        ),
                    ));
                }
            }
            chunk_diagnostics
        })
        .collect::<Vec<_>>();
    for mut chunk in entry_diagnostics {
        diagnostics.append(&mut chunk);
    }
}

fn validate_functions(
    artifact: &BytecodeArtifact,
    context: &ValidationContext,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let globals: BTreeMap<_, _> = artifact
        .globals
        .iter()
        .map(|global| (global.key, global))
        .collect();
    let functions: BTreeMap<_, _> = artifact
        .functions
        .iter()
        .map(|function| (function.key, function))
        .collect();
    let native: BTreeMap<_, _> = artifact
        .native_imports
        .iter()
        .map(|import| (import.import.key, &import.import))
        .collect();
    let host: BTreeMap<_, _> = artifact
        .host_imports
        .iter()
        .map(|import| (import.import.key, &import.import))
        .collect();
    let function_diagnostics = artifact
        .functions
        .par_iter()
        .map(|function| {
            let mut diagnostics = Vec::new();
            validate_function(
                function,
                &globals,
                &functions,
                &native,
                &host,
                context,
                &mut diagnostics,
            );
            diagnostics
        })
        .collect::<Vec<_>>();
    for mut function in function_diagnostics {
        diagnostics.append(&mut function);
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_function(
    function: &BytecodeFunction,
    globals: &BTreeMap<SymbolKey, &erabasic_bytecode::BytecodeGlobal>,
    functions: &BTreeMap<SymbolKey, &BytecodeFunction>,
    native: &BTreeMap<SymbolKey, &erabasic_bytecode::RuntimeImport>,
    host: &BTreeMap<SymbolKey, &erabasic_bytecode::RuntimeImport>,
    _context: &ValidationContext,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    if function.code.is_empty() {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::InvalidControlFlow,
            format!("function {} has no instructions", function.name),
        ));
        return;
    }
    let mut label_names = BTreeSet::new();
    if function.labels.iter().any(|label| {
        label.name.is_empty()
            || !label_names.insert(label.name.to_ascii_uppercase())
            || label.instruction as usize >= function.code.len()
    }) {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::InvalidControlFlow,
            format!(
                "function {} has an invalid dynamic-label table",
                function.name
            ),
        ));
        return;
    }
    let mut states = vec![None; function.code.len()];
    states[0] = Some(Vec::<BytecodeType>::new());
    let mut work = VecDeque::from([0usize]);
    let mut observed_max = 0usize;
    while let Some(index) = work.pop_front() {
        let Some(mut stack) = states[index].clone() else {
            continue;
        };
        let successors = match apply_instruction(
            function, index, &mut stack, globals, functions, native, host,
        ) {
            Ok(successors) => successors,
            Err((code, message)) => {
                diagnostics.push(ValidationDiagnostic::instruction(
                    code,
                    &function.name,
                    index,
                    message,
                ));
                continue;
            }
        };
        observed_max = observed_max.max(stack.len());
        for successor in successors {
            if successor >= function.code.len() {
                diagnostics.push(ValidationDiagnostic::instruction(
                    ValidationCode::InvalidControlFlow,
                    &function.name,
                    index,
                    "control flow leaves the function",
                ));
                continue;
            }
            match &states[successor] {
                Some(existing) if existing != &stack => {
                    diagnostics.push(ValidationDiagnostic::instruction(
                        ValidationCode::StackMismatch,
                        &function.name,
                        successor,
                        "control-flow predecessors have different stack states",
                    ));
                }
                Some(_) => {}
                None => {
                    states[successor] = Some(stack.clone());
                    work.push_back(successor);
                }
            }
        }
    }
    if observed_max > function.max_stack as usize {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::ResourceLimit,
            format!("function {} understates its maximum stack", function.name),
        ));
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn apply_instruction(
    function: &BytecodeFunction,
    index: usize,
    stack: &mut Vec<BytecodeType>,
    globals: &BTreeMap<SymbolKey, &erabasic_bytecode::BytecodeGlobal>,
    functions: &BTreeMap<SymbolKey, &BytecodeFunction>,
    native: &BTreeMap<SymbolKey, &erabasic_bytecode::RuntimeImport>,
    host: &BTreeMap<SymbolKey, &erabasic_bytecode::RuntimeImport>,
) -> Result<Vec<usize>, (ValidationCode, String)> {
    let instruction = &function.code[index];
    let opcode_value = Opcode::try_from(instruction.opcode).map_err(|unknown| {
        (
            ValidationCode::UnknownOpcode,
            format!("unknown opcode {unknown}"),
        )
    })?;
    let next = || {
        (index + 1 < function.code.len())
            .then_some(index + 1)
            .into_iter()
            .collect()
    };
    match opcode_value {
        Opcode::Nop | Opcode::Yield | Opcode::ForBreak | Opcode::SelectEnd => {
            expect_payload(&instruction.payload, 0)?;
        }
        Opcode::PushInteger => {
            expect_payload(&instruction.payload, 8)?;
            stack.push(BytecodeType::Integer);
        }
        Opcode::PushString => {
            let length = read_u32(&instruction.payload, 0)? as usize;
            if instruction.payload.len() != 4 + length
                || std::str::from_utf8(&instruction.payload[4..]).is_err()
            {
                return Err((
                    ValidationCode::InvalidOperand,
                    "invalid UTF-8 string operand".into(),
                ));
            }
            stack.push(BytecodeType::String);
        }
        Opcode::LoadVariable | Opcode::StoreVariable | Opcode::MakePlace => {
            expect_payload(&instruction.payload, 19)?;
            let key = read_key(&instruction.payload)?;
            let indices = read_u16(&instruction.payload, 16)? as usize;
            let global = globals.get(&key).copied().ok_or((
                ValidationCode::MissingReference,
                "variable operand does not resolve".into(),
            ))?;
            let maximum_indices =
                global.dimensions.len() + usize::from(global.storage == BytecodeStorage::Character);
            if indices > maximum_indices {
                return Err((
                    ValidationCode::InvalidOperand,
                    "variable index count exceeds its schema".into(),
                ));
            }
            if matches!(opcode_value, Opcode::LoadVariable | Opcode::MakePlace)
                && instruction.payload[18] != 0
            {
                return Err((
                    ValidationCode::InvalidOperand,
                    "load instruction has a store operation tag".into(),
                ));
            }
            if opcode_value == Opcode::StoreVariable {
                if !global.mutable {
                    return Err((
                        ValidationCode::InvalidOperand,
                        "store instruction targets an immutable variable".into(),
                    ));
                }
                if instruction.payload[18] > 10 {
                    return Err((
                        ValidationCode::InvalidOperand,
                        "store instruction has an unknown assignment operation".into(),
                    ));
                }
            }
            if opcode_value == Opcode::StoreVariable {
                pop_type(stack, global.value_type)?;
            }
            for _ in 0..indices {
                pop_type(stack, BytecodeType::Integer)?;
            }
            if opcode_value == Opcode::LoadVariable {
                stack.push(global.value_type);
            } else if opcode_value == Opcode::MakePlace {
                stack.push(match global.value_type {
                    BytecodeType::Integer => BytecodeType::IntegerPlace,
                    BytecodeType::String => BytecodeType::StringPlace,
                    BytecodeType::IntegerPlace | BytecodeType::StringPlace => {
                        return Err((
                            ValidationCode::InvalidOperand,
                            "a variable schema cannot contain place values".into(),
                        ));
                    }
                });
            }
        }
        Opcode::Unary => {
            expect_payload(&instruction.payload, 1)?;
            if instruction.payload[0] > 7 {
                return Err((
                    ValidationCode::InvalidOperand,
                    "unknown unary operation".into(),
                ));
            }
            pop_type(stack, BytecodeType::Integer)?;
            stack.push(BytecodeType::Integer);
        }
        Opcode::Binary => {
            expect_payload(&instruction.payload, 1)?;
            if instruction.payload[0] > 20 {
                return Err((
                    ValidationCode::InvalidOperand,
                    "unknown binary operation".into(),
                ));
            }
            let right = stack.pop().ok_or((
                ValidationCode::StackMismatch,
                "binary operation underflows the stack".into(),
            ))?;
            let left = stack.pop().ok_or((
                ValidationCode::StackMismatch,
                "binary operation underflows the stack".into(),
            ))?;
            let string_repeat = instruction.payload[0] == 0
                && matches!(
                    (left, right),
                    (BytecodeType::String, BytecodeType::Integer)
                        | (BytecodeType::Integer, BytecodeType::String)
                );
            if !string_repeat
                && (left != right || !matches!(left, BytecodeType::Integer | BytecodeType::String))
            {
                return Err((
                    ValidationCode::TypeMismatch,
                    "binary operands have incompatible types".into(),
                ));
            }
            if !string_repeat
                && left == BytecodeType::String
                && !matches!(instruction.payload[0], 3 | 7..=12)
            {
                return Err((
                    ValidationCode::TypeMismatch,
                    "binary operation is not defined for strings".into(),
                ));
            }
            let result =
                if string_repeat || (left == BytecodeType::String && instruction.payload[0] == 3) {
                    BytecodeType::String
                } else {
                    BytecodeType::Integer
                };
            stack.push(result);
        }
        Opcode::ToString => {
            expect_payload(&instruction.payload, 0)?;
            stack.pop().ok_or((
                ValidationCode::StackMismatch,
                "string conversion underflows the stack".into(),
            ))?;
            stack.push(BytecodeType::String);
        }
        Opcode::Concat => {
            expect_payload(&instruction.payload, 2)?;
            for _ in 0..read_u16(&instruction.payload, 0)? {
                pop_type(stack, BytecodeType::String)?;
            }
            stack.push(BytecodeType::String);
        }
        Opcode::Pop => {
            expect_payload(&instruction.payload, 0)?;
            stack.pop().ok_or((
                ValidationCode::StackMismatch,
                "pop underflows the stack".into(),
            ))?;
        }
        Opcode::Dup => {
            expect_payload(&instruction.payload, 0)?;
            let value = *stack.last().ok_or((
                ValidationCode::StackMismatch,
                "dup underflows the stack".into(),
            ))?;
            stack.push(value);
        }
        Opcode::StorePlace => {
            expect_payload(&instruction.payload, 0)?;
            let place = stack.pop().ok_or((
                ValidationCode::StackMismatch,
                "indirect store underflows the stack".into(),
            ))?;
            let value = stack.pop().ok_or((
                ValidationCode::StackMismatch,
                "indirect store underflows the stack".into(),
            ))?;
            if !matches!(
                (place, value),
                (BytecodeType::IntegerPlace, BytecodeType::Integer)
                    | (BytecodeType::StringPlace, BytecodeType::String)
            ) {
                return Err((
                    ValidationCode::TypeMismatch,
                    "indirect store place and value types differ".into(),
                ));
            }
        }
        Opcode::Jump => {
            expect_payload(&instruction.payload, 4)?;
            return Ok(vec![read_u32(&instruction.payload, 0)? as usize]);
        }
        Opcode::JumpIfFalse => {
            expect_payload(&instruction.payload, 4)?;
            pop_type(stack, BytecodeType::Integer)?;
            return Ok(vec![read_u32(&instruction.payload, 0)? as usize, index + 1]);
        }
        Opcode::ForStart => {
            expect_payload(&instruction.payload, 0)?;
            pop_type(stack, BytecodeType::Integer)?;
            pop_type(stack, BytecodeType::Integer)?;
            pop_type(stack, BytecodeType::Integer)?;
            pop_type(stack, BytecodeType::IntegerPlace)?;
            stack.push(BytecodeType::Integer);
        }
        Opcode::ForNext => {
            expect_payload(&instruction.payload, 0)?;
            stack.push(BytecodeType::Integer);
        }
        Opcode::SelectStart => {
            expect_payload(&instruction.payload, 0)?;
            stack.pop().ok_or((
                ValidationCode::StackMismatch,
                "SELECTCASE underflows the stack".into(),
            ))?;
        }
        Opcode::SelectCompare => {
            expect_payload(&instruction.payload, 1)?;
            let operands = if instruction.payload[0] == 6 { 2 } else { 1 };
            if instruction.payload[0] > 7 {
                return Err((
                    ValidationCode::InvalidOperand,
                    "SELECTCASE comparison has an unknown operation".into(),
                ));
            }
            for _ in 0..operands {
                stack.pop().ok_or((
                    ValidationCode::StackMismatch,
                    "CASE comparison underflows the stack".into(),
                ))?;
            }
            stack.push(BytecodeType::Integer);
        }
        Opcode::ResolveFunction => {
            expect_payload(&instruction.payload, 6)?;
            pop_type(stack, BytecodeType::String)?;
            stack.push(BytecodeType::String);
            if instruction.payload[4] > 1 {
                return Err((
                    ValidationCode::InvalidOperand,
                    "resolve-function allow-missing flag is invalid".into(),
                ));
            }
            if instruction.payload[5] > 1 {
                return Err((
                    ValidationCode::InvalidOperand,
                    "resolve-function method flag is invalid".into(),
                ));
            }
            if instruction.payload[4] == 1 {
                return Ok(vec![read_u32(&instruction.payload, 0)? as usize, index + 1]);
            }
        }
        Opcode::InvokeDynamic => {
            expect_payload(&instruction.payload, 3)?;
            if instruction.payload[2] > 1 {
                return Err((
                    ValidationCode::InvalidOperand,
                    "dynamic-invoke tail flag is invalid".into(),
                ));
            }
            for _ in 0..read_u16(&instruction.payload, 0)? {
                stack.pop().ok_or((
                    ValidationCode::StackMismatch,
                    "dynamic call argument underflows the stack".into(),
                ))?;
            }
            pop_type(stack, BytecodeType::String)?;
        }
        Opcode::JumpDynamicLabel => {
            expect_payload(&instruction.payload, 4)?;
            pop_type(stack, BytecodeType::String)?;
            let mut successors = vec![read_u32(&instruction.payload, 0)? as usize];
            successors.extend(
                function
                    .labels
                    .iter()
                    .map(|label| label.instruction as usize),
            );
            successors.sort_unstable();
            successors.dedup();
            return Ok(successors);
        }
        Opcode::InvokeEvent => {
            expect_payload(&instruction.payload, 0)?;
            pop_type(stack, BytecodeType::String)?;
        }
        Opcode::Call | Opcode::CallNative | Opcode::CallHost => {
            expect_payload(&instruction.payload, 7)?;
            let import_index = read_u32(&instruction.payload, 0)? as usize;
            let declared_arguments = read_u16(&instruction.payload, 4)? as usize;
            let import = function.imports.get(import_index).ok_or((
                ValidationCode::MissingReference,
                "call import index is out of bounds".into(),
            ))?;
            let (parameters, result) = match (opcode_value, import.kind) {
                (Opcode::Call, ImportKind::Function) => {
                    let target = functions.get(&import.key).ok_or((
                        ValidationCode::MissingReference,
                        "called function does not resolve".into(),
                    ))?;
                    (
                        target
                            .parameters
                            .iter()
                            .map(|parameter| parameter.value_type)
                            .collect(),
                        target.result,
                    )
                }
                (Opcode::CallNative, ImportKind::Native) => {
                    let target = native.get(&import.key).ok_or((
                        ValidationCode::MissingReference,
                        "native import does not resolve".into(),
                    ))?;
                    (target.parameters.clone(), target.result)
                }
                (Opcode::CallHost, ImportKind::Host) => {
                    let target = host.get(&import.key).ok_or((
                        ValidationCode::MissingReference,
                        "host import does not resolve".into(),
                    ))?;
                    (target.parameters.clone(), target.result)
                }
                _ => {
                    return Err((
                        ValidationCode::InvalidOperand,
                        "call opcode does not match its import kind".into(),
                    ));
                }
            };
            if parameters.len() != declared_arguments {
                return Err((
                    ValidationCode::InvalidOperand,
                    "call argument count does not match its import".into(),
                ));
            }
            for parameter in parameters.iter().rev() {
                pop_type(stack, *parameter)?;
            }
            let encoded_result = (instruction.payload[6] != u8::MAX)
                .then(|| opcode::decode_type(instruction.payload[6]))
                .flatten();
            if encoded_result != result {
                return Err((
                    ValidationCode::TypeMismatch,
                    "call result type does not match its import".into(),
                ));
            }
            if let Some(result) = result {
                stack.push(result);
            }
        }
        Opcode::Return => {
            expect_payload(&instruction.payload, 1)?;
            if instruction.payload[0] > 1 {
                return Err((
                    ValidationCode::InvalidOperand,
                    "return flag must be zero or one".into(),
                ));
            }
            if instruction.payload[0] != 0 {
                let result = function
                    .result
                    .or_else(|| {
                        (function.kind != erabasic_bytecode::BytecodeFunctionKind::Method)
                            .then_some(BytecodeType::Integer)
                    })
                    .ok_or((
                        ValidationCode::TypeMismatch,
                        "void function returns a value".into(),
                    ))?;
                pop_type(stack, result)?;
            } else if function.result.is_some()
                && function.kind != erabasic_bytecode::BytecodeFunctionKind::Event
            {
                return Err((
                    ValidationCode::TypeMismatch,
                    "value-returning function has an empty return".into(),
                ));
            }
            if !stack.is_empty() {
                return Err((
                    ValidationCode::StackMismatch,
                    "return leaves temporary values on the stack".into(),
                ));
            }
            return Ok(Vec::new());
        }
        Opcode::AwaitResume => {
            expect_payload(&instruction.payload, 1)?;
            stack.push(
                opcode::decode_type(instruction.payload[0])
                    .ok_or((ValidationCode::InvalidOperand, "invalid resume type".into()))?,
            );
        }
        Opcode::Trap => {
            if std::str::from_utf8(&instruction.payload).is_err() {
                return Err((
                    ValidationCode::InvalidOperand,
                    "trap message is not UTF-8".into(),
                ));
            }
            return Ok(Vec::new());
        }
    }
    Ok(next())
}

fn expect_payload(payload: &[u8], length: usize) -> Result<(), (ValidationCode, String)> {
    if payload.len() == length {
        Ok(())
    } else {
        Err((
            ValidationCode::InvalidOperand,
            format!("expected {length} payload bytes, found {}", payload.len()),
        ))
    }
}

fn read_u16(payload: &[u8], offset: usize) -> Result<u16, (ValidationCode, String)> {
    Ok(u16::from_le_bytes(
        payload
            .get(offset..offset + 2)
            .ok_or((
                ValidationCode::InvalidOperand,
                "truncated u16 operand".into(),
            ))?
            .try_into()
            .expect("two-byte slice"),
    ))
}

fn read_u32(payload: &[u8], offset: usize) -> Result<u32, (ValidationCode, String)> {
    Ok(u32::from_le_bytes(
        payload
            .get(offset..offset + 4)
            .ok_or((
                ValidationCode::InvalidOperand,
                "truncated u32 operand".into(),
            ))?
            .try_into()
            .expect("four-byte slice"),
    ))
}

fn read_key(payload: &[u8]) -> Result<SymbolKey, (ValidationCode, String)> {
    let mut key = [0; 16];
    key.copy_from_slice(payload.get(..16).ok_or((
        ValidationCode::InvalidOperand,
        "truncated symbol key".into(),
    ))?);
    Ok(SymbolKey(key))
}

fn pop_type(
    stack: &mut Vec<BytecodeType>,
    expected: BytecodeType,
) -> Result<(), (ValidationCode, String)> {
    let actual = stack.pop().ok_or((
        ValidationCode::StackMismatch,
        "instruction underflows the stack".into(),
    ))?;
    if actual == expected {
        Ok(())
    } else {
        Err((
            ValidationCode::TypeMismatch,
            format!("expected {expected:?}, found {actual:?}"),
        ))
    }
}
