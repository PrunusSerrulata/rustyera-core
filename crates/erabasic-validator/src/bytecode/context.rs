use std::collections::{BTreeMap, BTreeSet};

use erabasic_bytecode::{
    BytecodeArtifact, COMPILER_ABI_VERSION, CONTAINER_VERSION, FormatVersion, HOST_ABI_VERSION,
    HostCapability, HostImport, ISA_VERSION, NATIVE_ABI_VERSION, NativeImport, SymbolKey,
    VM_ABI_VERSION,
};

use crate::ValidationLimits;

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
            runtime_native_authorizations: artifact
                .runtime_native_authorizations
                .iter()
                .cloned()
                .map(|authorization| (authorization.key, authorization))
                .collect(),
            runtime_host_authorizations: artifact
                .runtime_host_authorizations
                .iter()
                .cloned()
                .map(|authorization| (authorization.key, authorization))
                .collect(),
            runtime_staged_authorizations: artifact
                .runtime_staged_authorizations
                .iter()
                .cloned()
                .map(|authorization| (authorization.key, authorization))
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
                .chain(
                    artifact
                        .runtime_host_authorizations
                        .iter()
                        .flat_map(|family| {
                            std::iter::once(&family.prototype)
                                .chain(family.stages.iter().map(|(_, import)| import))
                                .map(|import| import.capability)
                        }),
                )
                .collect(),
            ..Self::default()
        }
    }
}
