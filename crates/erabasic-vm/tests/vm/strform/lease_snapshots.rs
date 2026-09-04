use super::*;
fn pending_lease_snapshot_artifact() -> BytecodeArtifact {
    compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIM DYNAMIC VALUES, 2
VALUES:0 = 5
RESULT:8 = RAND:1000000
RESULT:6 = GETMETH("VALUE_LEASE", , 1, 2)
IF FLAG:7
RESULT:8 = ABS(FLAG:7)
ENDIF
RESULT:10 = GETMETH("PAIR_LEASE", , VALUES, GETMETH("VALUE_LEASE", , 2, WAIT_LEASE()))
FLAG:9 = VALUES:0
RETURN
@PAIR_LEASE(ITEMS, RIGHT)
#FUNCTION
#DIM REF ITEMS
#DIM DYNAMIC RIGHT
ITEMS:0 += 4
RETURNF ITEMS:0 * 100 + RIGHT
@VALUE_LEASE(LEFT, RIGHT)
#FUNCTION
#DIM DYNAMIC LEFT
#DIM DYNAMIC RIGHT
RETURNF LEFT * 10 + RIGHT
@WAIT_LEASE
#FUNCTION
INPUT
RETURNF RESULT:0
@PROBE_TITLE
RESULT:10 = GETMETH("IDENTITY_LEASE", , EXISTVAR(PROBE_LEASE_SOURCE(), 1))
FLAG:9 = 1
RETURN
@IDENTITY_LEASE(VALUE)
#FUNCTION
#DIM DYNAMIC VALUE
RETURNF VALUE
@PROBE_LEASE_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
INPUT
ENDIF
RETURNF "FLAG:0"
@JUMP_TITLE
CALL JUMP_OWNER
FLAG:9 = 1
RETURN
@JUMP_OWNER
#DIM DYNAMIC VALUES, 2
VALUES:0 = 5
JUMPFORM JUMP_WAIT(VALUES)
FLAG:8 = 1
RETURN
@JUMP_WAIT(ITEMS)
#DIM REF ITEMS
INPUT
ITEMS:0 += 1
RESULT:10 = ITEMS:0
RETURN
@FAULT_TITLE
RESULT:10 = GETMETH("VALUE_LEASE", , 1, 1 + FLAG:99999999)
RETURN
@SNAPSHOT_WAIT
INPUT
RETURN
"#,
        &method_options(true),
    )
}

pub(super) fn lease_snapshot_natives(
    artifact: &BytecodeArtifact,
) -> (
    NativeServiceRegistry,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct RestoreCounter(Arc<AtomicUsize>);
    impl NativeService for RestoreCounter {
        fn call(
            &mut self,
            _: NativeCallRequest,
        ) -> Result<NativeReady, erabasic_vm::ExecutionFailure> {
            Ok(NativeReady {
                value: Some(VmValue::Integer(0)),
                writes: Vec::new(),
            })
        }
        fn snapshot(&self) -> Result<Option<Vec<u8>>, String> {
            Ok(Some(vec![17]))
        }
        fn restore(&mut self, bytes: &[u8]) -> Result<(), String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            if bytes != [17] {
                return Err("invalid restore-counter state".into());
            }
            Ok(())
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(artifact, 123_456);
    let key = artifact
        .native_imports
        .iter()
        .find(|native| native.import.name.eq_ignore_ascii_case("ABS"))
        .unwrap()
        .import
        .key;
    natives.register(key, RestoreCounter(Arc::clone(&calls)));
    (natives, calls)
}

#[test]
#[allow(clippy::too_many_lines)] // Keep each fixture and its lifecycle assertions together.
fn pending_call_snapshot_lease_list_must_match_validated_cfg_before_native_restore() {
    use std::sync::atomic::Ordering;
    let artifact = pending_lease_snapshot_artifact();
    for entry_name in ["SYSTEM_TITLE", "PROBE_TITLE"] {
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == entry_name)
            .unwrap()
            .key;
        let (mut natives, _) = lease_snapshot_natives(&artifact);
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
        let mut host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        let request = report
            .events
            .iter()
            .find_map(|event| match event {
                VmEvent::HostPending { request, .. } => Some(*request),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{entry_name}: {report:?}"));
        let snapshot = vm.snapshot(&natives).unwrap();
        let original = serde_json::to_value(&snapshot).unwrap();
        let frame = &original["fibers"][fiber.0.to_string()]["frames"][0];
        assert_eq!(
            frame["user_calls"].as_array().unwrap().len(),
            if entry_name == "SYSTEM_TITLE" { 2 } else { 1 }
        );
        assert_eq!(
            frame["existvar_checks"].as_array().unwrap().len(),
            usize::from(entry_name == "PROBE_TITLE")
        );
        let mut attacks = vec![
            "delete_calls",
            "delete_calls_and_stack",
            "duplicate_call",
            "operand_budget",
        ];
        if entry_name == "SYSTEM_TITLE" {
            attacks.extend(["rewind_progress", "forge_compatible_origin", "forge_ref"]);
        } else {
            attacks.extend(["delete_probe", "duplicate_probe", "forge_probe_origin"]);
        }
        for attack in attacks {
            let mut corrupted = original.clone();
            let frame = &mut corrupted["fibers"][fiber.0.to_string()]["frames"][0];
            match attack {
                "delete_calls" => frame["user_calls"] = serde_json::json!([]),
                "delete_calls_and_stack" => {
                    frame["user_calls"] = serde_json::json!([]);
                    frame["stack"] = serde_json::json!([]);
                }
                "duplicate_call" => {
                    let copy = frame["user_calls"][0].clone();
                    frame["user_calls"].as_array_mut().unwrap().push(copy);
                }
                "rewind_progress" => {
                    // These fields remain mutually consistent; only the verified IP
                    // proves that the first retained actual was already captured.
                    frame["user_calls"][1]["next_slot"] = serde_json::json!(0);
                    frame["user_calls"][1]["captured"] = serde_json::json!([]);
                }
                "forge_compatible_origin" => {
                    let current =
                        usize::try_from(frame["user_calls"][1]["resolve"].as_u64().unwrap())
                            .unwrap();
                    let code = &artifact
                        .functions
                        .iter()
                        .find(|function| function.key == entry)
                        .unwrap()
                        .code;
                    let earlier = code[..current]
                        .iter()
                        .position(|instruction| {
                            instruction.opcode == Opcode::ResolveUserCall as u16
                                && instruction.payload == code[current].payload
                        })
                        .expect("same-shape earlier call");
                    frame["user_calls"][1]["resolve"] = serde_json::json!(earlier);
                }
                "forge_ref" => {
                    frame["user_calls"][0]["captured"][0] =
                        serde_json::to_value(VmValue::IntegerPlace(Box::default())).unwrap();
                }
                "delete_probe" => frame["existvar_checks"] = serde_json::json!([]),
                "duplicate_probe" => {
                    let copy = frame["existvar_checks"][0].clone();
                    frame["existvar_checks"].as_array_mut().unwrap().push(copy);
                }
                "forge_probe_origin" => frame["existvar_checks"][0]["begin"] = serde_json::json!(0),
                "operand_budget" => {}
                _ => unreachable!(),
            }
            let corrupted: VmSnapshot = serde_json::from_value(corrupted).unwrap();
            let (mut rejected_natives, restored_calls) = lease_snapshot_natives(&artifact);
            let before = vm.encode_snapshot(&rejected_natives).unwrap();
            let mut rejected_host = PendingHost {
                stability: HostWaitStability::StableInput,
                rebound: Vec::new(),
            };
            let mut config = VmConfig::default();
            if attack == "operand_budget" {
                config.maximum_operand_stack = 1;
            }
            assert!(
                matches!(
                    Vm::restore_snapshot(
                        validated(&artifact),
                        config,
                        corrupted,
                        &mut rejected_host,
                        &mut rejected_natives
                    ),
                    Err(VmError::Snapshot(_))
                ),
                "{entry_name}/{attack}"
            );
            assert_eq!(
                restored_calls.load(Ordering::SeqCst),
                0,
                "{entry_name}/{attack}: Native restore was invoked"
            );
            assert!(rejected_host.rebound.is_empty(), "{entry_name}/{attack}");
            assert_eq!(
                vm.encode_snapshot(&rejected_natives).unwrap(),
                before,
                "{entry_name}/{attack}"
            );
        }
        let (mut restored_natives, restored_calls) = lease_snapshot_natives(&artifact);
        let mut restored = Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            snapshot,
            &mut host,
            &mut restored_natives,
        )
        .unwrap();
        assert_eq!(restored_calls.load(Ordering::SeqCst), 1);
        restored
            .write_variable(
                named_key(&artifact, "RESULT"),
                &[0],
                None,
                VmValue::Integer(3),
            )
            .unwrap();
        restored.resume_host(request, HostReady::empty()).unwrap();
        let report = restored.run_slice(
            &mut ReadyHost::default(),
            &mut restored_natives,
            RunBudget::default(),
        );
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{report:?}"
        );
        assert_method_watch(
            &restored,
            &artifact,
            "RESULT",
            10,
            VmValue::Integer(if entry_name == "SYSTEM_TITLE" { 923 } else { 1 }),
        );
        assert_method_watch(
            &restored,
            &artifact,
            "FLAG",
            9,
            VmValue::Integer(if entry_name == "SYSTEM_TITLE" { 9 } else { 1 }),
        );
        if entry_name == "PROBE_TITLE" {
            assert_method_watch(&restored, &artifact, "FLAG", 0, VmValue::Integer(2));
        }
    }
}

#[test]
fn pending_jump_snapshot_uses_validated_terminal_stack_and_keeps_local_ref_alive() {
    let artifact = pending_lease_snapshot_artifact();
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "JUMP_TITLE")
        .unwrap()
        .key;
    let (mut natives, _) = lease_snapshot_natives(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    let request = report
        .events
        .iter()
        .find_map(|event| match event {
            VmEvent::HostPending { request, .. } => Some(*request),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{report:?}"));
    let snapshot = vm.snapshot(&natives).unwrap();
    let json = serde_json::to_value(&snapshot).unwrap();
    let owner = &json["fibers"][fiber.0.to_string()]["frames"][1];
    let function = serde_json::from_value(owner["function"].clone()).unwrap();
    let instruction = usize::try_from(owner["instruction"].as_u64().unwrap()).unwrap();
    let validated = validated(&artifact);
    assert!(
        validated
            .operand_stacks()
            .before(function, instruction)
            .is_none()
    );
    assert!(
        validated
            .operand_stacks()
            .terminal_user_call(function, instruction - 1)
            .is_some()
    );
    let mut restored = Vm::restore_snapshot(
        validated,
        VmConfig::default(),
        snapshot,
        &mut host,
        &mut natives,
    )
    .unwrap();
    restored.resume_host(request, HostReady::empty()).unwrap();
    let report = restored.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert_method_watch(&restored, &artifact, "RESULT", 10, VmValue::Integer(6));
    assert_method_watch(&restored, &artifact, "FLAG", 8, VmValue::Integer(0));
    assert_method_watch(&restored, &artifact, "FLAG", 9, VmValue::Integer(1));
}

#[test]
fn faulted_snapshot_keeps_partial_operand_diagnostics_but_no_active_leases() {
    let artifact = pending_lease_snapshot_artifact();
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "FAULT_TITLE")
        .unwrap()
        .key;
    let (mut natives, _) = lease_snapshot_natives(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    let fault = take_fault(report);
    assert_eq!(fault.code, VmFaultCode::Bounds);
    // A faulted primary is deliberately not a stable save point. The existing
    // VM contract can retain faulted secondary fibers beside a stable input root.
    assert!(vm.snapshot(&natives).is_err());
    let root = artifact
        .functions
        .iter()
        .find(|function| function.name == "SNAPSHOT_WAIT")
        .unwrap()
        .key;
    let primary = vm.spawn_entry(root, Vec::new()).unwrap();
    vm.set_primary_fiber(primary).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    let snapshot = vm.snapshot(&natives).unwrap();
    let json = serde_json::to_value(&snapshot).unwrap();
    for frame in json["fibers"][fiber.0.to_string()]["frames"]
        .as_array()
        .unwrap()
    {
        assert_eq!(frame["user_calls"], serde_json::json!([]));
        assert_eq!(frame["existvar_checks"], serde_json::json!([]));
        assert!(frame["runtime_form"].is_null());
    }
    let restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        snapshot,
        &mut host,
        &mut natives,
    )
    .unwrap();
    assert_eq!(
        restored.fiber_status(fiber),
        Some(FiberStatus::Faulted(fault))
    );
}

#[test]
fn strformcheck_catches_special_native_domains_and_retains_prior_effects() {
    for body in [
        "ARRAYREMOVE FLAG, -1, 1",
        "VARSET FLAG, 3, 0, 9999999",
        "ARRAYCOPY \"MISSING_ARRAY\", \"FLAG\"",
        "ADDCHARA 9999999",
        "DELCHARA -1",
        "RESULT:9 = CSVNAME(9999999) == \"unused\"",
        "RESULT:9 = SUMARRAY(RESULTS, 0, 1)",
        "RESULT:9 = FINDELEMENT(FLAG, \"wrong scalar type\", 0, 1)",
    ] {
        let source = format!(
            "@SYSTEM_TITLE\nRESULT:0 = STRFORMCHECK(\"{{FAILURE()}}\")\nRESULT:1 = FLAG:0\nRETURN\n@FAILURE\n#FUNCTION\nFLAG:0 += 1\n{body}\nFLAG:1 = 1\nRETURNF 99\n"
        );
        let artifact = compile_source_with_options(&source, &method_options(true));
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{body}: {report:?}"
        );
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{body}: {report:?}"
        );
        assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(0));
        assert_method_watch(&vm, &artifact, "RESULT", 1, VmValue::Integer(1));
        assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
    }
}

#[test]
fn strformcheck_randdata_failure_is_atomic_and_keeps_the_original_rng() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
DUMPRAND
FLAG:2 = RANDDATA:624
RESULT:0 = STRFORMCHECK("{BAD_STATE()}")
DUMPRAND
RESULT:1 = RANDDATA:624 == FLAG:2
RESULT:2 = FLAG:0
RETURN
@BAD_STATE
#FUNCTION
FLAG:0 += 1
RANDDATA:624 = 625
INITRAND
FLAG:1 = 1
RETURNF 99
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    for (index, expected) in [(0, 0), (1, 1), (2, 1)] {
        assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(expected));
    }
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
}

#[test]
fn strformcheck_does_not_catch_a_missing_random_provider() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = STRFORMCHECK("{DUMP_STATE()}")
FLAG:1 = 1
RETURN
@DUMP_STATE
#FUNCTION
FLAG:0 += 1
DUMPRAND
RETURNF 99
"#,
        &method_options(true),
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut NativeServiceRegistry::default(),
        RunBudget::default(),
    );
    let fault = take_fault(report);
    assert_eq!(
        fault.category,
        erabasic_vm::FaultCategory::InternalInvariant
    );
    assert_eq!(fault.code, erabasic_vm::VmFaultCode::Native);
    assert_eq!(fault.message, "random native service is not registered");
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(1));
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
}

#[test]
fn strformcheck_preserves_special_native_success_sentinels() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = STRFORMCHECK("{NO_MATCH()}")
RESULT:1 = FLAG:0
RETURN RESULT:0
@NO_MATCH
#FUNCTION
ARRAYREMOVE FLAG, 9999999, 1
PICKUPCHARA -1
FLAG:0 = GETCHARA(9999999)
RETURNF FLAG:0
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(1));
    assert_method_watch(&vm, &artifact, "RESULT", 1, VmValue::Integer(-1));
}

#[test]
fn arrayshift_extreme_offsets_do_not_panic_inside_a_checked_method() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
FLAG:0 = -9223372036854775807 - 1
RESULT:0 = STRFORMCHECK("{SHIFT()}")
RESULT:1 = TFLAG:0
RETURN RESULT:0
@SHIFT
#FUNCTION
TFLAG:0 = 3
ARRAYSHIFT TFLAG, FLAG:0, 9, 0, 1
RETURNF TFLAG:0
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(1));
    assert_method_watch(&vm, &artifact, "RESULT", 1, VmValue::Integer(9));
}
// Append to the existing tests/vm/strform.rs; not a new test module.
