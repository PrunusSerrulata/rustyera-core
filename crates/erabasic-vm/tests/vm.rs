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
    EraSaveScope, FiberStatus, HostCallRequest, HostCallResult, HostReady, HostRebindRequest,
    HostWaitStability, NativeServiceRegistry, RunBudget, RuntimeVm, SnapshotBlocker,
    SnapshotEligibility, Vm, VmBreakpoint, VmBreakpointLocation, VmConfig, VmDebugControl,
    VmDebugInspect, VmDebugVariableWrite, VmEvent, VmFaultCode, VmHost, VmRuntimeFill,
    VmRuntimePort, VmRuntimeStatePort, VmRuntimeStateTransaction, VmSnapshot, VmStepKind, VmValue,
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
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
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
    vm.read_variable(result, &[0], None).unwrap()
}

#[test]
fn power_statement_writes_the_destination_instead_of_passing_its_place_as_an_operand() {
    let artifact = compile_source("@SYSTEM_TITLE\nPOWER RESULT, 2, 3\nRETURN\n");
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(8));
}

#[test]
fn scalar_ref_parameters_store_aliases_and_mutate_the_callers_arrays() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM VALUES, 3\nVALUES:1 = 3\nCALL MUTATE_REF(VALUES)\nRETURN\n@MUTATE_REF(NUMBERS)\n#DIM REF NUMBERS\nNUMBERS:1 = 7\nRETURN\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let values = artifact
        .globals
        .iter()
        .find(|global| global.name == "VALUES")
        .expect("VALUES")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        vm.read_variable(values, &[1], None),
        Ok(VmValue::Integer(7))
    );
}

#[test]
fn dynamic_calls_bind_variable_arguments_as_refs_or_values_from_the_target_signature() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM VALUES, 3\nVALUES:1 = 3\nCALLFORM MUTATE_{1}(VALUES)\nCALLFORM READ_{1}(VALUES:1)\nRETURN\n@MUTATE_1(NUMBERS)\n#DIM REF NUMBERS\nNUMBERS:1 = 7\nRETURN\n@READ_1(VALUE)\n#DIM VALUE\nRESULT:1 = VALUE\nRETURN\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let values = artifact
        .globals
        .iter()
        .find(|global| global.name == "VALUES")
        .expect("VALUES")
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        vm.read_variable(values, &[1], None),
        Ok(VmValue::Integer(7))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(7))
    );
}

#[test]
fn while_false_branch_skips_past_wend_and_finite_loops_terminate() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM ITERATIONS\nWHILE ITERATIONS < 3\nITERATIONS ++\nWEND\nWHILE 0\nITERATIONS = 99\nWEND\nRETURN ITERATIONS\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(3));
}

#[test]
fn bare_return_preserves_the_legacy_result_array() {
    let artifact = compile_source("@SYSTEM_TITLE\nRESULT = 99\nRETURN\n");
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(99));
}

#[test]
fn logical_operators_short_circuit_their_right_operand() {
    for (expression, expected) in [
        ("1 || VALUES:1", 1),
        ("0 && VALUES:1", 0),
        ("0 !& VALUES:1", 1),
        ("1 !| VALUES:1", 0),
        ("1 && 7", 1),
        ("0 || 7", 1),
        ("1 !& 0", 1),
        ("0 !| 0", 1),
    ] {
        let artifact = compile_source(&format!(
            "@SYSTEM_TITLE\n#DIM VALUES, 1\nRETURN {expression}\n"
        ));
        assert_eq!(
            run_compiled_result(&artifact),
            VmValue::Integer(expected),
            "{expression}"
        );
    }
}

#[test]
fn rand_pseudo_variable_uses_the_random_native_instead_of_schema_storage() {
    let artifact = compile_source("@SYSTEM_TITLE\nRETURN RAND:1\n");
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(0));
}

#[test]
fn structured_map_native_preserves_order_and_commits_array_outputs() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIMS KEYS, 4\nRESULT:0 = MAP_CREATE(\"m\")\nRESULT:1 = MAP_SET(\"m\", \"b\", \"1\")\nRESULT:2 = MAP_SET(\"m\", \"a\", \"2\")\nRESULT:3 = MAP_SET(\"m\", \"b\", \"3\")\nRESULTS:0 = %MAP_GET(\"m\", \"b\")%\nRESULTS:1 = %MAP_GETKEYS(\"m\")%\nRESULTS:2 = %MAP_GETKEYS(\"m\", KEYS, 1)%\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .expect("RESULTS")
        .key;
    let keys = artifact
        .globals
        .iter()
        .find(|global| global.name == "KEYS")
        .expect("KEYS")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("3".into()))
    );
    assert_eq!(
        vm.read_variable(results, &[1], None),
        Ok(VmValue::String("b,a".into()))
    );
    assert_eq!(
        vm.read_variable(keys, &[0], None),
        Ok(VmValue::String("b".into()))
    );
    assert_eq!(
        vm.read_variable(keys, &[1], None),
        Ok(VmValue::String("a".into()))
    );
}

#[test]
fn structured_data_table_uses_deterministic_ids_and_updates_rows() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT:0 = DT_CREATE(\"t\")\nRESULT:1 = DT_COLUMN_ADD(\"t\", \"score\", \"int32\", 0)\nRESULT:2 = DT_ROW_ADD(\"t\", \"score\", 7)\nRESULT:3 = DT_ROW_SET(\"t\", RESULT:2, \"score\", 9)\nRESULT:4 = DT_CELL_GET(\"t\", RESULT:2, \"score\", 1)\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        vm.read_variable(result, &[2], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(result, &[3], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(result, &[4], None),
        Ok(VmValue::Integer(9))
    );
}

#[test]
fn structured_xml_mutations_match_the_reference_fixture_subset() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT:0 = XML_DOCUMENT(1, \"<root><item id='a'>one</item><item id='b'>two</item></root>\")\nRESULTS:0 = %XML_TOSTR(1)%\nRESULT:1 = XML_SET(RESULTS:0, \"//item[@id='b']\", \"changed\", 0, 1)\nRESULT:2 = XML_ADDATTRIBUTE(RESULTS:0, \"//item[@id='a']\", \"kind\", \"first\")\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .expect("RESULTS")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(result, &[2], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String(
            "<root><item id=\"a\" kind=\"first\">one</item><item id=\"b\">changed</item></root>"
                .into()
        ))
    );
}

#[test]
fn era_function_local_persists_across_calls() {
    let artifact = compile_source("@COUNTER\nLOCAL:0 += 1\nRESULT = LOCAL:0\nRETURN\n");
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let local = artifact
        .globals
        .iter()
        .find(|global| {
            global.name == "LOCAL" && global.storage == BytecodeStorage::FunctionPersistent
        })
        .expect("persistent LOCAL")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut host = ReadyHost::default();
    for expected in [1, 2] {
        vm.spawn_entry(entry, Vec::new()).unwrap();
        vm.run_slice(&mut host, &mut natives, RunBudget::default());
        assert_eq!(
            vm.read_variable(result, &[0], None),
            Ok(VmValue::Integer(expected))
        );
        assert_eq!(
            vm.read_variable(local, &[0], None),
            Ok(VmValue::Integer(expected))
        );
    }
}

#[test]
fn swap_native_commits_both_places() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 10\nFLAG:1 = 20\nSWAP FLAG:0, FLAG:1\nRESULT = FLAG:0 * 100 + FLAG:1\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .unwrap()
        .key;
    assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(20)));
    assert_eq!(vm.read_variable(flag, &[1], None), Ok(VmValue::Integer(10)));
}

#[test]
fn array_shift_and_remove_commit_after_validating_the_whole_array() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 1\nFLAG:1 = 2\nFLAG:2 = 3\nFLAG:3 = 4\nARRAYSHIFT FLAG, 1, 9, 0, 4\nARRAYREMOVE FLAG, 1, 2\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    let values = (0..4)
        .map(|index| vm.read_variable(flag, &[index], None).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        vec![
            VmValue::Integer(9),
            VmValue::Integer(3),
            VmValue::Integer(0),
            VmValue::Integer(0),
        ]
    );
}

#[test]
fn findelement_uses_the_verified_regex_subset() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"zz\"\nRESULTS:1 '= \"abc\"\nRESULTS:2 '= \"ab\"\nRESULT = FINDELEMENT(RESULTS, \"^ab$\", 0, 3, 1)\nRETURN\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(2));

    // An empty range never inspects an element, so even an invalid pattern is
    // intentionally not compiled and the query returns the not-found sentinel.
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"ab\"\nRESULT = FINDELEMENT(RESULTS, \"a(?=b)\", 0, 0, 0)\nRETURN\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(-1));

    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"ab\"\nRESULT = FINDELEMENT(RESULTS, \"a(?=b)\", 0, 1, 0)\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(report.events.iter().any(|event| matches!(
        event,
        VmEvent::FiberFaulted { fault, .. }
            if fault.message.contains("lookaround")
    )));
}

#[test]
fn arraysort_accepts_reference_forward_back_keywords() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 2\nFLAG:1 = 4\nFLAG:2 = 1\nFLAG:3 = 3\nARRAYSORT FLAG, BACK, 0, 4\nARRAYCOPY FLAG, FLAG\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        (0..4)
            .map(|index| vm.read_variable(flag, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(4),
            VmValue::Integer(3),
            VmValue::Integer(2),
            VmValue::Integer(1),
        ]
    );
}

#[test]
fn arraycopy_resolves_runtime_variable_names_and_array_queries_keep_places() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 3\nFLAG:1 = 1\nFLAG:2 = 3\nARRAYCOPY \"FLAG\", \"FLAG\"\nRESULT:0 = SUMARRAY(FLAG, 0, 3)\nRESULT:1 = MATCH(FLAG, 3, 0, 3)\nRESULT:2 = INRANGEARRAY(FLAG, 2, 3, 0, 3)\nRESULT:3 = GROUPMATCH(3, FLAG:0, FLAG:1, FLAG:2)\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        (0..4)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(7),
            VmValue::Integer(2),
            VmValue::Integer(2),
            VmValue::Integer(2),
        ]
    );
}

#[test]
fn arraycopy_copies_the_shared_extent_when_array_lengths_differ() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM TARGET_LIST, 3\nTARGET:0 = 7\nTARGET:1 = 8\nTARGET:2 = 9\nTARGET:3 = 10\nARRAYCOPY \"TARGET\", \"TARGET_LIST\"\nRESULT:0 = TARGET_LIST:0\nRESULT:1 = TARGET_LIST:1\nRESULT:2 = TARGET_LIST:2\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        (0..3)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(7),
            VmValue::Integer(8),
            VmValue::Integer(9),
        ]
    );
}

#[test]
fn arraycopy_intersects_each_dimension_and_preserves_other_destination_cells() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM SOURCE_VALUES, 2, 3\n#DIM DESTINATION_VALUES, 3, 2\nSOURCE_VALUES:0:0 = 1\nSOURCE_VALUES:0:1 = 2\nSOURCE_VALUES:0:2 = 3\nSOURCE_VALUES:1:0 = 4\nSOURCE_VALUES:1:1 = 5\nSOURCE_VALUES:1:2 = 6\nDESTINATION_VALUES:2:0 = 9\nDESTINATION_VALUES:2:1 = 9\nARRAYCOPY \"SOURCE_VALUES\", \"DESTINATION_VALUES\"\nRESULT:10 = DESTINATION_VALUES:0:0\nRESULT:11 = DESTINATION_VALUES:0:1\nRESULT:12 = DESTINATION_VALUES:1:0\nRESULT:13 = DESTINATION_VALUES:1:1\nRESULT:14 = DESTINATION_VALUES:2:0\nRESULT:15 = DESTINATION_VALUES:2:1\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        (0..6)
            .map(|index| vm.read_variable(result, &[index + 10], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(1),
            VmValue::Integer(2),
            VmValue::Integer(4),
            VmValue::Integer(5),
            VmValue::Integer(9),
            VmValue::Integer(9),
        ]
    );
}

#[test]
fn printsingleforms_expands_a_constant_template_in_the_current_function_scope() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nCALL DRAW_INFORMATIONLINE, \"地图\"\nRETURN\n@DRAW_INFORMATIONLINE(ARGS)\n#DIMS EQUAL\nEQUAL = =\nPRINTSINGLEFORMS \"== %ARGS% \" + \"%(EQUAL * 3)%\"\nRETURN\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
    assert_eq!(host.strings, vec!["== 地图 ==="]);
}

#[test]
fn arraymsort_reorders_complete_rows_before_committing() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 3\nFLAG:1 = 1\nFLAG:2 = 2\nFLAG:3 = 0\nRESULT:0 = 30\nRESULT:1 = 10\nRESULT:2 = 20\nRESULT:9 = ARRAYMSORT(FLAG, RESULT)\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        (0..3)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(10),
            VmValue::Integer(20),
            VmValue::Integer(30),
        ]
    );
    assert_eq!(
        vm.read_variable(result, &[9], None),
        Ok(VmValue::Integer(1))
    );
}

#[test]
fn arraymsortex_resolves_target_names_at_runtime() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 3\nFLAG:1 = 1\nFLAG:2 = 2\nFLAG:3 = 0\nTFLAG:0 = 30\nTFLAG:1 = 10\nTFLAG:2 = 20\nRESULTS:0 '= \"TFLAG\"\nRESULTS:1 '= \"\"\nRESULT:9 = ARRAYMSORTEX(FLAG, RESULTS, 1, -1)\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let tflag = artifact
        .globals
        .iter()
        .find(|global| global.name == "TFLAG")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        (0..3)
            .map(|index| vm.read_variable(tflag, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(10),
            VmValue::Integer(20),
            VmValue::Integer(30),
        ]
    );
}

#[test]
fn arraymsortex_rolls_back_when_a_later_dynamic_target_is_invalid() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 3\nFLAG:1 = 1\nFLAG:2 = 2\nFLAG:3 = 0\nTFLAG:0 = 30\nTFLAG:1 = 10\nTFLAG:2 = 20\nRESULTS:0 '= \"TFLAG\"\nRESULTS:1 '= \"MISSING\"\nRESULT:9 = ARRAYMSORTEX(FLAG, RESULTS, 1, -1)\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let tflag = artifact
        .globals
        .iter()
        .find(|global| global.name == "TFLAG")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(report.events.iter().any(|event| matches!(
        event,
        VmEvent::FiberFaulted { fault, .. } if fault.message.contains("MISSING")
    )));
    assert_eq!(
        (0..3)
            .map(|index| vm.read_variable(tflag, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(30),
            VmValue::Integer(10),
            VmValue::Integer(20),
        ]
    );
}

#[test]
fn character_mutations_commit_as_one_memory_transaction() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nADDVOIDCHARA\nADDCOPYCHARA 0\nSWAPCHARA 0, 1\nDELCHARA 1\nRESULT = CHARANUM\nRETURN\n",
    );
    assert!(
        artifact
            .native_imports
            .iter()
            .any(|import| import.import.name.eq_ignore_ascii_case("ADDVOIDCHARA")),
        "{:#?}",
        artifact.native_imports
    );
    let entry = artifact.functions[0].key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(2))
    );
}

#[test]
fn varset_fills_only_the_validated_half_open_range() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 1\nFLAG:1 = 2\nFLAG:2 = 3\nFLAG:3 = 4\nVARSET FLAG, 9, 1, 3\nRESULTS:0 '= \"a\"\nRESULTS:1 '= \"b\"\nRESULTS:2 '= \"c\"\nRESULTS:3 '= \"d\"\nVARSET RESULTS, \"x\", 3, 1\nRESULT = FLAG:1\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .unwrap()
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .unwrap()
        .key;
    assert_eq!(
        (0..4)
            .map(|index| vm.read_variable(flag, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(1),
            VmValue::Integer(9),
            VmValue::Integer(9),
            VmValue::Integer(4),
        ]
    );
    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(9))
    );
    assert_eq!(
        (0..4)
            .map(|index| vm.read_variable(results, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::String("a".into()),
            VmValue::String("x".into()),
            VmValue::String("x".into()),
            VmValue::String("d".into()),
        ]
    );
}

#[test]
fn direct_runtime_fills_validate_the_complete_batch_before_mutation() {
    let artifact = compile_source("@SYSTEM_TITLE\nRETURN\n");
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.write_variable(flag, &[0], None, VmValue::Integer(7))
        .unwrap();

    let error = vm.fill_runtime_variables(&[
        VmRuntimeFill {
            variable: flag,
            value: VmValue::Integer(1),
            all_characters: false,
        },
        VmRuntimeFill {
            variable: SymbolKey::derive("test.missing", b"runtime-fill"),
            value: VmValue::Integer(2),
            all_characters: false,
        },
    ]);
    assert!(matches!(error, Err(erabasic_vm::VmError::InvalidState(_))));
    assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(7)));

    vm.fill_runtime_variables(&[VmRuntimeFill {
        variable: flag,
        value: VmValue::Integer(3),
        all_characters: false,
    }])
    .unwrap();
    assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(3)));
    assert_eq!(vm.read_variable(flag, &[1], None), Ok(VmValue::Integer(3)));
}

#[test]
fn cvarset_prevalidates_and_fills_the_character_range() {
    let artifact =
        compile_source("@SYSTEM_TITLE\nADDVOIDCHARA\nCVARSET CFLAG, 1, 7, 0, 2\nRETURN\n");
    let entry = artifact.functions[0].key;
    let cflag = artifact
        .globals
        .iter()
        .find(|global| global.name == "CFLAG")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        (0..2)
            .map(|character| vm.read_variable(cflag, &[1], Some(character)).unwrap())
            .collect::<Vec<_>>(),
        vec![VmValue::Integer(7), VmValue::Integer(7)]
    );
}

#[test]
fn script_can_address_character_storage_explicitly_or_through_target() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nADDVOIDCHARA\nCFLAG:0:1 = 3\nCFLAG:1:1 = 4\nTARGET = 1\nRESULT:0 = CFLAG:1\nRESULT:1 = CFLAG:0:1\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let cflag = artifact
        .globals
        .iter()
        .find(|global| global.name == "CFLAG")
        .unwrap()
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        vm.read_variable(cflag, &[1], Some(0)),
        Ok(VmValue::Integer(3))
    );
    assert_eq!(
        vm.read_variable(cflag, &[1], Some(1)),
        Ok(VmValue::Integer(4))
    );
    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(4))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(3))
    );
}

#[test]
fn cvarset_invalid_range_does_not_write_any_character() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nADDVOIDCHARA\nTARGET = 0\nCFLAG:1 = 3\nTARGET = 1\nCFLAG:1 = 4\nCVARSET CFLAG, 1, 7, 0, 3\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let cflag = artifact
        .globals
        .iter()
        .find(|global| global.name == "CFLAG")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(report.events.iter().any(|event| matches!(
        event,
        VmEvent::FiberFaulted { fault, .. } if fault.message.contains("range")
    )));
    assert_eq!(
        (0..2)
            .map(|character| vm.read_variable(cflag, &[1], Some(character)).unwrap())
            .collect::<Vec<_>>(),
        vec![VmValue::Integer(3), VmValue::Integer(4)]
    );
}

#[test]
fn sortchara_reorders_characters_and_remaps_target() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nADDVOIDCHARA\nADDVOIDCHARA\nTARGET = 0\nNO = 30\nTARGET = 1\nNO = 10\nTARGET = 2\nNO = 20\nMASTER = -1\nSORTCHARA NO, FORWARD\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let no = artifact
        .globals
        .iter()
        .find(|global| global.name == "NO")
        .unwrap()
        .key;
    let target = artifact
        .globals
        .iter()
        .find(|global| global.name == "TARGET")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        (0..3)
            .map(|character| vm.read_variable(no, &[], Some(character)).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(10),
            VmValue::Integer(20),
            VmValue::Integer(30),
        ]
    );
    assert_eq!(vm.read_variable(target, &[], None), Ok(VmValue::Integer(1)));
}

#[test]
fn cmatch_counts_an_indexed_character_field_across_the_requested_range() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nADDVOIDCHARA\nADDVOIDCHARA\nADDVOIDCHARA\nTARGET = 0\nCFLAG:5 = 9\nTARGET = 1\nCFLAG:5 = 4\nTARGET = 2\nCFLAG:5 = 9\nRETURN CMATCH(CFLAG:5, 9, 0, 3)\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(2));
}

#[test]
fn failed_character_mutation_rolls_back_the_complete_candidate() {
    let artifact = compile_source("@SYSTEM_TITLE\nADDVOIDCHARA\nDELCHARA 0, 99\nRETURN\n");
    let entry = artifact.functions[0].key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(report.events.iter().any(|event| matches!(
        event,
        VmEvent::FiberFaulted { fault, .. } if fault.message.contains("out of range")
    )));
    // New-game memory starts with one character. ADDVOIDCHARA committed first,
    // while the later multi-delete validates every index before mutating memory.
    assert_eq!(vm.export_era_state().characters.len(), 2);
}

#[test]
fn character_csv_queries_use_loaded_templates_and_character_lookup() {
    let loaded = load_project(
        &ProjectFiles {
            csv: vec![FrontendFile {
                relative_path: "CHARA0.CSV".into(),
                payload: CsvFilePayload::Utf8("NO,10\nNAME,Alice\nBASE,0,100\nCFLAG,1,7\n".into()),
            }],
            erb: Vec::new(),
        },
        &CsvLoadOptions::default(),
    )
    .data
    .expect("character CSV should load");
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\nRESULT:0 = GETCHARA(10)\nRESULT:1 = CSVBASE(10, 0)\nRESULT:2 = CSVCFLAG(10, 1)\nRESULT:3 = CSVNAME(10) == \"Alice\"\nRETURN\n",
        loaded,
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        (0..4)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(0),
            VmValue::Integer(100),
            VmValue::Integer(7),
            VmValue::Integer(1),
        ]
    );
}

#[test]
fn resetdata_clears_initial_characters_before_script_insertion() {
    let loaded = load_project(
        &ProjectFiles {
            csv: vec![FrontendFile {
                relative_path: "Chara/CHARA0.CSV".into(),
                payload: CsvFilePayload::Utf8("NO,0\nNAME,Master\n".into()),
            }],
            erb: Vec::new(),
        },
        &CsvLoadOptions {
            search_subdirectories: true,
            ..CsvLoadOptions::default()
        },
    )
    .data
    .expect("character CSV should load");
    let artifact = compile_source_with_data("@SYSTEM_TITLE\nRETURN\n", loaded);
    let mut vm = RuntimeVm::new(validated(&artifact), VmConfig::default());
    assert_eq!(vm.export_era_state().characters.len(), 1);

    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::ResetGameData)
        .unwrap();
    vm.commit_runtime_state(prepared).unwrap();

    assert!(vm.export_era_state().characters.is_empty());
}

#[test]
fn duplicate_event_handlers_share_persistent_era_locals() {
    let artifact =
        compile_source("@EVENTTRAIN\nVARSET LOCAL\nRETURN\n@EVENTTRAIN\nLOCAL:0 = 1\nRETURN\n");
    let entries = artifact
        .functions
        .iter()
        .filter(|function| function.name.eq_ignore_ascii_case("EVENTTRAIN"))
        .map(|function| function.key)
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 2);

    for entry in entries {
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    }
}

#[test]
fn regexpmatch_writes_reference_capture_outputs_atomically() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT:0 = REGEXPMATCH(\"ab ac\", \"a(.)\", 1)\nRESULT:2 = REGEXPMATCH(\"az\", \"a(.)\", RESULT:5, RESULTS)\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(result, &[2], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(result, &[5], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("az".into()))
    );
    assert_eq!(
        vm.read_variable(results, &[1], None),
        Ok(VmValue::String("z".into()))
    );
}

#[test]
fn initrand_and_dumprand_exchange_all_randdata_state_atomically() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nDUMPRAND\nRESULT:0 = RAND:1000000\nINITRAND\nRESULT:1 = RAND:1000000\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 1234);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert_eq!(
        vm.read_variable(result, &[0], None),
        vm.read_variable(result, &[1], None)
    );
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
    let (mut artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    artifact.globals.push(global(
        SymbolKey::derive("test.snapshot", b"dense-zero-array"),
        vec![16_384],
    ));
    artifact.refresh_ids().unwrap();
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
    let uncompressed = serde_json::to_vec(&snapshot).unwrap();
    let bytes = snapshot.encode().unwrap();
    assert!(bytes.len() < uncompressed.len() / 4);
    let mut understated = bytes.clone();
    understated[20..28].copy_from_slice(&((uncompressed.len() as u64) - 1).to_le_bytes());
    assert!(VmSnapshot::decode(&understated, uncompressed.len()).is_err());
    let decoded = VmSnapshot::decode(&bytes, uncompressed.len()).unwrap();
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
fn quiescent_vm_snapshot_round_trips_without_host_wait_rebinding() {
    let artifact = compile_source("@SYSTEM_TITLE\nRETURN\n");
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::Completed(_))
    ));
    assert_eq!(
        vm.snapshot_eligibility(&natives),
        SnapshotEligibility::Eligible
    );

    let snapshot = vm.snapshot(&natives).unwrap();
    let mut restore_host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        snapshot,
        &mut restore_host,
        &mut natives,
    )
    .unwrap();
    assert!(restore_host.rebound.is_empty());
    assert!(matches!(
        restored.fiber_status(fiber),
        Some(FiberStatus::Completed(_))
    ));
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
    let first_report = vm.run_slice(
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
        Some(FiberStatus::Runnable)
    ));
    assert!(first_report.events.is_empty());
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

#[test]
fn finite_work_spanning_many_fiber_quanta_does_not_trip_budget_watchdog() {
    let entry = SymbolKey::derive("test.function", b"finite");
    let mut code = vec![erabasic_bytecode::EncodedInstruction::new(Opcode::Nop, Vec::new()); 8];
    code.push(opcode::return_value(false));
    let artifact = artifact(vec![function(entry, "FINITE", code)], Vec::new());
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
            maximum_instructions: 32,
            maximum_host_calls: 0,
            fiber_quantum: 2,
        },
    );

    assert_eq!(report.instructions, 9);
    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::Completed(None))
    ));
    assert!(!report.events.iter().any(|event| matches!(
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
fn debugger_pause_step_and_variable_batch_are_coherent_and_atomic() {
    let (artifact, entry, _) = call_artifact(7, vec![1]);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let stop = vm.request_pause().unwrap();
    let page = vm.variables(stop.token, None, 32).unwrap();
    let variable = page.values.first().expect("project variable").clone();
    let mut invalid_target = variable.target.clone();
    invalid_target.target.indices[0] = 99;
    assert!(
        vm.write_variables(
            stop.token,
            &[
                VmDebugVariableWrite {
                    target: variable.target.clone(),
                    value: VmValue::Integer(41),
                    expected_revision: variable.revision,
                },
                VmDebugVariableWrite {
                    target: invalid_target,
                    value: VmValue::Integer(42),
                    expected_revision: variable.revision,
                },
            ],
        )
        .is_err()
    );
    assert_eq!(
        VmDebugInspect::read_variable(&vm, stop.token, &variable.target)
            .unwrap()
            .value,
        VmValue::Integer(0)
    );

    vm.step(stop.token, fiber, VmStepKind::Instruction).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::default();
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::DebugStopped(_))),
        "{:#?}",
        report.events
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
fn function_breakpoints_rebind_to_the_new_hot_reload_generation() {
    let (base, entry, _) = call_artifact(1, vec![1]);
    let (target, _, _) = call_artifact(2, vec![1]);
    let patch = create_patch(&base, &target);
    let mut vm = Vm::new(validated(&base), VmConfig::default());
    vm.update_breakpoints(
        &[VmBreakpoint {
            id: 9,
            enabled: true,
            hit_count: 0,
            location: VmBreakpointLocation::Function(entry),
        }],
        &[],
    )
    .unwrap();
    vm.prepare_hot_reload(&patch, &ValidationContext::for_artifact(&target))
        .unwrap();
    vm.commit_hot_reload().unwrap();
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::default();
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(report.events.iter().any(|event| matches!(
        event,
        VmEvent::DebugStopped(stop)
            if matches!(stop.reason, erabasic_vm::VmDebugStopReason::Breakpoint(9))
    )));
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
fn ordinary_save_excludes_and_restore_preserves_global_save_variables() {
    let ordinary = SymbolKey::derive("test.variable", b"ordinary");
    let global_key = SymbolKey::derive("test.variable", b"global");
    let mut ordinary_definition = global(ordinary, vec![1]);
    ordinary_definition.name = "ORDINARY".into();
    let mut global_definition = global(global_key, vec![1]);
    global_definition.name = "GLOBAL_VALUE".into();
    global_definition.persistence = BytecodePersistence::GlobalSave;
    let artifact = artifact(Vec::new(), vec![ordinary_definition, global_definition]);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.write_variable(ordinary, &[0], None, VmValue::Integer(11))
        .unwrap();
    vm.write_variable(global_key, &[0], None, VmValue::Integer(21))
        .unwrap();
    let save = vm.export_era_state();
    assert!(save.variables.contains_key(&ordinary));
    assert!(!save.variables.contains_key(&global_key));

    vm.write_variable(ordinary, &[0], None, VmValue::Integer(12))
        .unwrap();
    vm.write_variable(global_key, &[0], None, VmValue::Integer(22))
        .unwrap();
    vm.reset_with_era_state(&save).unwrap();
    assert_eq!(
        vm.read_variable(ordinary, &[0], None),
        Ok(VmValue::Integer(11))
    );
    assert_eq!(
        vm.read_variable(global_key, &[0], None),
        Ok(VmValue::Integer(22))
    );
}

#[test]
fn global_overlay_transaction_changes_only_global_save_storage() {
    let ordinary = SymbolKey::derive("test.variable", b"ordinary-overlay");
    let global_key = SymbolKey::derive("test.variable", b"global-overlay");
    let mut ordinary_definition = global(ordinary, vec![1]);
    ordinary_definition.name = "ORDINARY_OVERLAY".into();
    let mut global_definition = global(global_key, vec![1]);
    global_definition.name = "GLOBAL_OVERLAY".into();
    global_definition.persistence = BytecodePersistence::GlobalSave;
    let artifact = artifact(Vec::new(), vec![ordinary_definition, global_definition]);
    let mut vm = RuntimeVm::new(validated(&artifact), VmConfig::default());
    vm.vm_mut()
        .write_variable(ordinary, &[0], None, VmValue::Integer(10))
        .unwrap();
    let mut state = vm.vm().export_era_state_for(EraSaveScope::Global);
    state.variables.get_mut(&global_key).unwrap().values[0] = VmValue::Integer(20);
    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::OverlayGlobal(Box::new(state)))
        .unwrap();
    vm.commit_runtime_state(prepared).unwrap();
    assert_eq!(
        vm.vm().read_variable(ordinary, &[0], None),
        Ok(VmValue::Integer(10))
    );
    assert_eq!(
        vm.vm().read_variable(global_key, &[0], None),
        Ok(VmValue::Integer(20))
    );
}

#[test]
fn isolated_fork_copies_memory_without_copying_live_execution() {
    let key = SymbolKey::derive("test.variable", b"candidate");
    let artifact = artifact(Vec::new(), vec![global(key, vec![1])]);
    let mut live = RuntimeVm::new(validated(&artifact), VmConfig::default());
    live.vm_mut()
        .write_variable(key, &[0], None, VmValue::Integer(7))
        .unwrap();

    let mut candidate = live.fork_isolated().unwrap();
    assert_eq!(
        candidate.vm().read_variable(key, &[0], None),
        Ok(VmValue::Integer(7))
    );
    candidate
        .vm_mut()
        .write_variable(key, &[0], None, VmValue::Integer(9))
        .unwrap();

    assert_eq!(
        live.vm().read_variable(key, &[0], None),
        Ok(VmValue::Integer(7))
    );
    assert!(!candidate.has_runnable_fibers());
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
fn dynamic_try_resolves_before_arguments_and_form_call_invokes_target() {
    let artifact = compile_source(
        "@ORACLE_COMPAT\nRESULT = 0\nTRYCALLFORM ORACLE_MISSING(1 / LOCAL)\nCALLFORM ORACLE_DYNAMIC_{1}(4)\nRETURN\n@ORACLE_DYNAMIC_1(ARG)\nFLAG:0 = ARG\nRETURN\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "ORACLE_COMPAT")
        .expect("entry")
        .key;
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .expect("FLAG")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(4)));
}

#[test]
fn indexed_data_targets_dynamic_labels_and_try_lists_execute_lazily() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nPRINTDATA RESULT:2\nDATA chosen\nENDDATA\nSTRDATA RESULTS:3\nDATA stored\nENDDATA\nTRYCALLLIST\nFUNC MISSING, 1 / LOCAL\nFUNC LIST_TARGET, 7\nENDFUNC\nRESULTS:11 = %\"MISSING_LABEL\"%\nTRYCGOTOFORM %RESULTS:11%\nCATCH\nRESULT:3 = 3\nENDCATCH\nTRYGOTOLIST\nFUNC MISSING_LABEL\nFUNC FOUND_LABEL\nENDFUNC\nRESULT:4 = 99\n$FOUND_LABEL\nRESULT:4 = 4\nRETURN\n@LIST_TARGET(ARG)\nFLAG:0 = ARG\nRETURN\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("entry")
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .expect("RESULTS")
        .key;
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .expect("FLAG")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        vm.read_variable(result, &[2], None),
        Ok(VmValue::Integer(0))
    );
    assert_eq!(
        vm.read_variable(result, &[3], None),
        Ok(VmValue::Integer(3))
    );
    assert_eq!(
        vm.read_variable(result, &[4], None),
        Ok(VmValue::Integer(4))
    );
    assert_eq!(
        vm.read_variable(results, &[3], None),
        Ok(VmValue::String("stored".into()))
    );
    assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(7)));
}

#[test]
fn callevent_runs_the_reference_event_group_inside_the_calling_fiber() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT = 0\nCALLEVENT EVENTFIRST\nRESULT:3 = 9\nRETURN\n@EVENTFIRST\n#PRI\nRESULT:0 += 1\nRETURN\n@EVENTFIRST\nRESULT:1 += 2\nRETURN\n@EVENTFIRST\n#LATER\nRESULT:2 += 4\nRETURN\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("entry")
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    for (index, expected) in [(0, 1), (1, 2), (2, 4), (3, 9)] {
        assert_eq!(
            vm.read_variable(result, &[index], None),
            Ok(VmValue::Integer(expected))
        );
    }
}

#[test]
fn dynamic_calls_apply_omission_conversion_and_event_compatibility_options() {
    let mut options = AnalyzerOptions::analysis_mode();
    options.compatible_function_argument_optional = true;
    options.compatible_function_argument_auto_convert = true;
    options.compatible_call_event = true;
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nCALLFORM STRING_TARGET()\nCALLFORM STRING_TARGET(7)\nCALLFORM EVENTFIRST\nRETURN\n@STRING_TARGET(ARGS)\nRESULTS:0 = %ARGS%\nRETURN\n@EVENTFIRST\nRESULT:0 = 8\nRETURN\n",
        &options,
    );
    assert!(artifact.call_compatibility.allow_omitted_arguments);
    assert!(artifact.call_compatibility.auto_convert_integer_to_string);
    assert!(artifact.call_compatibility.allow_event_as_normal);
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("entry")
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .expect("RESULTS")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(8))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("7".into()))
    );
}

#[test]
fn compatibility_rest_matches_the_reference_oracle_fixture() {
    let artifact = compile_source(include_str!(
        "../../../tools/emuera-reference-cli/tests/fixture/erb/oracle.erb"
    ));
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "ORACLE_COMPAT_REST")
        .expect("oracle entry")
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .expect("RESULTS")
        .key;
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .expect("FLAG")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    for (index, expected) in [(1, 0), (2, 3), (3, 4), (4, 1), (5, 2)] {
        assert_eq!(
            vm.read_variable(result, &[index], None),
            Ok(VmValue::Integer(expected))
        );
    }
    assert_eq!(
        vm.read_variable(results, &[10], None),
        Ok(VmValue::String("STORED".into()))
    );
    assert_eq!(vm.read_variable(flag, &[1], None), Ok(VmValue::Integer(7)));
    assert_eq!(vm.read_variable(flag, &[2], None), Ok(VmValue::Integer(8)));
}

#[test]
fn dynamic_statement_calls_enforce_method_and_normal_function_kinds() {
    let valid = compile_source(
        "@SYSTEM_TITLE\nCALLFORMF METHOD_TARGET(3)\nRETURN\n@METHOD_TARGET(ARG)\n#FUNCTION\nRETURNF ARG + 1\n",
    );
    let entry = valid
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("entry")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&valid);
    let mut vm = Vm::new(validated(&valid), VmConfig::default());
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

    let invalid = compile_source(
        "@SYSTEM_TITLE\nCALLFORM METHOD_TARGET(3)\nRETURN\n@METHOD_TARGET(ARG)\n#FUNCTION\nRETURNF ARG + 1\n",
    );
    let entry = invalid
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("entry")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&invalid);
    let mut vm = Vm::new(validated(&invalid), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(report.events.iter().any(|event| matches!(
        event,
        VmEvent::FiberFaulted { fault, .. } if fault.code == VmFaultCode::TypeMismatch
    )));
}

#[test]
fn compiled_bit_mutations_prevalidate_and_update_the_target() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT = 0\nSETBIT RESULT, 1, 3\nINVERTBIT RESULT, 1\nCLEARBIT RESULT, 3\nRETURN\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(0));
}

#[test]
fn compiled_split_preserves_empty_fields_and_reports_the_full_count() {
    let artifact =
        compile_source("@SYSTEM_TITLE\n#DIMS TEMP, 4\nSPLIT \"a//b/\", \"/\", TEMP\nRETURN\n");
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(4));
}

#[test]
fn getnum_resolves_the_referenced_builtin_name_table_at_runtime() {
    let mut data = project_data();
    data.static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Cflag)
        .unwrap()
        .lookup
        .insert("dynamic-key".into(), 17);
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\nRESULT = GETNUM(CFLAG, \"dynamic-key\")\nRETURN\n",
        data,
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(17));
}

#[test]
fn native_tail_matches_the_reference_oracle_fixture() {
    let artifact = compile_source(
        "@ORACLE_NATIVE\n#DIMS PARTS, 4\n#DIMS JOINED, 4\nRESULT:0 = 0\nSETBIT RESULT:0, 1, 3\nINVERTBIT RESULT:0, 1\nCLEARBIT RESULT:0, 3\nSPLIT \"a//b/\", \"/\", PARTS, RESULT:1\nRESULT:2 = STRCOUNT(\"ababa\", \"aba\")\nRESULT:3 = GETPALAMLV(499, 5)\nRESULTS:0 = %ESCAPE(\"a+b\")%\nJOINED:0 = %\"a\"%\nJOINED:1 = %\"b\"%\nJOINED:2 = %\"c\"%\nRESULT:4 = STRLENS(\"Ab\")\nRESULT:5 = STRLENSU(\"Aé\")\nRESULT:12 = STRLENSU(\"😀\")\nRESULT:6 = STRFINDU(\"aβc\", \"β\")\nRESULT:7 = ENCODETOUNI(\"β\")\nRESULT:8 = UNICODEBYTE(\"β\")\nRESULTS:1 = %CHARATU(\"aβ\", 1)%\nRESULTS:2 = %TOUPPER(\"Abc\")%\nRESULTS:3 = %TOLOWER(\"AbC\")%\nRESULTS:4 = %STRJOIN(JOINED, \"/\", 1, 2)%\nRESULTS:5 = %STRJOIN(JOINED)%\nRESULTS:6 = %UNICODE(946)%\nRESULT:9 = TOINT(\"12.9\")\nRESULT:10 = ISNUMERIC(\"0x10\")\nRESULT:11 = COLOR_FROMRGB(1, 2, 3)\nRESULTS:7 = %CONVERT(255, 16)%\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(0))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(4))
    );
    assert_eq!(
        vm.read_variable(result, &[2], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(result, &[3], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("a\\+b".into()))
    );
    for (index, expected) in [(4, 2), (5, 2), (6, 1), (7, 946), (8, 946)] {
        assert_eq!(
            vm.read_variable(result, &[index], None),
            Ok(VmValue::Integer(expected))
        );
    }
    for (index, expected) in [(9, 12), (10, 1), (11, 0x0001_0203)] {
        assert_eq!(
            vm.read_variable(result, &[index], None),
            Ok(VmValue::Integer(expected))
        );
    }
    assert_eq!(
        vm.read_variable(result, &[12], None),
        Ok(VmValue::Integer(2))
    );
    for (index, expected) in [
        (1, "β"),
        (2, "ABC"),
        (3, "abc"),
        (4, "b/c"),
        (5, "a,b,c,"),
        (6, "β"),
        (7, "ff"),
    ] {
        assert_eq!(
            vm.read_variable(results, &[index], None),
            Ok(VmValue::String(expected.into()))
        );
    }
}

#[test]
fn unicode_u_positions_use_scalars_but_lengths_match_dotnet_utf16_units() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT:0 = STRLENSU(\"A😀\")\nRESULT:1 = STRFINDU(\"A😀B\", \"B\")\nRESULT:2 = STRLENS(\"Aé\")\nRESULTS:0 = %CHARATU(\"A😀B\", 1)%\nRETURN\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(3))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(result, &[2], None),
        Ok(VmValue::Integer(3))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("😀".into()))
    );
}

#[test]
fn runtime_fault_resolves_to_utf8_source_location() {
    let entry = SymbolKey::derive("test.function", b"fault");
    let first = erabasic_bytecode::EncodedInstruction::new(Opcode::Nop, Vec::new());
    let first_length = first.encoded_len();
    let trap = erabasic_bytecode::EncodedInstruction::new(Opcode::Trap, b"intentional".to_vec());
    let length = first_length + trap.encoded_len();
    let mut artifact = artifact(
        vec![function(entry, "FAULT", vec![first, trap])],
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
        statement_fingerprints: vec![
            Digest::hash("test.statement", &[b"fault"]),
            Digest::hash("test.statement", &[b"overlap"]),
        ],
        entries: vec![
            SourceMapEntry {
                function: entry,
                code_start: 0,
                code_end: length,
                source_index: 0,
                byte_start: "@FAULT\n".len() as u64,
                byte_end: text.len() as u64,
                statement_fingerprint: 0,
                origin_chain: None,
            },
            // A later, narrower overlapping entry must not override the serialized map's first
            // match. The generation index is an execution cache, not a semantic reordering.
            SourceMapEntry {
                function: entry,
                code_start: first_length,
                code_end: length,
                source_index: 0,
                byte_start: 0,
                byte_end: "@FAULT".len() as u64,
                statement_fingerprint: 1,
                origin_chain: None,
            },
        ],
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
