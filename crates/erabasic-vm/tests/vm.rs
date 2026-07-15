use erabasic_analyzer::{
    AnalysisInput, AnalyzerOptions, ExtensionRegistry, ProjectSource, SourcePayload,
    analyze_project,
};
use erabasic_bytecode::{
    ArtifactManifest, BytecodeArtifact, BytecodeFunction, BytecodeGlobal, BytecodePersistence,
    BytecodeStorage, BytecodeType, Digest, FunctionImport, HostCapability, HostEffect, HostImport,
    HostSnapshotCapability, ImportKind, Opcode, RuntimeImport, SourceMap, SourceMapEntry,
    SourceRecord, SymbolKey, create_patch, opcode,
};
use erabasic_compiler::{CompilerOptions, compile_project, default_host_registry};
use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};
use erabasic_validator::{ValidatedArtifact, ValidationContext, validate_bytecode};
use erabasic_vm::{
    FiberStatus, HostCallRequest, HostCallResult, HostReady, HostRebindRequest, HostWaitStability,
    NativeServiceRegistry, RunBudget, SnapshotBlocker, SnapshotEligibility, Vm, VmConfig, VmEvent,
    VmFaultCode, VmHost, VmSnapshot, VmValue,
};

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
        parameters: Vec::new(),
        result: None,
        imports: Vec::new(),
        max_stack: 16,
        code,
    }
}

fn artifact(functions: Vec<BytecodeFunction>, globals: Vec<BytecodeGlobal>) -> BytecodeArtifact {
    let mut artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        project_data: project_data(),
        globals,
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions,
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
    let analysis = analyze_project(
        AnalysisInput {
            project_data: project_data(),
            sources: vec![ProjectSource {
                relative_path: "main.erb".into(),
                payload: SourcePayload::Utf8(source.into()),
            }],
        },
        &AnalyzerOptions::analysis_mode(),
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

fn run_compiled_result(artifact: &BytecodeArtifact) -> VmValue {
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(artifact);
    let mut vm = Vm::new(validated(artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    vm.read_variable(result, &[0], None).unwrap()
}

#[derive(Default)]
struct ReadyHost {
    calls: Vec<i64>,
}

impl VmHost for ReadyHost {
    fn call(&mut self, request: HostCallRequest) -> HostCallResult {
        if let Some(VmValue::Integer(value)) = request.arguments.first() {
            self.calls.push(*value);
        }
        HostCallResult::Ready(HostReady::empty())
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
    artifact.host_imports.push(HostImport {
        import: runtime_import,
        effect: HostEffect {
            pure: false,
            may_suspend: true,
            may_error: true,
            mutates_runtime: true,
        },
        capability: HostCapability::Input,
        snapshot_capability: stability,
    });
    artifact.refresh_ids().unwrap();
    (artifact, entry)
}

#[test]
fn cooperative_fibers_are_round_robin_and_complete_independently() {
    let (artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let first = vm.spawn_entry(entry, Vec::new()).unwrap();
    let second = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::default();
    let report = vm.run_slice(
        &mut host,
        &mut natives,
        RunBudget {
            maximum_instructions: 32,
            maximum_host_calls: 8,
            fiber_quantum: 1,
        },
    );
    assert_eq!(host.calls, vec![7, 7]);
    assert!(matches!(
        vm.fiber_status(first),
        Some(FiberStatus::Completed(None))
    ));
    assert!(matches!(
        vm.fiber_status(second),
        Some(FiberStatus::Completed(None))
    ));
    assert_eq!(
        report
            .events
            .iter()
            .filter(|event| matches!(event, VmEvent::FiberCompleted { .. }))
            .count(),
        2
    );
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

#[test]
fn stable_wait_snapshot_round_trips_and_requires_exact_artifact() {
    let (artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let mut natives = NativeServiceRegistry::default();
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert_eq!(
        vm.snapshot_eligibility(&natives),
        SnapshotEligibility::Eligible
    );
    let snapshot = vm.snapshot(&natives).unwrap();
    let bytes = snapshot.encode().unwrap();
    let decoded = VmSnapshot::decode(&bytes, bytes.len()).unwrap();
    let mut restore_host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        decoded.clone(),
        &mut restore_host,
        &mut natives,
    )
    .unwrap();
    assert_eq!(restored.artifact_id(), artifact.manifest.artifact_id);
    assert_eq!(restore_host.rebound.len(), 1);

    let mut different = artifact.clone();
    different
        .source_map
        .sources
        .push(erabasic_bytecode::SourceRecord {
            relative_path: "other.erb".into(),
            content_hash: Digest::default(),
            byte_len: 0,
            line_starts: vec![0],
        });
    different.refresh_ids().unwrap();
    assert!(
        Vm::restore_snapshot(
            validated(&different),
            VmConfig::default(),
            decoded,
            &mut restore_host,
            &mut natives,
        )
        .is_err()
    );
}

#[test]
fn transient_qte_wait_cannot_be_snapshotted() {
    let (artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::Transient,
        rebound: Vec::new(),
    };
    let mut natives = NativeServiceRegistry::default();
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(matches!(
        vm.snapshot_eligibility(&natives),
        SnapshotEligibility::Ineligible(ref blockers)
            if blockers.contains(&SnapshotBlocker::TransientHostWait(fiber))
    ));
}

#[test]
fn never_snapshot_capability_accepts_only_transient_host_waits() {
    let (artifact, entry) = host_artifact(HostSnapshotCapability::Never);
    let mut stable_vm = Vm::new(validated(&artifact), VmConfig::default());
    let stable_fiber = stable_vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut stable_host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let mut natives = NativeServiceRegistry::default();
    stable_vm.run_slice(&mut stable_host, &mut natives, RunBudget::default());
    assert!(matches!(
        stable_vm.fiber_status(stable_fiber),
        Some(FiberStatus::Faulted(ref fault)) if fault.code == VmFaultCode::Host
    ));

    let mut transient_vm = Vm::new(validated(&artifact), VmConfig::default());
    let transient_fiber = transient_vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut transient_host = PendingHost {
        stability: HostWaitStability::Transient,
        rebound: Vec::new(),
    };
    transient_vm.run_slice(&mut transient_host, &mut natives, RunBudget::default());
    assert!(matches!(
        transient_vm.fiber_status(transient_fiber),
        Some(FiberStatus::WaitingHost(_))
    ));
    assert!(matches!(
        transient_vm.snapshot_eligibility(&natives),
        SnapshotEligibility::Ineligible(ref blockers)
            if blockers.contains(&SnapshotBlocker::TransientHostWait(transient_fiber))
    ));
}

#[test]
fn host_resume_is_typed_and_late_responses_are_stale() {
    let (artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let mut natives = NativeServiceRegistry::default();
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    let Some(FiberStatus::WaitingHost(request)) = vm.fiber_status(fiber) else {
        panic!("fiber should be waiting for its host request");
    };
    vm.resume_host(request, HostReady::empty()).unwrap();
    assert!(vm.resume_host(request, HostReady::empty()).is_err());
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::Completed(None))
    ));
}

#[test]
fn persistent_budget_exhaustion_trips_the_watchdog() {
    let entry = SymbolKey::derive("test.function", b"loop");
    let artifact = artifact(
        vec![function(entry, "LOOP", vec![opcode::jump(Opcode::Jump, 0)])],
        Vec::new(),
    );
    let mut vm = Vm::new(
        validated(&artifact),
        VmConfig {
            maximum_consecutive_budget_exhaustions: 1,
            ..VmConfig::default()
        },
    );
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::default();
    let report = vm.run_slice(
        &mut host,
        &mut natives,
        RunBudget {
            maximum_instructions: 4,
            maximum_host_calls: 0,
            fiber_quantum: 2,
        },
    );
    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::Faulted(ref fault)) if fault.code == VmFaultCode::RunawayExecution
    ));
    assert!(report.events.iter().any(|event| matches!(
        event,
        VmEvent::FiberFaulted { fault, .. } if fault.code == VmFaultCode::RunawayExecution
    )));
}

fn call_artifact(
    helper_value: i64,
    dimensions: Vec<u64>,
) -> (BytecodeArtifact, SymbolKey, SymbolKey) {
    let main = SymbolKey::derive("test.function", b"main");
    let helper = SymbolKey::derive("test.function", b"helper");
    let variable = SymbolKey::derive("test.variable", b"value");
    let mut main_function = function(
        main,
        "MAIN",
        vec![
            opcode::call(Opcode::Call, 0, 0, Some(BytecodeType::Integer)),
            opcode::return_value(true),
        ],
    );
    main_function.result = Some(BytecodeType::Integer);
    main_function.imports.push(FunctionImport {
        kind: ImportKind::Function,
        key: helper,
    });
    let mut helper_function = function(
        helper,
        "HELPER",
        vec![
            opcode::push_integer(helper_value),
            opcode::return_value(true),
        ],
    );
    helper_function.result = Some(BytecodeType::Integer);
    (
        artifact(
            vec![main_function, helper_function],
            vec![global(variable, dimensions)],
        ),
        main,
        variable,
    )
}

#[test]
fn hot_reload_pins_old_stacks_and_migrates_compatible_state() {
    let (base, entry, variable) = call_artifact(1, vec![2]);
    let (target, _, _) = call_artifact(2, vec![3]);
    let patch = create_patch(&base, &target);
    let mut vm = Vm::new(validated(&base), VmConfig::default());
    vm.write_variable(variable, &[0], None, VmValue::Integer(7))
        .unwrap();
    vm.write_variable(variable, &[1], None, VmValue::Integer(8))
        .unwrap();
    let old = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::default();
    vm.run_slice(
        &mut host,
        &mut natives,
        RunBudget {
            maximum_instructions: 1,
            maximum_host_calls: 0,
            fiber_quantum: 1,
        },
    );
    vm.prepare_hot_reload(&patch, &ValidationContext::for_artifact(&target))
        .unwrap();
    vm.commit_hot_reload().unwrap();
    let new = vm.spawn_entry(entry, Vec::new()).unwrap();
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(matches!(
        vm.fiber_status(old),
        Some(FiberStatus::Completed(Some(VmValue::Integer(1))))
    ));
    assert!(matches!(
        vm.fiber_status(new),
        Some(FiberStatus::Completed(Some(VmValue::Integer(2))))
    ));
    assert_eq!(
        vm.read_variable(variable, &[0], None).unwrap(),
        VmValue::Integer(7)
    );
    assert_eq!(
        vm.read_variable(variable, &[1], None).unwrap(),
        VmValue::Integer(8)
    );
    assert_eq!(
        vm.read_variable(variable, &[2], None).unwrap(),
        VmValue::Integer(0)
    );
}

#[test]
fn incompatible_hot_reload_is_rejected_atomically() {
    let (base, _, variable) = call_artifact(1, vec![2]);
    let mut target = base.clone();
    target.globals[0].value_type = BytecodeType::String;
    target.refresh_ids().unwrap();
    let patch = create_patch(&base, &target);
    let mut vm = Vm::new(validated(&base), VmConfig::default());
    vm.write_variable(variable, &[0], None, VmValue::Integer(11))
        .unwrap();
    let original_id = vm.artifact_id();
    assert!(
        vm.prepare_hot_reload(&patch, &ValidationContext::for_artifact(&target))
            .is_err()
    );
    assert!(vm.pending_hot_reload().is_none());
    assert_eq!(vm.artifact_id(), original_id);
    assert_eq!(
        vm.read_variable(variable, &[0], None).unwrap(),
        VmValue::Integer(11)
    );
}

#[test]
fn traditional_state_overlay_restores_persistent_arrays_without_stacks() {
    let entry = SymbolKey::derive("test.function", b"save");
    let variable = SymbolKey::derive("test.variable", b"save");
    let artifact = artifact(
        vec![function(entry, "SAVE", vec![opcode::return_value(false)])],
        vec![global(variable, vec![2])],
    );
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.write_variable(variable, &[1], None, VmValue::Integer(42))
        .unwrap();
    let save = vm.export_era_state();
    vm.write_variable(variable, &[1], None, VmValue::Integer(0))
        .unwrap();
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.reset_with_era_state(&save).unwrap();
    assert_eq!(report.restored_variables, 1);
    assert_eq!(vm.fiber_ids().count(), 0);
    assert_eq!(
        vm.read_variable(variable, &[1], None).unwrap(),
        VmValue::Integer(42)
    );
}

#[test]
fn compiled_arithmetic_executes_and_updates_project_storage() {
    let artifact = compile_source("@SYSTEM_TITLE\nRESULT = (2 + 3) * 4\nRETURN\n");
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(20));
}

#[test]
fn compiled_assignment_matches_reference_smoke_input() {
    // The macOS/Windows reference smoke suite executes the exact `RESULT = 9`
    // statement and observes RESULT=9 through the C# VM watch projection.
    let artifact = compile_source("@SYSTEM_TITLE\nRESULT = 9\nRETURN\n");
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(9));
}

#[test]
fn runtime_fault_resolves_to_utf8_source_location() {
    let entry = SymbolKey::derive("test.function", b"fault");
    let instruction =
        erabasic_bytecode::EncodedInstruction::new(Opcode::Trap, b"intentional".to_vec());
    let length = instruction.encoded_len();
    let mut artifact = artifact(
        vec![function(entry, "FAULT", vec![instruction])],
        Vec::new(),
    );
    let text = "@FAULT\nTRAP 中文\n";
    artifact.source_map = SourceMap {
        sources: vec![SourceRecord {
            relative_path: "fault.erb".into(),
            content_hash: Digest::hash("test.source", &[text.as_bytes()]),
            byte_len: text.len() as u64,
            line_starts: vec![0, "@FAULT\n".len() as u64],
        }],
        entries: vec![SourceMapEntry {
            function: entry,
            code_start: 0,
            code_end: length,
            source_index: 0,
            byte_start: "@FAULT\n".len() as u64,
            byte_end: text.len() as u64,
            origin_chain: Vec::new(),
        }],
    };
    artifact.refresh_ids().unwrap();
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    vm.run_slice(
        &mut ReadyHost::default(),
        &mut NativeServiceRegistry::default(),
        RunBudget::default(),
    );
    let Some(FiberStatus::Faulted(fault)) = vm.fiber_status(fiber) else {
        panic!("fiber should fault");
    };
    let source = fault.source.expect("fault should have a source location");
    assert_eq!(source.relative_path, "fault.erb");
    assert_eq!(source.line, 2);
    assert_eq!(source.byte_column, 0);
}
