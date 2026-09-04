use super::*;
pub(super) fn match_options() -> AnalyzerOptions {
    AnalyzerOptions {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        ..AnalyzerOptions::analysis_mode()
    }
}
pub(super) fn run_match_source(source: &str) -> (BytecodeArtifact, Vm, erabasic_vm::VmRunReport) {
    let artifact = compile_source_with_options(source, &match_options());
    let entry = artifact
        .functions
        .iter()
        .find(|f| f.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    (artifact, vm, report)
}
pub(super) fn match_result(artifact: &BytecodeArtifact, vm: &Vm, index: u64) -> VmValue {
    let result = artifact
        .globals
        .iter()
        .find(|v| v.name == "RESULT" && v.owner.is_none())
        .unwrap()
        .key;
    vm.read_variable(result, &[index], None).unwrap()
}

#[test]
fn matchall_orders_ranges_before_needle_and_never_evaluates_token_indices() {
    for form in [false, true] {
        let expression = "MATCHALL(FLAG:IGNORED(), NEEDLE(), BEG(), ENDING(), OUT:IGNORED())";
        let expression = if form {
            format!("TOINT(STRFORM(\"{{{expression}}}\"))")
        } else {
            expression.into()
        };
        let source = format!(
            r"@SYSTEM_TITLE
#DIM DYNAMIC OUT, 1
FLAG:0 = 7
FLAG:1 = 8
FLAG:2 = 7
RESULT:10 = {expression}
RESULT:11 = OUT:0
RESULT:12 = FLAG:4
RESULT:13 = FLAG:5
RETURN
@IGNORED
#FUNCTION
FLAG:5 += 1
RETURNF 0
@BEG
#FUNCTION
FLAG:4 = FLAG:4 * 10 + 1
RETURNF 0
@ENDING
#FUNCTION
FLAG:4 = FLAG:4 * 10 + 2
RETURNF 3
@NEEDLE
#FUNCTION
FLAG:4 = FLAG:4 * 10 + 3
RETURNF 7
"
        );
        let (artifact, vm, report) = run_match_source(&source);
        assert!(
            !report
                .events
                .iter()
                .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        assert_eq!(
            (10..14)
                .map(|i| match_result(&artifact, &vm, i))
                .collect::<Vec<_>>(),
            vec![
                VmValue::Integer(2),
                VmValue::Integer(0),
                VmValue::Integer(123),
                VmValue::Integer(0)
            ]
        );
    }
}

#[test]
fn matchall_indexed_const_input_preserves_reference_restructure_failure() {
    let source = r"@SYSTEM_TITLE
#DIM CONST WORDS, 2 = 1, 2
RESULT:10 = MATCHALL(WORDS:0, 1)
RESULT:11 = 9
RETURN
";
    let artifact = compile_source_with_options(source, &match_options());
    let spec = artifact
        .functions
        .iter()
        .flat_map(|function| &function.code)
        .find(|instruction| {
            erabasic_bytecode::Opcode::try_from(instruction.opcode)
                == Ok(erabasic_bytecode::Opcode::BeginMatchCall)
        })
        .map(|instruction| erabasic_bytecode::MatchCallSpec::decode(&instruction.payload).unwrap())
        .expect("MATCHALL opener");
    assert!(spec.input_restructured_to_scalar);

    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );

    let (artifact, vm, report) = run_match_source(
        "@SYSTEM_TITLE\n#DIM CONST WORDS, 2 = 1, 2\nRESULT:10 = STRFORMCHECK(\"{MATCHALL(WORDS:0, 1)}\")\nRESULT:11 = 9\nRETURN\n",
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(match_result(&artifact, &vm, 10), VmValue::Integer(0));
    assert_eq!(match_result(&artifact, &vm, 11), VmValue::Integer(9));
}

#[test]
fn matchall_empty_range_still_evaluates_needle_but_reversed_range_does_not() {
    let (artifact, vm, report) = run_match_source(
        r#"@SYSTEM_TITLE
RESULT:10 = MATCHALL(FLAG, NEEDLE(), 9, 9)
RESULT:11 = STRFORMCHECK("{MATCHALL(FLAG, NEEDLE(), -1, ENDING())}")
RESULT:12 = FLAG:8
RESULT:13 = FLAG:9
RETURN
@NEEDLE
#FUNCTION
FLAG:8 += 1
RETURNF 7
@ENDING
#FUNCTION
FLAG:9 += 1
RETURNF 2
"#,
    );
    assert!(
        !report
            .events
            .iter()
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(
        (10..14)
            .map(|i| match_result(&artifact, &vm, i))
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(0),
            VmValue::Integer(0),
            VmValue::Integer(1),
            VmValue::Integer(1)
        ]
    );
}

#[test]
fn matchall_checks_output_only_on_matching_write_and_preserves_tail() {
    let (artifact, vm, report) = run_match_source(
        r#"@SYSTEM_TITLE
#DIMS DYNAMIC WRONG, 1
#DIM DYNAMIC OUT, 3
FLAG:0 = 7
FLAG:1 = 9
OUT:2 = 81
RESULT:10 = MATCHALL(FLAG, 4, 0, 2, WRONG)
RESULT:11 = STRFORMCHECK("{MATCHALL(FLAG, 7, 0, 2, WRONG)}")
MATCHALL FLAG, 7, 0, 2, OUT
RESULT:12 = RESULT
RESULT:13 = OUT:2
RETURN
"#,
    );
    assert!(
        !report
            .events
            .iter()
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(
        (10..14)
            .map(|i| match_result(&artifact, &vm, i))
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(0),
            VmValue::Integer(0),
            VmValue::Integer(1),
            VmValue::Integer(81)
        ]
    );
}

#[test]
fn matchall_string_input_and_character_input_use_live_field_zero() {
    let (artifact, vm, report) = run_match_source(
        r#"@SYSTEM_TITLE
#DIMS DYNAMIC WORDS, 3
WORDS:0 = x
WORDS:1 = z
WORDS:2 = x
RESULT:10 = MATCHALL(WORDS, "x")
DELALLCHARA
ADDVOIDCHARA
ADDVOIDCHARA
BASE:0:0 = 7
BASE:1:0 = 7
BASE:0:1 = 9
BASE:1:1 = 8
RESULT:11 = MATCHALL(BASE:0:1, 7)
RETURN
"#,
    );
    assert!(
        !report
            .events
            .iter()
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(match_result(&artifact, &vm, 10), VmValue::Integer(2));
    assert_eq!(match_result(&artifact, &vm, 11), VmValue::Integer(2));
}

#[test]
fn matchall_character_input_observes_prior_output_through_character_array_ref() {
    let (artifact, vm, report) = run_match_source(
        r"@SYSTEM_TITLE
DELALLCHARA
ADDVOIDCHARA
ADDVOIDCHARA
BASE:0:0 = 7
BASE:1:0 = 7
RESULT:10 = SCAN(BASE:1)
RETURN
@SCAN(OUT)
#FUNCTION
#DIM REF OUT, 0
RETURNF MATCHALL(BASE, 7, 0, 2, OUT)
",
    );
    assert!(
        !report
            .events
            .iter()
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(match_result(&artifact, &vm, 10), VmValue::Integer(1));
}

#[test]
fn matchallex_uses_exact_name_lookup_before_begin_side_effect() {
    for ignore_case in [false, true] {
        let source = "@SYSTEM_TITLE\nFLAG:0 = 7\nRESULT:10 = STRFORMCHECK(\"{MATCHALLEX(\\\"flag\\\", 7, BEG(), 1)}\")\nRESULT:11 = FLAG:8\nRETURN\n@BEG\n#FUNCTION\nFLAG:8 += 1\nRETURNF 0\n";
        let mut options = match_options();
        options.ignore_case = ignore_case;
        let artifact = compile_source_with_options(source, &options);
        assert_eq!(artifact.call_compatibility.ignore_case, ignore_case);
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        vm.spawn_entry(
            artifact
                .functions
                .iter()
                .find(|f| f.name == "SYSTEM_TITLE")
                .unwrap()
                .key,
            Vec::new(),
        )
        .unwrap();
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        let report = vm.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget::default(),
        );
        assert!(
            !report
                .events
                .iter()
                .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        assert_eq!(
            match_result(&artifact, &vm, 10),
            VmValue::Integer(i64::from(ignore_case))
        );
        assert_eq!(
            match_result(&artifact, &vm, 11),
            VmValue::Integer(i64::from(ignore_case))
        );
    }
}

#[test]
fn matchall_bounded_chunks_block_stable_snapshot_and_keep_one_needle() {
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nRESULT:10 = MATCHALL(FLAG, NEEDLE(), 0, 600)\nRETURN\n@NEEDLE\n#FUNCTION\nFLAG:900 += 1\nRETURNF 0\n",
        &match_options(),
    );
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(
        artifact
            .functions
            .iter()
            .find(|f| f.name == "SYSTEM_TITLE")
            .unwrap()
            .key,
        Vec::new(),
    )
    .unwrap();
    let mut reached_chunk = false;
    for _ in 0..200 {
        let report = vm.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget {
                maximum_instructions: 1,
                fiber_quantum: 1,
                ..RunBudget::default()
            },
        );
        assert!(
            report.instructions <= 1,
            "MATCH exceeded its slice budget: {report:?}"
        );
        assert!(
            !report
                .events
                .iter()
                .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        let saved = inspect_snapshot(
            &vm.encode_unrestricted_snapshot(&natives).unwrap(),
            VmConfig::default().maximum_snapshot_bytes,
        )
        .unwrap()
        .state;
        let active = saved["fibers"]
            .as_object()
            .unwrap()
            .values()
            .flat_map(|fiber| fiber["frames"].as_array().unwrap())
            .flat_map(|frame| frame["match_calls"].as_array().unwrap())
            .any(|call| call["state"]["needle"].is_object());
        if active {
            assert!(
                vm.snapshot(&natives).is_err(),
                "a runnable scan is not a stable snapshot point"
            );
            reached_chunk = true;
            break;
        }
    }
    assert!(
        reached_chunk,
        "large MATCH must yield a bounded scanner chunk"
    );
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        !report
            .events
            .iter()
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(match_result(&artifact, &vm, 10), VmValue::Integer(600));
    let flag = artifact
        .globals
        .iter()
        .find(|v| v.name == "FLAG" && v.owner.is_none())
        .unwrap()
        .key;
    assert_eq!(
        vm.read_variable(flag, &[900], None).unwrap(),
        VmValue::Integer(1)
    );
}

#[test]
fn matchall_caught_late_bounds_error_keeps_previous_output_writes() {
    let (artifact, vm, report) = run_match_source(
        r#"@SYSTEM_TITLE
#DIM DYNAMIC OUT, 3
DELALLCHARA
ADDVOIDCHARA
ADDVOIDCHARA
BASE:0:0 = 7
BASE:1:0 = 7
OUT:0 = 99
RESULT:10 = STRFORMCHECK("{MATCHALL(BASE, REMOVE_LAST(), 0, , OUT)}")
RESULT:11 = OUT:0
RETURN
@REMOVE_LAST
#FUNCTION
DELCHARA 1
RETURNF 7
"#,
    );
    assert!(
        !report
            .events
            .iter()
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(match_result(&artifact, &vm, 10), VmValue::Integer(0));
    assert_eq!(match_result(&artifact, &vm, 11), VmValue::Integer(0));
}

#[test]
#[allow(clippy::too_many_lines)] // Each corruption is checked against the same suspended needle.
fn matchall_needle_can_wait_and_restore_without_repeating_side_effects() {
    for form in [false, true] {
        let expression = "MATCHALL(FLAG, NEEDLE(), 0, 2)";
        let expression = if form {
            format!("TOINT(STRFORM(\"{{{expression}}}\"))")
        } else {
            expression.into()
        };
        let artifact = compile_source_with_options(
            &format!(
                "@SYSTEM_TITLE\nFLAG:0 = 7\nFLAG:1 = 7\nRESULT:10 = {expression}\nRETURN\n@NEEDLE\n#FUNCTION\nFLAG:8 += 1\nINPUT\nRETURNF 7\n"
            ),
            &match_options(),
        );
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        let mut host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        vm.spawn_entry(
            artifact
                .functions
                .iter()
                .find(|f| f.name == "SYSTEM_TITLE")
                .unwrap()
                .key,
            Vec::new(),
        )
        .unwrap();
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        let request = report
            .events
            .iter()
            .find_map(|event| match event {
                VmEvent::HostPending { request, .. } => Some(*request),
                _ => None,
            })
            .expect("needle waits");
        let snapshot = vm.snapshot(&natives).unwrap();
        if !form {
            let original = serde_json::to_value(&snapshot).unwrap();
            for corruption in ["needle", "cursor", "list", "owner"] {
                let mut corrupt = original.clone();
                let frames = corrupt["fibers"]
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                    .next()
                    .unwrap()["frames"]
                    .as_array_mut()
                    .unwrap();
                let frame = &mut frames[0];
                match corruption {
                    "needle" => {
                        frame["match_calls"][0]["state"]["needle"] =
                            serde_json::to_value(VmValue::Integer(7)).unwrap();
                    }
                    "cursor" => frame["match_calls"][0]["state"]["cursor"] = serde_json::json!(1),
                    "list" => frame["match_calls"] = serde_json::json!([]),
                    "owner" => {
                        frame["match_calls"][0]["state"]["input"]["owner"] =
                            serde_json::json!(9999);
                    }
                    _ => unreachable!(),
                }
                assert!(
                    Vm::restore_snapshot(
                        validated(&artifact),
                        VmConfig::default(),
                        serde_json::from_value(corrupt).unwrap(),
                        &mut PendingHost {
                            stability: HostWaitStability::StableInput,
                            rebound: Vec::new()
                        },
                        &mut NativeServiceRegistry::for_artifact(&artifact)
                    )
                    .is_err(),
                    "{corruption}"
                );
            }
        }
        let mut restored = Vm::restore_snapshot(
            validated(&artifact),
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
            !report
                .events
                .iter()
                .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        assert_eq!(match_result(&artifact, &restored, 10), VmValue::Integer(2));
        let flag = artifact
            .globals
            .iter()
            .find(|v| v.name == "FLAG" && v.owner.is_none())
            .unwrap()
            .key;
        assert_eq!(
            restored.read_variable(flag, &[8], None).unwrap(),
            VmValue::Integer(1)
        );
    }
}

#[test]
fn match_wire_validation_rejects_phase_forgery_pop_and_original_identity() {
    let base = compile_source_with_options(
        "@SYSTEM_TITLE\nRESULT:10 = MATCHALL(FLAG, 0, 0, 3)\nRETURN\n",
        &match_options(),
    );
    for corruption in ["phase", "pop", "identity"] {
        let mut artifact = base.clone();
        if corruption == "identity" {
            artifact.manifest.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraEm,
            );
        } else {
            let function = artifact
                .functions
                .iter_mut()
                .find(|f| f.name == "SYSTEM_TITLE")
                .unwrap();
            let at = function
                .code
                .iter()
                .position(|op| Opcode::try_from(op.opcode) == Ok(Opcode::MatchCallRange))
                .unwrap();
            if corruption == "phase" {
                let mut payload = function.code[at].payload.to_vec();
                payload[4] = 1;
                function.code[at] =
                    erabasic_bytecode::EncodedInstruction::new(Opcode::MatchCallRange, payload);
            } else {
                // Neither ordinary values nor POP may consume the opaque token.
                function.code[at] =
                    erabasic_bytecode::EncodedInstruction::new(Opcode::Pop, Vec::new());
            }
        }
        artifact.refresh_ids().unwrap();
        let report = validate_bytecode(
            artifact.clone().into_unvalidated(),
            &erabasic_compiler::runtime_native_validation_context(
                &artifact,
                &default_host_registry(),
            ),
        );
        assert!(report.value.is_none(), "{corruption}");
    }
}
