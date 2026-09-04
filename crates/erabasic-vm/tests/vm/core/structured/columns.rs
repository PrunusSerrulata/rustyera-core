use super::*;
fn column_entry(artifact: &BytecodeArtifact, name: &str) -> SymbolKey {
    artifact
        .functions
        .iter()
        .find(|function| function.name == name)
        .unwrap()
        .key
}

fn column_result(vm: &Vm, artifact: &BytecodeArtifact, index: u64) -> VmValue {
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT" && global.owner.is_none())
        .unwrap()
        .key;
    vm.read_variable(result, &[index], None).unwrap()
}

fn run_column_entry(
    vm: &mut Vm,
    natives: &mut NativeServiceRegistry,
    artifact: &BytecodeArtifact,
    name: &str,
) -> erabasic_vm::VmRunReport {
    vm.spawn_entry(column_entry(artifact, name), Vec::new())
        .unwrap();
    vm.run_slice(&mut ReadyHost::default(), natives, RunBudget::default())
}

fn assert_column_success(report: &erabasic_vm::VmRunReport) {
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
}

#[test]
fn column_options_evaluate_column_then_table_and_apply_each_default_immediately() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
DT_CREATE "t"
DT_COLUMN_ADD "t", "value", "int32", 0
RESULT:10 = 0
RESULT:0 = 91
DT_COLUMN_OPTIONS TABLE_NAME(), COLUMN_NAME(), DEFAULT, 7, DEFAULT, NEXT_DEFAULT()
RESULT:11 = RESULT:0
RESULT:22 = DT_ROW_ADD("t")
RESULT:23 = DT_CELL_GET("t", 1, "value")
RETURN RESULT:0
@COLUMN_NAME
#FUNCTIONS
RESULT:10 = RESULT:10 * 10 + 1
RETURNF "value"
@TABLE_NAME
#FUNCTIONS
RESULT:10 = RESULT:10 * 10 + 2
RETURNF "t"
@NEXT_DEFAULT
#FUNCTION
RESULT:10 = RESULT:10 * 10 + 3
RESULT:20 = DT_ROW_ADD("t")
RESULT:21 = DT_CELL_GET("t", 0, "value")
RETURNF 9
"#,
    );
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    assert_column_success(&run_column_entry(
        &mut vm,
        &mut natives,
        &artifact,
        "SYSTEM_TITLE",
    ));
    for (index, expected) in [(10, 123), (11, 91), (21, 7), (23, 9)] {
        assert_eq!(
            column_result(&vm, &artifact, index),
            VmValue::Integer(expected)
        );
    }
}

#[test]
fn column_options_type_error_preserves_prior_default_without_evaluating_value_body() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
DT_CREATE "t"
DT_COLUMN_ADD "t", "value", "int32"
RESULT:0 = 81
RESULT:10 = 0
DT_COLUMN_OPTIONS "t", "value", DEFAULT, 7, DEFAULT, WRONG_VALUE()
RETURN RESULT:0
@WRONG_VALUE
#FUNCTIONS
RESULT:10 = 1
RETURNF "wrong"
@OBSERVE
RESULT:20 = DT_ROW_ADD("t")
RESULT:21 = DT_CELL_GET("t", 0, "value")
RETURN RESULT:0
"#,
    );
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let report = run_column_entry(&mut vm, &mut natives, &artifact, "SYSTEM_TITLE");
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. }))
    );
    assert_eq!(column_result(&vm, &artifact, 10), VmValue::Integer(0));
    assert_eq!(column_result(&vm, &artifact, 0), VmValue::Integer(81));
    assert_column_success(&run_column_entry(
        &mut vm,
        &mut natives,
        &artifact,
        "OBSERVE",
    ));
    assert_eq!(column_result(&vm, &artifact, 21), VmValue::Integer(7));
}

#[test]
fn column_options_missing_targets_fault_after_result_and_do_not_run_values() {
    for (table, column, expected) in [("missing", "value", -1), ("t", "missing", 0)] {
        let source = format!(
            r#"@SYSTEM_TITLE
DT_CREATE "t"
DT_COLUMN_ADD "t", "value", "int32"
RESULT:10 = 0
DT_COLUMN_OPTIONS "{table}", "{column}", DEFAULT, VALUE_BODY()
RETURN RESULT:0
@VALUE_BODY
#FUNCTION
RESULT:10 = 1
RETURNF 3
"#
        );
        let artifact = compile_source(&source);
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        let report = run_column_entry(&mut vm, &mut natives, &artifact, "SYSTEM_TITLE");
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. }))
        );
        assert_eq!(column_result(&vm, &artifact, 0), VmValue::Integer(expected));
        assert_eq!(column_result(&vm, &artifact, 10), VmValue::Integer(0));
    }
}

#[test]
fn column_options_default_applies_only_to_unprovided_cells_and_checks_final_nullability() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
DT_CREATE "t"
DT_COLUMN_ADD "t", "value", "int64"
DT_COLUMN_OPTIONS "t", "value", DEFAULT, -9223372036854775807 - 1
RESULT:10 = DT_ROW_ADD("t")
DT_ROW_ADD "t", "value",
RESULT:11 = DT_CELL_GET("t", 0, "value")
RESULT:12 = DT_CELL_ISNULL("t", 1, "value")
DT_COLUMN_OPTIONS "t", "value", DEFAULT, 9
RESULT:13 = DT_CELL_GET("t", 0, "value")
DT_CREATE "required"
DT_COLUMN_ADD "required", "value", "int32", 0
DT_COLUMN_OPTIONS "required", "value", DEFAULT, 7
DT_ROW_ADD "required", "value",, "value", 12
RESULT:14 = DT_CELL_GET("required", 0, "value")
DT_ROW_ADD "required", "value",
RETURN RESULT:0
"#,
    );
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let report = run_column_entry(&mut vm, &mut natives, &artifact, "SYSTEM_TITLE");
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. }))
    );
    for (index, expected) in [(11, i64::MIN), (12, 1), (13, i64::MIN), (14, 12)] {
        assert_eq!(
            column_result(&vm, &artifact, index),
            VmValue::Integer(expected)
        );
    }
}

#[test]
fn column_options_do_not_retarget_columns_recreated_by_a_value_method() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
DT_CREATE "t"
DT_COLUMN_ADD "t", "value", "int32"
DT_COLUMN_OPTIONS "t", "value", DEFAULT, 7, DEFAULT, REPLACE_COLUMN(), DEFAULT, 19
RESULT:10 = DT_ROW_ADD("t")
RESULTS:10 '= DT_CELL_GETS("t", 0, "value")
RETURN RESULT:0
@REPLACE_COLUMN
#FUNCTION
DT_COLUMN_REMOVE "t", "value"
DT_COLUMN_ADD "t", "value", "string"
DT_COLUMN_OPTIONS "t", "value", DEFAULT, "new"
RETURNF 11
"#,
    );
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    assert_column_success(&run_column_entry(
        &mut vm,
        &mut natives,
        &artifact,
        "SYSTEM_TITLE",
    ));
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS" && global.owner.is_none())
        .unwrap()
        .key;
    assert_eq!(
        vm.read_variable(results, &[10], None),
        Ok(VmValue::String("new".into()))
    );
}

#[test]
fn column_options_active_ticket_survives_a_value_method_snapshot() {
    for replace in [false, true] {
        let replacement = if replace {
            "DT_COLUMN_REMOVE \"t\", \"value\"\nDT_COLUMN_ADD \"t\", \"value\", \"int32\"\nDT_COLUMN_OPTIONS \"t\", \"value\", DEFAULT, 31\n"
        } else {
            ""
        };
        let artifact = compile_source(&format!(
            r#"@SYSTEM_TITLE
DT_CREATE "t"
DT_COLUMN_ADD "t", "value", "int32"
DT_COLUMN_OPTIONS "t", "value", DEFAULT, 7, DEFAULT, WAIT_VALUE(), DEFAULT, 19
RESULT:20 = DT_ROW_ADD("t")
RESULT:21 = DT_CELL_GET("t", 0, "value")
RETURN RESULT:0
@WAIT_VALUE
#FUNCTION
{replacement}INPUT
RETURNF 11
"#
        ));
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        vm.spawn_entry(column_entry(&artifact, "SYSTEM_TITLE"), Vec::new())
            .unwrap();
        let mut host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        assert_column_success(&report);
        let request = report
            .events
            .iter()
            .find_map(|event| match event {
                VmEvent::HostPending { request, .. } => Some(*request),
                _ => None,
            })
            .expect("value method input wait");
        let bytes = vm.snapshot(&natives).unwrap().encode().unwrap();
        let snapshot =
            VmSnapshot::decode(&bytes, VmConfig::default().maximum_snapshot_bytes).unwrap();
        let mut restored_natives = NativeServiceRegistry::for_artifact(&artifact);
        let mut restored = Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            snapshot,
            &mut host,
            &mut restored_natives,
        )
        .unwrap();
        restored.resume_host(request, HostReady::empty()).unwrap();
        assert_column_success(&restored.run_slice(
            &mut ReadyHost::default(),
            &mut restored_natives,
            RunBudget::default(),
        ));
        assert_eq!(
            column_result(&restored, &artifact, 21),
            VmValue::Integer(if replace { 31 } else { 19 })
        );
    }
}

#[test]
fn corrupted_snapshot_column_ticket_faults_on_use_without_changing_the_default() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
DT_CREATE "t"
DT_COLUMN_ADD "t", "value", "int32"
DT_COLUMN_OPTIONS "t", "value", DEFAULT, 7, DEFAULT, WAIT_VALUE()
RETURN RESULT:0
@WAIT_VALUE
#FUNCTION
INPUT
RETURNF 11
@OBSERVE
RESULT:20 = DT_ROW_ADD("t")
RESULT:21 = DT_CELL_GET("t", 0, "value")
RETURN RESULT:0
"#,
    );
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(column_entry(&artifact, "SYSTEM_TITLE"), Vec::new())
        .unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert_column_success(&report);
    let request = report
        .events
        .iter()
        .find_map(|event| match event {
            VmEvent::HostPending { request, .. } => Some(*request),
            _ => None,
        })
        .expect("value method input wait");
    let mut payload = serde_json::to_value(vm.snapshot(&natives).unwrap()).unwrap();
    let mut changed = 0;
    for fiber in payload["fibers"].as_object_mut().unwrap().values_mut() {
        for frame in fiber["frames"].as_array_mut().unwrap() {
            for value in frame["stack"].as_array_mut().unwrap() {
                if value["type"] == "string" && value["value"] == "dtc1:0000000000000002:3" {
                    value["value"] = "dtc1:0000000000000000:3".into();
                    changed += 1;
                }
            }
        }
    }
    assert_eq!(changed, 2, "retained and apply-argument ticket copies");
    let snapshot = serde_json::from_value(payload).unwrap();
    let mut restored_natives = NativeServiceRegistry::for_artifact(&artifact);
    // String operands are not scanned as capabilities during snapshot restore.
    let mut restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        snapshot,
        &mut host,
        &mut restored_natives,
    )
    .unwrap();
    restored.resume_host(request, HostReady::empty()).unwrap();
    let report = restored.run_slice(
        &mut ReadyHost::default(),
        &mut restored_natives,
        RunBudget::default(),
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. }))
    );
    assert_column_success(&run_column_entry(
        &mut restored,
        &mut restored_natives,
        &artifact,
        "OBSERVE",
    ));
    assert_eq!(column_result(&restored, &artifact, 21), VmValue::Integer(7));
}

#[test]
fn xml_replace_stored_key_overload_executes_without_rewriting_the_key() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
#DIMS XML_KEY
RESULT = XML_DOCUMENT("doc", "<root>old</root>")
RESULT:10 = XML_REPLACE("doc", "<root>expression</root>")
XML_KEY '= "doc"
XML_REPLACE XML_KEY, "<root>statement</root>"
RESULT:11 = RESULT
RESULTS:10 '= XML_KEY
RESULTS:11 '= XML_TOSTR("doc")
RETURN RESULT
"#,
    );
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(artifact.functions[0].key, Vec::new())
        .unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert_column_success(&report);
    for index in [10, 11] {
        assert_eq!(column_result(&vm, &artifact, index), VmValue::Integer(1));
    }
    let results = artifact
        .globals
        .iter()
        .find(|v| v.name == "RESULTS")
        .unwrap()
        .key;
    assert_eq!(
        vm.read_variable(results, &[10], None).unwrap(),
        VmValue::String("doc".into())
    );
    assert_eq!(
        vm.read_variable(results, &[11], None).unwrap(),
        VmValue::String("<root>statement</root>".into())
    );
}
