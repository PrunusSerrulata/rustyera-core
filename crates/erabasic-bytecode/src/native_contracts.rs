use crate::{
    CandidatePolicy, CapabilityFallback, OperationContract, OperationDebugPolicy,
    OperationHotReloadPolicy, OperationPersistence, OperationSnapshotPolicy, OperationState,
    OperationWaitPolicy, TransactionPolicy,
};

#[must_use]
pub fn canonical_native_contract(name: &str) -> OperationContract {
    let name = name.to_ascii_lowercase();
    let structured =
        name.starts_with("map_") || name.starts_with("xml_") || name.starts_with("dt_");
    let random = matches!(
        name.as_str(),
        "rand" | "randomize" | "initrand" | "dumprand"
    );
    let variable_read = matches!(name.as_str(), "getvar" | "getvars" | "existmeth");
    let variable_mutation = matches!(
        name.as_str(),
        "swap"
            | "swapvar"
            | "arrayremove"
            | "arrayshift"
            | "arraysort"
            | "arraycopy"
            | "setvar"
            | "varset"
            | "cvarset"
            | "arraymsort"
            | "arraymsortex"
            | "addchara"
            | "addspchara"
            | "adddefchara"
            | "addvoidchara"
            | "delchara"
            | "delallchara"
            | "swapchara"
            | "copychara"
            | "addcopychara"
            | "pickupchara"
            | "sortchara"
            | "reset_stain"
            | "setbit"
            | "clearbit"
            | "invertbit"
            | "split"
            | "__encodetouni_result"
    );
    let mutable = structured || random || variable_mutation;
    OperationContract {
        state: if structured || random {
            OperationState::Native
        } else if variable_mutation || variable_read {
            OperationState::Vm
        } else {
            OperationState::Pure
        },
        transaction: if mutable {
            TransactionPolicy::CloneCommit
        } else {
            TransactionPolicy::ReadOnly
        },
        candidate: if mutable {
            CandidatePolicy::CloneCommit
        } else {
            CandidatePolicy::ReadOnly
        },
        persistence: if structured {
            OperationPersistence::ExtensionScoped
        } else if variable_mutation {
            OperationPersistence::VariableScoped
        } else if random {
            OperationPersistence::RuntimeOnly
        } else {
            OperationPersistence::None
        },
        snapshot: OperationSnapshotPolicy::Included,
        hot_reload: OperationHotReloadPolicy::Preserve,
        wait: OperationWaitPolicy::Immediate,
        capability_fallback: CapabilityFallback::NotApplicable,
        debug: if name == "existmeth" {
            OperationDebugPolicy::Forbidden
        } else if mutable {
            OperationDebugPolicy::Transactional
        } else {
            OperationDebugPolicy::Pure
        },
        portability: crate::OperationPortability::Portable,
    }
}
