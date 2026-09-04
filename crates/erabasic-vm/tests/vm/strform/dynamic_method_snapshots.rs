use super::*;
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Keep both serialized continuation shapes and their corruption matrix in one scenario."
)]
fn suspended_dynamic_method_snapshots_validate_origin_generation_slots_and_form_bindings() {
    let source = r#"@SYSTEM_TITLE
RESULT:0 = GETMETH("SNAP_PAIR", , 2, SNAP_INPUT())
RETURN RESULT:0
@FORM_TITLE
RESULTS:0 '= STRFORM("{GETMETH(\"SNAP_PAIR\", , 2, SNAP_INPUT())}")
RETURN
@SNAP_PAIR(LEFT, RIGHT)
#FUNCTION
#DIM DYNAMIC LEFT
#DIM DYNAMIC RIGHT
RETURNF LEFT * 10 + RIGHT
@SNAP_INPUT
#FUNCTION
INPUT
RETURNF RESULT
"#;
    let artifact = compile_source_with_options(source, &method_options(true));
    for entry_name in ["SYSTEM_TITLE", "FORM_TITLE"] {
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == entry_name)
            .unwrap()
            .key;
        let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 123_456);
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
            .unwrap_or_else(|| panic!("{entry_name}: expected stable input, got {report:?}"));
        let snapshot = vm.snapshot(&natives).unwrap();
        let json = serde_json::to_value(&snapshot).unwrap();
        let mut cancelled = vm.clone();
        cancelled.cancel_fiber(fiber).unwrap();
        assert!(cancelled.resume_host(request, HostReady::empty()).is_err());
        let cancelled = serde_json::to_value(cancelled.snapshot(&natives).unwrap()).unwrap();
        assert!(
            cancelled["fibers"][fiber.0.to_string()]["frames"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        for corruption in ["origin", "generation", "slot", "target", "capture"] {
            let mut corrupted = json.clone();
            let frame = &mut corrupted["fibers"][fiber.0.to_string()]["frames"][0];
            if entry_name == "SYSTEM_TITLE" {
                let pending = &mut frame["user_calls"][0];
                match corruption {
                    "origin" => pending["resolve"] = serde_json::json!(usize::MAX),
                    "generation" => pending["call"]["generation"] = serde_json::json!(999),
                    "slot" => pending["next_slot"] = serde_json::json!(999),
                    "target" => {
                        pending["call"]["function"] = serde_json::to_value(entry).unwrap();
                    }
                    "capture" => {
                        pending["captured"][0] =
                            serde_json::to_value(VmValue::String("forged".into())).unwrap();
                    }
                    _ => unreachable!(),
                }
            } else {
                let continuation = &mut frame["runtime_form"];
                let call = continuation["work"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find_map(|task| task.get_mut("CaptureMethodArgument"))
                    .expect("suspended STRFORM capture");
                match corruption {
                    "origin" => call["call"]["function"] = serde_json::to_value(entry).unwrap(),
                    "generation" => call["call"]["generation"] = serde_json::json!(999),
                    "slot" => call["next_slot"] = serde_json::json!(999),
                    "target" => call["call"]["bindings"] = serde_json::json!([]),
                    "capture" => {
                        call["captured"][0] =
                            serde_json::to_value(VmValue::String("forged".into())).unwrap();
                    }
                    _ => unreachable!(),
                }
            }
            let corrupted: VmSnapshot = serde_json::from_value(corrupted).unwrap();
            let mut rejected_host = PendingHost {
                stability: HostWaitStability::StableInput,
                rebound: Vec::new(),
            };
            let before = vm.encode_snapshot(&natives).unwrap();
            assert!(
                Vm::restore_snapshot(
                    validated(&artifact),
                    VmConfig::default(),
                    corrupted,
                    &mut rejected_host,
                    &mut natives
                )
                .is_err(),
                "{entry_name}/{corruption}"
            );
            assert!(
                rejected_host.rebound.is_empty(),
                "invalid method state must be rejected before host rebind"
            );
            assert_eq!(vm.encode_snapshot(&natives).unwrap(), before);
        }
        let mut restored = Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            snapshot,
            &mut host,
            &mut natives,
        )
        .unwrap();
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
            &mut natives,
            RunBudget::default(),
        );
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{entry_name}: {report:?}"
        );
        if entry_name == "SYSTEM_TITLE" {
            assert_method_watch(&restored, &artifact, "RESULT", 0, VmValue::Integer(23));
        } else {
            assert_method_watch(
                &restored,
                &artifact,
                "RESULTS",
                0,
                VmValue::String("23".into()),
            );
        }
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Keep the suspended REF fixture, corruption matrix and forwarding restoration assertions together."
)]
fn active_ref_method_snapshots_restore_local_arrays_and_reject_invalid_aliases() {
    let source = METHOD_FIXTURE_SOURCE.to_owned()
        + r#"
@REF_SNAPSHOT_TITLE
#DIM DYNAMIC LOCAL_NUMBERS, 3
#DIMS DYNAMIC LOCAL_WORDS, 3
LOCAL_NUMBERS:0 = 10
LOCAL_WORDS:1 '= "before"
RESULT:0 = GETMETH("SNAP_FORWARD", , LOCAL_NUMBERS, LOCAL_WORDS)
RESULT:1 = LOCAL_NUMBERS:0
RESULTS:0 '= LOCAL_WORDS:1
RETURN RESULT:0
@SNAP_FORWARD(NUMBERS, TEXTS)
#FUNCTION
#DIM REF NUMBERS
#DIMS REF TEXTS
RETURNF GETMETH("SNAP_WAIT", , NUMBERS, TEXTS)
@SNAP_WAIT(NUMBERS, TEXTS)
#FUNCTION
#DIM REF NUMBERS
#DIMS REF TEXTS
INPUT
NUMBERS:0 += 1
TEXTS:1 '= "restored"
RETURNF NUMBERS:0
"#;
    let source = source
        + r#"
@FORM_REF_SNAPSHOT_TITLE
#DIM DYNAMIC LOCAL_NUMBERS, 3
#DIMS DYNAMIC LOCAL_WORDS, 3
LOCAL_NUMBERS:0 = 10
LOCAL_WORDS:1 '= "before"
RESULT:0 = TOINT(STRFORM("{GETMETH(\"SNAP_FORWARD\", , LOCAL_NUMBERS, LOCAL_WORDS)}"))
RESULT:1 = LOCAL_NUMBERS:0
RESULTS:0 '= LOCAL_WORDS:1
RETURN RESULT:0
"#;
    let header = METHOD_FIXTURE_HEADER.to_owned() + "\n#DIM CONST SNAP_LOCKED, 3 = 1, 2, 3\n";
    let artifact = compile_with_header(&header, &source, &method_options(true));
    for entry_name in ["REF_SNAPSHOT_TITLE", "FORM_REF_SNAPSHOT_TITLE"] {
        let function = |name| {
            artifact
                .functions
                .iter()
                .find(|function| function.name == name)
                .unwrap()
        };
        let entry = function(entry_name).key;
        let forward = function("SNAP_FORWARD");
        let target = function("SNAP_WAIT");
        let parameter_key = |slot: usize| {
            serde_json::to_value(target.parameters[slot].key)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        };
        let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 123_456);
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
            .unwrap_or_else(|| panic!("expected active REF input: {report:?}"));
        let snapshot = serde_json::to_value(vm.snapshot(&natives).unwrap()).unwrap();
        let frames = &snapshot["fibers"][fiber.0.to_string()]["frames"];
        assert_eq!(frames.as_array().unwrap().len(), 3);
        for corruption in [
            "fiber",
            "owner",
            "generation",
            "type",
            "rank",
            "indices",
            "character",
            "character_source",
            "backing",
            "immutable",
            "cycle",
            "cell_type",
            "cell_shape",
        ] {
            let mut corrupted = snapshot.clone();
            let frames = &mut corrupted["fibers"][fiber.0.to_string()]["frames"];
            let target_id = frames[2]["id"].clone();
            let cell = &mut frames[2]["locals"][parameter_key(0)];
            let place = &mut cell[2]["IntegerPlaces"][0][1];
            match corruption {
                "fiber" => place["fiber"] = serde_json::json!(999),
                "owner" => place["frame"] = serde_json::json!(999),
                "generation" => frames[0]["generation"] = serde_json::json!(999),
                "type" => {
                    place["variable"] =
                        serde_json::to_value(named_key(&artifact, "METHOD_WORDS")).unwrap();
                    place["frame"] = serde_json::Value::Null;
                }
                "rank" => {
                    place["variable"] =
                        serde_json::to_value(named_key(&artifact, "METHOD_MATRIX")).unwrap();
                    place["frame"] = serde_json::Value::Null;
                }
                "indices" => place["indices"] = serde_json::json!([0]),
                "character" => place["character"] = serde_json::json!(0),
                "backing" => place["backing"] = serde_json::json!(u64::MAX),
                "character_source" => {
                    place["variable"] =
                        serde_json::to_value(named_key(&artifact, "CFLAG")).unwrap();
                    place["frame"] = serde_json::Value::Null;
                }
                "immutable" => {
                    place["variable"] =
                        serde_json::to_value(named_key(&artifact, "SNAP_LOCKED")).unwrap();
                    place["frame"] = serde_json::Value::Null;
                }
                "cycle" => {
                    place["variable"] = serde_json::to_value(target.parameters[0].key).unwrap();
                    place["frame"] = target_id;
                }
                "cell_type" => {
                    let places = cell[2]["IntegerPlaces"].take();
                    cell[0] = serde_json::to_value(BytecodeType::StringPlace).unwrap();
                    cell[2] = serde_json::json!({"StringPlaces": places});
                }
                "cell_shape" => cell[1] = serde_json::json!([2]),
                _ => unreachable!(),
            }
            let corrupted: VmSnapshot = serde_json::from_value(corrupted).unwrap();
            let before = vm.encode_snapshot(&natives).unwrap();
            let mut rejected_host = PendingHost {
                stability: HostWaitStability::StableInput,
                rebound: Vec::new(),
            };
            assert!(
                Vm::restore_snapshot(
                    validated(&artifact),
                    VmConfig::default(),
                    corrupted,
                    &mut rejected_host,
                    &mut natives
                )
                .is_err(),
                "{corruption}"
            );
            assert!(
                rejected_host.rebound.is_empty(),
                "{corruption}: invalid alias rebound the host"
            );
            assert_eq!(
                vm.encode_snapshot(&natives).unwrap(),
                before,
                "{corruption}: native state changed"
            );
        }
        // Normalized bindings and a valid explicit forwarding chain must both retain the caller's arrays.
        for forwarding_chain in [false, true] {
            let mut snapshot = snapshot.clone();
            if forwarding_chain {
                let frames = &mut snapshot["fibers"][fiber.0.to_string()]["frames"];
                let owner = frames[1]["id"].clone();
                for (slot, storage) in ["IntegerPlaces", "StringPlaces"].into_iter().enumerate() {
                    let place = &mut frames[2]["locals"][parameter_key(slot)][2][storage][0][1];
                    place["variable"] = serde_json::to_value(forward.parameters[slot].key).unwrap();
                    place["frame"] = owner.clone();
                }
            }
            let snapshot: VmSnapshot = serde_json::from_value(snapshot).unwrap();
            let mut restored_host = PendingHost {
                stability: HostWaitStability::StableInput,
                rebound: Vec::new(),
            };
            let mut restored = Vm::restore_snapshot(
                validated(&artifact),
                VmConfig::default(),
                snapshot,
                &mut restored_host,
                &mut natives,
            )
            .unwrap();
            assert!(!restored_host.rebound.is_empty());
            restored.resume_host(request, HostReady::empty()).unwrap();
            let report = restored.run_slice(
                &mut ReadyHost::default(),
                &mut natives,
                RunBudget::default(),
            );
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "forwarding={forwarding_chain}: {report:?}"
            );
            assert_method_watch(&restored, &artifact, "RESULT", 0, VmValue::Integer(11));
            assert_method_watch(&restored, &artifact, "RESULT", 1, VmValue::Integer(11));
            assert_method_watch(
                &restored,
                &artifact,
                "RESULTS",
                0,
                VmValue::String("restored".into()),
            );
        }
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Keep the pending-expression corruption matrix beside the valid restore control."
)]
fn runtime_form_snapshot_rejects_invalid_pending_operator_types_before_external_restore() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULTS:0 '= STRFORM("{GETMETH(\"SNAP_PAIR\", , SNAP_INPUT(), 2 + 3)}")
RETURN
@SNAP_PAIR(LEFT, RIGHT)
#FUNCTION
#DIM DYNAMIC LEFT
#DIM DYNAMIC RIGHT
RETURNF LEFT * 10 + RIGHT
@SNAP_INPUT
#FUNCTION
INPUT
RETURNF RESULT
"#,
        &method_options(true),
    );
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 123_456);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm
        .spawn_entry(
            artifact
                .functions
                .iter()
                .find(|function| function.name == "SYSTEM_TITLE")
                .unwrap()
                .key,
            Vec::new(),
        )
        .unwrap();
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
        .unwrap_or_else(|| panic!("expected suspended argument: {report:?}"));
    let snapshot = serde_json::to_value(vm.snapshot(&natives).unwrap()).unwrap();
    for corruption in [
        "binary",
        "unary",
        "condition",
        "comparison",
        "increment",
        "postfix",
    ] {
        let mut corrupted = snapshot.clone();
        let work = corrupted["fibers"][fiber.0.to_string()]["frames"][0]["runtime_form"]["work"]
            .as_array_mut()
            .unwrap();
        let call = work
            .iter_mut()
            .find_map(|task| task.get_mut("CaptureMethodArgument"))
            .unwrap();
        let argument = &mut call["arguments"][1];
        let span = argument["span"].clone();
        let integer = serde_json::json!({"kind": {"Integer": 1}, "span": span});
        let string = serde_json::json!({"kind": {"String": "wrong"}, "span": span});
        argument["kind"] = match corruption {
            "binary" => {
                serde_json::json!({"Binary": {"op": "Subtract", "left": string, "right": integer}})
            }
            "unary" => serde_json::json!({"Unary": {"op": "Minus", "operand": string}}),
            "condition" => {
                serde_json::json!({"Ternary": {"condition": string, "then_expr": integer, "else_expr": integer}})
            }
            "comparison" => {
                serde_json::json!({"Binary": {"op": "Equal", "left": string, "right": integer}})
            }
            "increment" => serde_json::json!({"Unary": {"op": "PreIncrement", "operand": integer}}),
            "postfix" => serde_json::json!({"Postfix": {"op": "Increment", "operand": integer}}),
            _ => unreachable!(),
        };
        let corrupted: VmSnapshot = serde_json::from_value(corrupted).unwrap();
        let before = vm.encode_snapshot(&natives).unwrap();
        let mut rejected_host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        assert!(
            Vm::restore_snapshot(
                validated(&artifact),
                VmConfig::default(),
                corrupted,
                &mut rejected_host,
                &mut natives
            )
            .is_err(),
            "{corruption}"
        );
        assert!(rejected_host.rebound.is_empty());
        assert_eq!(vm.encode_snapshot(&natives).unwrap(), before);
    }
    let mut restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        serde_json::from_value(snapshot).unwrap(),
        &mut host,
        &mut natives,
    )
    .unwrap();
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
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(
        &restored,
        &artifact,
        "RESULTS",
        0,
        VmValue::String("35".into()),
    );
}

#[test]
fn dynamic_method_const_ref_rejection_does_not_evaluate_immutable_index() {
    let header = METHOD_FIXTURE_HEADER.to_owned() + "\n#DIM CONST METHOD_LOCKED, 3 = 1, 2, 3\n";
    let mut source = METHOD_FIXTURE_SOURCE.to_owned();
    for (index, actual) in ["METHOD_LOCKED:METHOD_INDEX()"].iter().enumerate() {
        let expression =
            format!("GETMETH(METHOD_NAME(\"METHOD_REF_INT\"), METHOD_FALLBACK(), {actual})");
        write!(
            source,
            "\n@REF_REJECT_{index}\nCALL METHOD_RESET\nRESULT:0 = {expression}\nRETURN\n"
        )
        .unwrap();
        let escaped = expression.replace('"', "\\\"");
        write!(source, "\n@FORM_REF_REJECT_{index}\nCALL METHOD_RESET\nRESULTS:0 '= STRFORM(\"{{{escaped}}}\")\nRETURN\n").unwrap();
    }
    let artifact = compile_with_header(&header, &source, &method_options(true));
    for entry in ["REF_REJECT_0", "FORM_REF_REJECT_0"] {
        let (vm, report) = run_method_case(&artifact, entry, VmConfig::default());
        assert_eq!(
            take_fault(report).code,
            VmFaultCode::TypeMismatch,
            "{entry}"
        );
        for (name, expected) in [
            ("METHOD_TRACE", 1),
            ("METHOD_BODY_COUNT", 0),
            ("METHOD_INDEX_COUNT", 0),
        ] {
            assert_method_watch(&vm, &artifact, name, 0, VmValue::Integer(expected));
        }
        assert_method_watch(&vm, &artifact, "METHOD_LOCKED", 0, VmValue::Integer(1));
    }
}

#[test]
fn discarded_method_tokens_leave_no_pending_snapshot_state() {
    let (mut artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    let target_key = SymbolKey::derive("test.method", b"discarded");
    let mut target = function(
        target_key,
        "DISCARDED_METHOD",
        vec![opcode::push_integer(42), opcode::return_value(true)],
    );
    target.kind = erabasic_bytecode::BytecodeFunctionKind::Method;
    target.result = Some(BytecodeType::Integer);
    artifact.functions.push(target);
    artifact
        .functions
        .iter_mut()
        .find(|function| function.key == entry)
        .unwrap()
        .code
        .splice(
            0..0,
            [
                opcode::push_string("MISSING_METHOD"),
                opcode::resolve_user_call(&erabasic_bytecode::UserCallSpec {
                    mode: erabasic_bytecode::UserCallMode::MethodInteger,
                    allow_missing: true,
                    missing_target: 5,
                    arguments: Vec::new(),
                }),
                opcode::invoke_user_call(1),
                erabasic_bytecode::EncodedInstruction::new(Opcode::Pop, Vec::new()),
                opcode::jump(Opcode::Jump, 6),
                opcode::abandon_user_call(1),
            ],
        );
    artifact.refresh_ids().unwrap();
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 123_456);
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::HostPending { .. })),
        "{report:?}"
    );
    let snapshot = vm.snapshot(&natives).unwrap();
    let json = serde_json::to_value(&snapshot).unwrap();
    assert!(
        json["fibers"][fiber.0.to_string()]["frames"][0]["user_calls"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            snapshot,
            &mut host,
            &mut natives
        )
        .is_ok()
    );
}

#[test]
fn resolved_method_metadata_obeys_the_vm_operand_limit() {
    let artifact = compile_with_header(
        METHOD_FIXTURE_HEADER,
        METHOD_FIXTURE_SOURCE,
        &method_options(true),
    );
    let (vm, report) = run_method_case(
        &artifact,
        "METHOD_CASE_TRAILING_DEFAULTS",
        VmConfig {
            maximum_operand_stack: 3,
            ..VmConfig::default()
        },
    );
    assert_eq!(take_fault(report).code, VmFaultCode::ResourceLimit);
    assert_method_watch(&vm, &artifact, "METHOD_BODY_COUNT", 0, VmValue::Integer(0));
}

#[track_caller]
pub(super) fn take_fault(report: erabasic_vm::VmRunReport) -> erabasic_vm::VmFault {
    let debug = format!("{report:#?}");
    report
        .events
        .into_iter()
        .find_map(|event| match event {
            VmEvent::FiberFaulted { fault, .. } => Some(fault),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected a VM fault, got {debug}"))
}
