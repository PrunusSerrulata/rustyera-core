use std::collections::BTreeMap;

use erabasic_analyzer::{builtin_function_names, builtin_instruction_names};
use erabasic_bytecode::{
    CandidatePolicy, CapabilityFallback, HostCapability, HostEffect, HostSnapshotCapability,
    MethodResult, OperationContract, OperationDebugPolicy, OperationHotReloadPolicy,
    OperationPersistence, OperationSnapshotPolicy, OperationState, OperationWaitPolicy,
    TransactionPolicy,
};
use serde::{Deserialize, Serialize};

mod contracts;
mod hosts;

use contracts::{host_contract, native_contract};
use hosts::{AUDIO, CLOCK, GRAPHICS, INPUT, NETWORK, STORAGE, SYSTEM, TEXT, register_hosts};

pub(crate) fn column_options_contract() -> OperationContract {
    let mut contract = native_contract("dt_column_options");
    contract.debug = OperationDebugPolicy::Forbidden;
    contract
}

pub(crate) fn html_query_binding(name: &str) -> HostBinding {
    // Only specialized lowering emits these imports; they are not catalog callables.
    let contract = host_contract("rustyera.text", "HTML_STRINGLINES");
    HostBinding {
        namespace: "rustyera.text".into(),
        name: name.to_ascii_lowercase(),
        abi_version: 1,
        effect: contract.effect(),
        capability: HostCapability::Text,
        snapshot_capability: contract.snapshot_capability(),
        contract,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostBinding {
    pub namespace: String,
    pub name: String,
    pub abi_version: u32,
    pub effect: HostEffect,
    pub capability: HostCapability,
    pub snapshot_capability: HostSnapshotCapability,
    pub contract: OperationContract,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ExecutionBinding {
    Native(OperationContract),
    ExpressionMethod { result: MethodResult },
    Host(HostBinding),
    Unsupported { reason: String },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostRegistry {
    bindings: BTreeMap<String, ExecutionBinding>,
}

impl HostRegistry {
    pub fn register(&mut self, era_name: impl Into<String>, binding: HostBinding) -> bool {
        self.bindings.insert(
            era_name.into().to_ascii_uppercase(),
            ExecutionBinding::Host(binding),
        );
        true
    }

    pub fn register_execution(
        &mut self,
        era_name: impl Into<String>,
        binding: ExecutionBinding,
    ) -> bool {
        self.bindings
            .insert(era_name.into().to_ascii_uppercase(), binding)
            .is_none()
    }

    #[must_use]
    pub fn classification(&self, era_name: &str) -> Option<&ExecutionBinding> {
        self.bindings.get(era_name).or_else(|| {
            era_name
                .bytes()
                .any(|byte| byte.is_ascii_lowercase())
                .then(|| era_name.to_ascii_uppercase())
                .and_then(|name| self.bindings.get(&name))
        })
    }

    #[must_use]
    pub fn resolve(&self, era_name: &str) -> Option<HostBinding> {
        match self.classification(era_name) {
            Some(ExecutionBinding::Host(binding)) => Some(binding.clone()),
            Some(
                ExecutionBinding::Native(_)
                | ExecutionBinding::ExpressionMethod { .. }
                | ExecutionBinding::Unsupported { .. },
            )
            | None => None,
        }
    }
}

#[must_use]
pub fn default_host_registry() -> HostRegistry {
    let mut registry = HostRegistry::default();
    for name in builtin_instruction_names()
        .into_iter()
        .chain(builtin_function_names())
    {
        let binding = if matches!(name.as_str(), "GETMETH" | "GETMETHS") {
            ExecutionBinding::ExpressionMethod {
                result: if name == "GETMETHS" {
                    MethodResult::String
                } else {
                    MethodResult::Integer
                },
            }
        } else if native_is_implemented(&name) {
            ExecutionBinding::Native(native_contract(&name))
        } else {
            ExecutionBinding::Unsupported {
                reason: "the pinned runtime has no classified implementation for this built-in"
                    .into(),
            }
        };
        registry.bindings.entry(name).or_insert(binding);
    }
    registry.register_execution(
        "__INDEXBYNAME",
        ExecutionBinding::Native(native_contract("__INDEXBYNAME")),
    );
    registry.register_execution(
        "__ENCODETOUNI_RESULT",
        ExecutionBinding::Native(native_contract("__ENCODETOUNI_RESULT")),
    );

    register_hosts(
        &mut registry,
        INPUT,
        "rustyera.input",
        HostCapability::Input,
        true,
    );
    register_hosts(
        &mut registry,
        TEXT,
        "rustyera.text",
        HostCapability::Text,
        false,
    );
    register_hosts(
        &mut registry,
        CLOCK,
        "rustyera.clock",
        HostCapability::Clock,
        true,
    );
    register_hosts(
        &mut registry,
        GRAPHICS,
        "rustyera.graphics",
        HostCapability::Graphics,
        true,
    );
    register_hosts(
        &mut registry,
        AUDIO,
        "rustyera.audio",
        HostCapability::Audio,
        true,
    );
    register_hosts(
        &mut registry,
        STORAGE,
        "rustyera.storage",
        HostCapability::Storage,
        true,
    );
    register_hosts(
        &mut registry,
        SYSTEM,
        "rustyera.system",
        HostCapability::System,
        true,
    );
    register_hosts(
        &mut registry,
        NETWORK,
        "rustyera.network",
        HostCapability::Network,
        true,
    );
    // Preserve CALLSHARP as an external extension intent without embedding the
    // reference runtime's CLR plugin loader. The raw call expression is the
    // single ABI argument, so frontends can provide an explicit adapter.
    registry.register_execution(
        "CALLSHARP",
        ExecutionBinding::Host(extension_binding("CALLSHARP")),
    );
    registry
}

fn native_is_implemented(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("map_")
        || name.starts_with("xml_")
        || name.starts_with("dt_")
        || IMPLEMENTED_NATIVE_NAMES.contains(&name.as_str())
}

const IMPLEMENTED_NATIVE_NAMES: &[&str] = &[
    "abs",
    "sign",
    "sqrt",
    "cbrt",
    "log",
    "log10",
    "exponent",
    "power",
    "getbit",
    "bitcount",
    "strlen",
    "strlenu",
    "strform",
    "strformcheck",
    "existmeth",
    "toint",
    "isnumeric",
    "unchecked_add",
    "unchecked_sub",
    "unchecked_mul",
    "unchecked_neg",
    "unicode",
    "convert",
    "color_fromrgb",
    "color_fromname",
    "max",
    "min",
    "limit",
    "inrange",
    "tostr",
    "substring",
    "substringu",
    "strfind",
    "strfindu",
    "strcount",
    "strlens",
    "strlensu",
    "getpalamlv",
    "getexplv",
    "replace",
    "escape",
    "unicodetostr",
    "encodetouni",
    "unicodebyte",
    "charatu",
    "tolower",
    "toupper",
    "rand",
    "randomize",
    "initrand",
    "dumprand",
    "swap",
    "swapvar",
    "setbit",
    "clearbit",
    "invertbit",
    "split",
    "getnum",
    "erdname",
    "strjoin",
    "arrayremove",
    "arrayshift",
    "arraysort",
    "arraycopy",
    "getvar",
    "getvars",
    "setvar",
    "varset",
    "cvarset",
    "arraymsort",
    "arraymsortex",
    "findelement",
    "findlastelement",
    "regexpmatch",
    "sumarray",
    "sumcarray",
    "maxarray",
    "maxcarray",
    "minarray",
    "mincarray",
    "match",
    "cmatch",
    "inrangearray",
    "inrangecarray",
    "groupmatch",
    "nosames",
    "allsames",
    "charanum",
    "getchara",
    "getspchara",
    "existcsv",
    "csvname",
    "csvcallname",
    "csvnickname",
    "csvmastername",
    "csvcstr",
    "csvbase",
    "csvabl",
    "csvmark",
    "csvexp",
    "csvrelation",
    "csvtalent",
    "csvcflag",
    "csvequip",
    "csvjuel",
    "findchara",
    "findlastchara",
    "addchara",
    "addspchara",
    "adddefchara",
    "addvoidchara",
    "delchara",
    "delallchara",
    "swapchara",
    "copychara",
    "addcopychara",
    "pickupchara",
    "sortchara",
    "reset_stain",
];

#[must_use]
pub fn extension_binding(name: &str) -> HostBinding {
    let contract = OperationContract {
        state: OperationState::External,
        transaction: TransactionPolicy::Forbidden,
        candidate: CandidatePolicy::Forbidden,
        persistence: OperationPersistence::RuntimeOnly,
        snapshot: OperationSnapshotPolicy::PendingBlocks,
        hot_reload: OperationHotReloadPolicy::ActiveBlocks,
        wait: OperationWaitPolicy::TransientExternal,
        capability_fallback: CapabilityFallback::Unsupported,
        debug: OperationDebugPolicy::Forbidden,
        portability: erabasic_bytecode::OperationPortability::ExtensionDefined,
    };
    HostBinding {
        namespace: "rustyera.extension".into(),
        name: name.to_ascii_lowercase(),
        abi_version: 1,
        effect: contract.effect(),
        capability: HostCapability::Extension,
        snapshot_capability: HostSnapshotCapability::Never,
        contract,
    }
}

#[cfg(test)]
mod tests {
    use super::{ExecutionBinding, default_host_registry};

    #[test]
    fn default_registry_lookup_remains_ascii_case_insensitive() {
        let registry = default_host_registry();
        assert!(matches!(
            registry.classification("print"),
            Some(ExecutionBinding::Host(_))
        ));
        assert_eq!(
            registry.classification("PRINT"),
            registry.classification("print")
        );
    }
}
