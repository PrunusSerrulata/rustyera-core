use erabasic_bytecode::{
    CandidatePolicy, CapabilityFallback, OperationContract, OperationDebugPolicy,
    OperationHotReloadPolicy, OperationPersistence, OperationSnapshotPolicy, OperationState,
    OperationWaitPolicy, TransactionPolicy,
};

pub(super) fn native_contract(name: &str) -> OperationContract {
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
        portability: erabasic_bytecode::OperationPortability::Portable,
    }
}

#[allow(clippy::too_many_lines)]
pub(super) fn host_contract(namespace: &str, name: &str) -> OperationContract {
    let (state, transaction, persistence, snapshot, hot_reload, wait, fallback) = match namespace {
        "rustyera.text"
            if matches!(
                name,
                "GETDISPLAYLINE"
                    | "HTML_GETPRINTEDSTR"
                    | "HTML_STRINGLEN"
                    | "HTML_SUBSTRING"
                    | "HTML_STRINGLINES"
            ) =>
        {
            (
                OperationState::External,
                TransactionPolicy::Forbidden,
                OperationPersistence::RuntimeOnly,
                OperationSnapshotPolicy::PendingBlocks,
                OperationHotReloadPolicy::ActiveBlocks,
                OperationWaitPolicy::TransientExternal,
                CapabilityFallback::Unsupported,
            )
        }
        "rustyera.text"
            if matches!(name, "BARSTR" | "MONEYSTR" | "TOSTR" | "TOFULL" | "TOHALF") =>
        {
            (
                OperationState::Controller,
                TransactionPolicy::ReadOnly,
                OperationPersistence::ProjectDerived,
                OperationSnapshotPolicy::Included,
                OperationHotReloadPolicy::Rebuild,
                OperationWaitPolicy::Immediate,
                CapabilityFallback::CanonicalProjection,
            )
        }
        "rustyera.text" => (
            OperationState::Presentation,
            TransactionPolicy::CloneCommit,
            OperationPersistence::RuntimeOnly,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Preserve,
            OperationWaitPolicy::Immediate,
            CapabilityFallback::CanonicalProjection,
        ),
        "rustyera.audio" => (
            OperationState::Presentation,
            TransactionPolicy::BufferedEffect,
            OperationPersistence::RuntimeOnly,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Rebuild,
            OperationWaitPolicy::Immediate,
            CapabilityFallback::IntentNoOp,
        ),
        "rustyera.graphics"
            if matches!(
                name,
                "GLOAD" | "GSAVE" | "GCREATEFROMFILE" | "GGETTEXTSIZE" | "GGETCOLOR"
            ) =>
        {
            (
                OperationState::External,
                TransactionPolicy::Forbidden,
                OperationPersistence::ProjectDerived,
                OperationSnapshotPolicy::PendingBlocks,
                OperationHotReloadPolicy::ActiveBlocks,
                OperationWaitPolicy::TransientExternal,
                CapabilityFallback::Unsupported,
            )
        }
        "rustyera.graphics" => (
            OperationState::Presentation,
            TransactionPolicy::CloneCommit,
            OperationPersistence::RuntimeOnly,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Rebuild,
            OperationWaitPolicy::Immediate,
            CapabilityFallback::ScriptResult,
        ),
        "rustyera.input"
            if matches!(
                name,
                "GETKEY" | "GETKEYTRIGGERED" | "MOUSEX" | "MOUSEY" | "MOUSEB" | "AWAIT"
            ) =>
        {
            (
                OperationState::Controller,
                TransactionPolicy::Forbidden,
                OperationPersistence::RuntimeOnly,
                OperationSnapshotPolicy::PendingBlocks,
                OperationHotReloadPolicy::ActiveBlocks,
                OperationWaitPolicy::TransientExternal,
                CapabilityFallback::ScriptResult,
            )
        }
        "rustyera.input"
            if matches!(
                name,
                "GETTEXTBOX"
                    | "SETTEXTBOX"
                    | "CLEARTEXTBOX"
                    | "HOTKEY_STATE"
                    | "HOTKEY_STATE_INIT"
                    | "FLOWINPUT"
                    | "FLOWINPUTS"
                    | "BREAKBUTTON"
                    | "ISACTIVE"
            ) =>
        {
            (
                OperationState::Controller,
                TransactionPolicy::Forbidden,
                OperationPersistence::RuntimeOnly,
                OperationSnapshotPolicy::Included,
                OperationHotReloadPolicy::Preserve,
                OperationWaitPolicy::Immediate,
                CapabilityFallback::ScriptResult,
            )
        }
        "rustyera.input" => (
            OperationState::Controller,
            TransactionPolicy::Forbidden,
            OperationPersistence::RuntimeOnly,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Preserve,
            OperationWaitPolicy::StableInput,
            CapabilityFallback::ScriptResult,
        ),
        "rustyera.clock" | "rustyera.network" => (
            OperationState::External,
            TransactionPolicy::Forbidden,
            OperationPersistence::None,
            OperationSnapshotPolicy::PendingBlocks,
            OperationHotReloadPolicy::ActiveBlocks,
            OperationWaitPolicy::TransientExternal,
            CapabilityFallback::ScriptResult,
        ),
        "rustyera.storage" if name == "PUTFORM" => (
            OperationState::Vm,
            TransactionPolicy::CloneCommit,
            OperationPersistence::Ordinary,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Preserve,
            OperationWaitPolicy::Immediate,
            CapabilityFallback::NotApplicable,
        ),
        "rustyera.storage" if name == "SAVENOS" => (
            OperationState::Vm,
            TransactionPolicy::ReadOnly,
            OperationPersistence::None,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Preserve,
            OperationWaitPolicy::Immediate,
            CapabilityFallback::NotApplicable,
        ),
        "rustyera.storage" => (
            OperationState::External,
            TransactionPolicy::Forbidden,
            OperationPersistence::Ordinary,
            OperationSnapshotPolicy::PendingBlocks,
            OperationHotReloadPolicy::ActiveBlocks,
            OperationWaitPolicy::TransientExternal,
            CapabilityFallback::Unsupported,
        ),
        "rustyera.system" if matches!(name, "SAVEGAME" | "LOADGAME") => (
            OperationState::Controller,
            TransactionPolicy::Forbidden,
            OperationPersistence::RuntimeOnly,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Preserve,
            OperationWaitPolicy::StableInput,
            CapabilityFallback::NotApplicable,
        ),
        "rustyera.system"
            if matches!(
                name,
                "ENUMFUNCBEGINSWITH"
                    | "ENUMFUNCENDSWITH"
                    | "ENUMFUNCWITH"
                    | "ENUMVARBEGINSWITH"
                    | "ENUMVARENDSWITH"
                    | "ENUMVARWITH"
            ) =>
        {
            (
                OperationState::Vm,
                TransactionPolicy::CloneCommit,
                OperationPersistence::VariableScoped,
                OperationSnapshotPolicy::Included,
                OperationHotReloadPolicy::Preserve,
                OperationWaitPolicy::Immediate,
                CapabilityFallback::NotApplicable,
            )
        }
        "rustyera.system"
            if matches!(
                name,
                "GETCONFIG"
                    | "GETCONFIGS"
                    | "VARSIZE"
                    | "EXISTFUNCTION"
                    | "EXISTVAR"
                    | "GETDOINGFUNCTION"
            ) =>
        {
            (
                OperationState::Controller,
                TransactionPolicy::ReadOnly,
                OperationPersistence::ProjectDerived,
                OperationSnapshotPolicy::Included,
                OperationHotReloadPolicy::Rebuild,
                OperationWaitPolicy::Immediate,
                CapabilityFallback::CanonicalProjection,
            )
        }
        "rustyera.system" => (
            OperationState::Controller,
            TransactionPolicy::CloneCommit,
            OperationPersistence::RuntimeOnly,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Preserve,
            OperationWaitPolicy::Immediate,
            CapabilityFallback::NotApplicable,
        ),
        _ => (
            OperationState::External,
            TransactionPolicy::Forbidden,
            OperationPersistence::RuntimeOnly,
            OperationSnapshotPolicy::PendingBlocks,
            OperationHotReloadPolicy::ActiveBlocks,
            OperationWaitPolicy::TransientExternal,
            CapabilityFallback::Unsupported,
        ),
    };
    OperationContract {
        state,
        transaction,
        candidate: match (namespace, wait, transaction) {
            ("rustyera.clock", _, TransactionPolicy::Forbidden) => CandidatePolicy::FrozenClock,
            (_, OperationWaitPolicy::StableInput | OperationWaitPolicy::TransientExternal, _) => {
                CandidatePolicy::Forbidden
            }
            (_, _, TransactionPolicy::ReadOnly) => CandidatePolicy::ReadOnly,
            (_, _, TransactionPolicy::CloneCommit) => CandidatePolicy::CloneCommit,
            (_, _, TransactionPolicy::BufferedEffect) => CandidatePolicy::BufferedEffect,
            (_, _, TransactionPolicy::Forbidden) => CandidatePolicy::Forbidden,
        },
        persistence,
        snapshot,
        hot_reload,
        wait,
        capability_fallback: fallback,
        // The debugger deliberately rejects every Host import, including reference
        // METHOD_SAFE printing and media commands.
        debug: OperationDebugPolicy::Forbidden,
        portability: if erabasic_analyzer::builtin_callable_portability(name)
            == erabasic_analyzer::CallablePortability::FrontendObservation
        {
            erabasic_bytecode::OperationPortability::FrontendObservation
        } else if matches!(namespace, "rustyera.audio" | "rustyera.network") {
            erabasic_bytecode::OperationPortability::PlatformIntent
        } else {
            erabasic_bytecode::OperationPortability::Portable
        },
    }
}
