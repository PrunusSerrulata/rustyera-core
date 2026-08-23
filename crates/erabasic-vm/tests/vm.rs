use erabasic_analyzer::{
    AnalysisInput, AnalyzerOptions, ExtensionRegistry, ProjectSource, SourcePayload,
    analyze_project,
};
use erabasic_bytecode::{
    ArtifactManifest, BytecodeArtifact, BytecodeFunction, BytecodeGlobal, BytecodePersistence,
    BytecodeStorage, BytecodeType, CapabilityFallback, Digest, FunctionImport, HostCapability,
    HostImport, HostSnapshotCapability, ImportKind, Opcode, OperationContract,
    OperationDebugPolicy, OperationHotReloadPolicy, OperationPersistence, OperationSnapshotPolicy,
    OperationState, OperationWaitPolicy, RuntimeImport, SourceMap, SourceMapEntry, SourceRecord,
    SymbolKey, TransactionPolicy, create_patch, opcode,
};
use erabasic_compiler::{CompilerOptions, compile_project, default_host_registry};
use erabasic_csv::{
    CsvLoadOptions, FilePayload as CsvFilePayload, FrontendFile, ProjectFiles, load_project,
};
use erabasic_validator::{ValidatedArtifact, ValidationContext, validate_bytecode};
use erabasic_vm::{
    EraSaveScope, FiberId, FiberStatus, HostCallRequest, HostCallResult, HostReady,
    HostRebindRequest, HostWaitStability, NativeServiceRegistry, RunBudget, RuntimeVm,
    SnapshotBlocker, SnapshotEligibility, Vm, VmBreakpoint, VmBreakpointLocation, VmConfig,
    VmDebugControl, VmDebugInspect, VmDebugVariableWrite, VmDriveMode, VmEvent, VmFaultCode,
    VmHost, VmPreparationStage, VmRuntimeFill, VmRuntimePort, VmRuntimeStatePort,
    VmRuntimeStateTransaction, VmSnapshot, VmStepKind, VmValue, inspect_snapshot,
};
use unicode_width::UnicodeWidthStr;

fn project_data() -> erabasic_data::ProjectData {
    load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .expect("default project data should load")
}

fn function(
    key: SymbolKey,
    name: &str,
    code: Vec<erabasic_bytecode::EncodedInstruction>,
) -> BytecodeFunction {
    BytecodeFunction {
        key,
        name: name.into(),
        kind: erabasic_bytecode::BytecodeFunctionKind::Normal,
        parameters: Vec::new(),
        result: None,
        labels: Vec::new(),
        imports: Vec::new(),
        max_stack: 16,
        code,
    }
}

fn artifact(functions: Vec<BytecodeFunction>, globals: Vec<BytecodeGlobal>) -> BytecodeArtifact {
    let mut artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        call_compatibility: erabasic_bytecode::BytecodeCallCompatibility::default(),
        project_data: project_data(),
        globals,
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions,
        event_groups: Vec::new(),
        source_map: SourceMap::default(),
    };
    artifact.refresh_ids().unwrap();
    artifact
}

fn validated(artifact: &BytecodeArtifact) -> ValidatedArtifact {
    let report = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &ValidationContext::for_artifact(artifact),
    );
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    report.value.expect("artifact should validate")
}

fn global(key: SymbolKey, dimensions: Vec<u64>) -> BytecodeGlobal {
    BytecodeGlobal {
        key,
        name: "VALUE".into(),
        value_type: BytecodeType::Integer,
        dimensions,
        mutable: true,
        storage: BytecodeStorage::Project,
        persistence: BytecodePersistence::GameSave,
        initial_values: Vec::new(),
        owner: None,
    }
}

fn compile_source(source: &str) -> BytecodeArtifact {
    compile_source_with_data(source, project_data())
}

fn compile_source_with_options(source: &str, options: &AnalyzerOptions) -> BytecodeArtifact {
    let analysis = analyze_project(
        AnalysisInput {
            project_data: project_data(),
            sources: vec![ProjectSource {
                relative_path: "main.erb".into(),
                payload: SourcePayload::Utf8(source.into()),
            }],
        },
        options,
        &ExtensionRegistry::default(),
    );
    assert!(analysis.project.is_some(), "{:#?}", analysis.diagnostics);
    let compilation = compile_project(
        &analysis.project.unwrap(),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(
        compilation.artifact.is_some(),
        "{:#?}",
        compilation.diagnostics
    );
    compilation.artifact.unwrap()
}

fn compile_source_with_data(
    source: &str,
    project_data: erabasic_data::ProjectData,
) -> BytecodeArtifact {
    let analysis = analyze_project(
        AnalysisInput {
            project_data,
            sources: vec![ProjectSource {
                relative_path: "main.erb".into(),
                payload: SourcePayload::Utf8(source.into()),
            }],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(analysis.project.is_some(), "{:#?}", analysis.diagnostics);
    let analysis_diagnostics = analysis.diagnostics.clone();
    let compilation = compile_project(
        &analysis.project.unwrap(),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(
        compilation.artifact.is_some(),
        "analysis: {analysis_diagnostics:#?}\ncompilation: {:#?}",
        compilation.diagnostics,
    );
    compilation.artifact.unwrap()
}

fn run_compiled_result(artifact: &BytecodeArtifact) -> VmValue {
    run_compiled_result_with_report(artifact).0
}

fn run_compiled_result_with_report(
    artifact: &BytecodeArtifact,
) -> (VmValue, erabasic_vm::VmRunReport) {
    run_compiled_entry_result_with_report(artifact, "SYSTEM_TITLE", 0)
}

fn run_compiled_entry_result_with_report(
    artifact: &BytecodeArtifact,
    entry_name: &str,
    index: u64,
) -> (VmValue, erabasic_vm::VmRunReport) {
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == entry_name)
        .unwrap_or_else(|| panic!("missing entry {entry_name}"))
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(artifact);
    let mut vm = Vm::new(validated(artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
    (vm.read_variable(result, &[index], None).unwrap(), report)
}

fn run_compiled_string_result(artifact: &BytecodeArtifact) -> VmValue {
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .expect("RESULTS")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(artifact);
    let mut vm = Vm::new(validated(artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
    vm.read_variable(results, &[0], None).unwrap()
}

#[derive(Default)]
struct ReadyHost {
    calls: Vec<i64>,
    strings: Vec<String>,
}

impl VmHost for ReadyHost {
    fn call(&mut self, request: HostCallRequest) -> HostCallResult {
        if let Some(VmValue::Integer(value)) = request.arguments.first() {
            self.calls.push(*value);
        } else if let Some(VmValue::String(value)) = request.arguments.first() {
            self.strings.push(value.clone());
        }
        let value = request.import.result.map(|result| match result {
            BytecodeType::Integer => VmValue::Integer(0),
            BytecodeType::String => VmValue::String(String::new()),
            BytecodeType::IntegerPlace | BytecodeType::StringPlace => {
                panic!("test host cannot synthesize a place result")
            }
        });
        HostCallResult::Ready(HostReady {
            value,
            writes: Vec::new(),
        })
    }
}

fn host_artifact(stability: HostSnapshotCapability) -> (BytecodeArtifact, SymbolKey) {
    let entry = SymbolKey::derive("test.function", b"host");
    let import_key = SymbolKey::derive("test.host", b"print");
    let runtime_import = RuntimeImport {
        key: import_key,
        namespace: "test".into(),
        name: "operation".into(),
        abi_version: 1,
        parameters: vec![BytecodeType::Integer],
        result: None,
    };
    let mut function = function(
        entry,
        "HOST",
        vec![
            opcode::push_integer(7),
            opcode::call(Opcode::CallHost, 0, 1, None),
            opcode::return_value(false),
        ],
    );
    function.imports.push(FunctionImport {
        kind: ImportKind::Host,
        key: import_key,
    });
    let mut artifact = artifact(vec![function], Vec::new());
    let contract = OperationContract {
        state: OperationState::Controller,
        transaction: TransactionPolicy::Forbidden,
        candidate: erabasic_bytecode::CandidatePolicy::Forbidden,
        persistence: OperationPersistence::RuntimeOnly,
        snapshot: if stability == HostSnapshotCapability::StableWait {
            OperationSnapshotPolicy::Included
        } else {
            OperationSnapshotPolicy::PendingBlocks
        },
        hot_reload: if stability == HostSnapshotCapability::StableWait {
            OperationHotReloadPolicy::Preserve
        } else {
            OperationHotReloadPolicy::ActiveBlocks
        },
        wait: if stability == HostSnapshotCapability::StableWait {
            OperationWaitPolicy::StableInput
        } else {
            OperationWaitPolicy::TransientExternal
        },
        capability_fallback: CapabilityFallback::ScriptResult,
        debug: OperationDebugPolicy::Forbidden,
        portability: erabasic_bytecode::OperationPortability::Portable,
    };
    artifact.host_imports.push(HostImport {
        import: runtime_import,
        effect: contract.effect(),
        capability: HostCapability::Input,
        snapshot_capability: stability,
        contract,
    });
    artifact.refresh_ids().unwrap();
    (artifact, entry)
}

struct PendingHost {
    stability: HostWaitStability,
    rebound: Vec<HostRebindRequest>,
}

impl VmHost for PendingHost {
    fn call(&mut self, _request: HostCallRequest) -> HostCallResult {
        HostCallResult::Pending {
            stability: self.stability,
            rebind_payload: b"input-line".to_vec(),
        }
    }

    fn rebind_snapshot(&mut self, requests: &[HostRebindRequest]) -> Result<(), String> {
        self.rebound = requests.to_vec();
        Ok(())
    }
}

#[path = "vm/arrays.rs"]
mod arrays;
#[path = "vm/characters.rs"]
mod characters;
#[path = "vm/core.rs"]
mod core;
#[path = "vm/debug_snapshot.rs"]
mod debug_snapshot;
#[path = "vm/replace.rs"]
mod replace;
#[path = "vm/runtime.rs"]
mod runtime;
#[path = "vm/strform.rs"]
mod strform;
