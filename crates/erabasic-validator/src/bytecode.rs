mod host_authorization;
mod instructions;
mod native_authorization;
mod provenance;
mod runtime_symbols;
mod source_map;
mod staged_authorization;

pub use provenance::{ValidatedOperandStacks, ValidatedStackState, ValidatedStackToken};

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use erabasic_bytecode::{
    BytecodeArtifact, BytecodeConstant, BytecodeFunction, BytecodeStorage, BytecodeType,
    COMPILER_ABI_VERSION, CONTAINER_VERSION, FormatVersion, HOST_ABI_VERSION, HostCapability,
    HostImport, ISA_VERSION, NATIVE_ABI_VERSION, NativeImport, SymbolKey, UnvalidatedArtifact,
    VM_ABI_VERSION,
};
use rayon::prelude::*;

use crate::{ValidationCode, ValidationDiagnostic, ValidationLimits, ValidationReport};
use source_map::validate_source_map;

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
    pub runtime_native_authorizations:
        BTreeMap<SymbolKey, erabasic_bytecode::RuntimeNativeAuthorization>,
    pub runtime_host_authorizations:
        BTreeMap<SymbolKey, erabasic_bytecode::RuntimeHostAuthorization>,
    pub runtime_staged_authorizations:
        BTreeMap<SymbolKey, erabasic_bytecode::RuntimeStagedAuthorization>,
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
            runtime_native_authorizations: BTreeMap::new(),
            runtime_host_authorizations: BTreeMap::new(),
            runtime_staged_authorizations: BTreeMap::new(),
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
pub struct ValidatedArtifact(Arc<BytecodeArtifact>, Arc<ValidatedOperandStacks>);

impl ValidatedArtifact {
    /// Control-flow stack provenance produced by the same successful validation pass.
    /// This data is never read from the artifact or snapshot payload.
    #[must_use]
    pub fn operand_stacks(&self) -> &ValidatedOperandStacks {
        &self.1
    }

    /// Refresh identities on a compiler artifact without losing its validation provenance.
    ///
    /// Identity refresh canonicalizes ordering and changes only manifest identities,
    /// preserving the structural invariants established by validation.
    ///
    /// # Errors
    ///
    /// Returns an error if a canonical identity section cannot be encoded.
    pub fn refresh_ids(mut self) -> Result<Self, String> {
        Arc::make_mut(&mut self.0)
            .refresh_ids()
            .map_err(|error| error.to_string())?;
        Ok(self)
    }

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
    native_authorization::validate(&artifact, context, &mut diagnostics);
    host_authorization::validate(&artifact, context, &mut diagnostics);
    staged_authorization::validate(&artifact, context, &mut diagnostics);
    if let Err(message) = runtime_symbols::validate_runtime_builtins(&artifact.runtime_builtins) {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::InvalidOperand,
            message,
        ));
    }
    if let Err(message) = runtime_symbols::validate_runtime_variables(&artifact) {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::InvalidOperand,
            message,
        ));
    }
    validate_source_map(&artifact, &mut diagnostics);
    let operand_stacks = validate_functions(&artifact, context, &mut diagnostics);
    ValidationReport {
        value: diagnostics.is_empty().then_some(ValidatedArtifact(
            Arc::new(artifact),
            Arc::new(operand_stacks),
        )),
        diagnostics,
    }
}

fn validate_versions(
    artifact: &BytecodeArtifact,
    context: &ValidationContext,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let manifest = &artifact.manifest;
    if let Err(error) = manifest.compatibility.validate() {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::UnsupportedVersion,
            format!("unsupported artifact compatibility: {error}"),
        ));
    }
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

fn validate_functions(
    artifact: &BytecodeArtifact,
    context: &ValidationContext,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) -> ValidatedOperandStacks {
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
    let staged: BTreeMap<_, _> = artifact
        .runtime_staged_authorizations
        .iter()
        .map(|authorization| (authorization.key, authorization))
        .collect();
    let function_diagnostics = artifact
        .functions
        .par_iter()
        .map(|function| {
            let mut diagnostics = Vec::new();
            let stacks = validate_function(
                function,
                &globals,
                &functions,
                &native,
                &host,
                &staged,
                context,
                &mut diagnostics,
            );
            (function.key, stacks, diagnostics)
        })
        .collect::<Vec<_>>();
    let mut stacks = BTreeMap::new();
    for (function, provenance, mut function_diagnostics) in function_diagnostics {
        diagnostics.append(&mut function_diagnostics);
        stacks.insert(function, provenance);
    }
    ValidatedOperandStacks::new(stacks)
}

fn validate_function_header(
    function: &BytecodeFunction,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) -> Option<instructions::ProbeIndex> {
    if function.code.is_empty() {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::InvalidControlFlow,
            format!("function {} has no instructions", function.name),
        ));
        return None;
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
        return None;
    }
    match instructions::ProbeIndex::new(function) {
        Ok(probes) => Some(probes),
        Err((index, (code, message))) => {
            diagnostics.push(ValidationDiagnostic::instruction(
                code,
                &function.name,
                index,
                message,
            ));
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_function(
    function: &BytecodeFunction,
    globals: &BTreeMap<SymbolKey, &erabasic_bytecode::BytecodeGlobal>,
    functions: &BTreeMap<SymbolKey, &BytecodeFunction>,
    native: &BTreeMap<SymbolKey, &erabasic_bytecode::RuntimeImport>,
    host: &BTreeMap<SymbolKey, &erabasic_bytecode::RuntimeImport>,
    staged: &BTreeMap<SymbolKey, &erabasic_bytecode::RuntimeStagedAuthorization>,
    context: &ValidationContext,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) -> provenance::FunctionStackProvenance {
    let Some(probes) = validate_function_header(function, diagnostics) else {
        return provenance::FunctionStackProvenance::default();
    };
    let mut states = vec![None; function.code.len()];
    states[0] = Some(Vec::<instructions::StackValue>::new());
    let mut work = VecDeque::from([0usize]);
    let mut observed_max = 0usize;
    let mut terminal_user_calls = BTreeMap::new();
    while let Some(index) = work.pop_front() {
        let Some(mut stack) = states[index].clone() else {
            continue;
        };
        let successors = match apply_instruction(
            function,
            index,
            &mut stack,
            globals,
            functions,
            native,
            host,
            staged,
            &context.runtime_staged_authorizations,
            &probes,
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
        if successors.is_empty()
            && function.code[index].opcode == erabasic_bytecode::Opcode::InvokeUserCall as u16
        {
            terminal_user_calls.insert(index, ValidatedStackState::from_stack(&stack));
        }
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
    provenance::FunctionStackProvenance::new(states, terminal_user_calls)
}

use instructions::apply_instruction;
