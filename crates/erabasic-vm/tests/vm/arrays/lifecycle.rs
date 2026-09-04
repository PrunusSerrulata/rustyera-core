use super::*;
#[test]
fn nested_bit_candidate_commit_preserves_inherited_roots_and_parent_atomicity() {
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nRESULT:10 = BITSET(FLAG, INDEX())\nRETURN\n@INDEX\n#FUNCTION\nINPUT\nRETURNF 64\n",
        &bit_options(),
    );
    let mut live = RuntimeVm::new(validated(&artifact), VmConfig::default());
    live.spawn_entry(
        artifact
            .functions
            .iter()
            .find(|f| f.name == "SYSTEM_TITLE")
            .unwrap()
            .key,
        Vec::new(),
    )
    .unwrap();
    let report = live.drive(RunBudget::default(), VmDriveMode::Normal);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmPortEvent::HostCall(_))),
        "{report:?}"
    );
    let before = live.encode_unrestricted_snapshot().unwrap();
    let mut outer = live.fork_isolated().unwrap();
    let mut inner = outer.fork_isolated().unwrap();
    assert!(matches!(
        inner.prepare_runtime_state(VmRuntimeStateTransaction::ResetNewGame),
        Err(VmError::InvalidState(_))
    ));
    let save = inner.export_era_state();
    assert!(matches!(
        inner.restore_era_state(&save),
        Err(VmError::InvalidState(_))
    ));
    outer
        .commit_candidate_state(inner.into_candidate_state().unwrap())
        .unwrap();
    assert_eq!(live.encode_unrestricted_snapshot().unwrap(), before);
    live.commit_candidate_state(outer.into_candidate_state().unwrap())
        .unwrap();
    assert_eq!(live.encode_unrestricted_snapshot().unwrap(), before);
}

#[test]
fn copychara_invalidates_prepared_candidate_even_when_cell_revisions_match() {
    let artifact = compile_source_with_options(
        r"@SYSTEM_TITLE
DELALLCHARA
ADDVOIDCHARA
ADDVOIDCHARA
CFLAG:0:0 = 7
CFLAG:1:0 = 19
IF 0
CALL COPY_ROW
ENDIF
RESULT:10 = KEEP(CFLAG:1:0, WAIT_INDEX())
RETURN
@KEEP(VALUES, DUMMY)
#FUNCTION
#DIM REF VALUES
#DIM DUMMY
RETURNF VALUES:0
@WAIT_INDEX
#FUNCTION
INPUT
RETURNF 0
@COPY_ROW
COPYCHARA 0, 1
RETURN
",
        &bit_options(),
    );
    let mut live = RuntimeVm::new(validated(&artifact), VmConfig::default());
    live.spawn_entry(
        artifact
            .functions
            .iter()
            .find(|f| f.name == "SYSTEM_TITLE")
            .unwrap()
            .key,
        Vec::new(),
    )
    .unwrap();
    let report = live.drive(RunBudget::default(), VmDriveMode::Normal);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmPortEvent::HostCall(_))),
        "{report:?}"
    );
    let candidate = live
        .fork_isolated()
        .unwrap()
        .into_candidate_state()
        .unwrap();
    let copy = live
        .spawn_entry(
            artifact
                .functions
                .iter()
                .find(|f| f.name == "COPY_ROW")
                .unwrap()
                .key,
            Vec::new(),
        )
        .unwrap();
    let report = live.drive(RunBudget::default(), VmDriveMode::Normal);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmPortEvent::FiberCompleted(id, _) if *id == copy)),
        "{report:?}"
    );
    let current = live.encode_unrestricted_snapshot().unwrap();
    let flag = artifact
        .globals
        .iter()
        .find(|variable| variable.name == "CFLAG")
        .unwrap()
        .key;
    assert_eq!(
        live.read_runtime_state(&[erabasic_vm::VmRuntimeRead {
            variable: flag,
            indices: vec![0],
            character: Some(1)
        }])
        .unwrap(),
        vec![VmValue::Integer(7)]
    );
    assert!(matches!(
        live.commit_candidate_state(candidate),
        Err(VmError::InvalidState(_))
    ));
    assert_eq!(live.encode_unrestricted_snapshot().unwrap(), current);
}

#[test]
fn match_scan_zero_first_failure_and_partial_failure_charge_exact_work() {
    // This CHECK enters a user method so MATCH uses the bytecode scanner, not
    // RuntimeForm's already-single-row task. Range length is captured before
    // SHRINK executes. Row0 writes remain visible when a later row faults.
    for (end, remove, expected_check, expected_out) in [
        ("0", "DELALLCHARA", 1, 99),
        ("", "DELALLCHARA", 0, 99),
        ("", "DELCHARA 1", 0, 0),
    ] {
        let source = format!(
            r#"@SYSTEM_TITLE
#DIM DYNAMIC OUT, 3
DELALLCHARA
ADDVOIDCHARA
ADDVOIDCHARA
BASE:0:0 = 7
BASE:1:0 = 7
OUT:0 = 99
RESULT:10 = STRFORMCHECK("{{SCAN(OUT)}}")
RESULT:11 = OUT:0
RESULT:12 = 17
RETURN
@SCAN(OUTPUT)
#FUNCTION
#DIM REF OUTPUT
RETURNF MATCHALL(BASE, SHRINK(), 0, {end}, OUTPUT)
@SHRINK
#FUNCTION
{remove}
RETURNF 7
"#
        );
        let artifact = compile_source_with_options(&source, &match_options());
        let entry = artifact
            .functions
            .iter()
            .find(|f| f.name == "SYSTEM_TITLE")
            .unwrap()
            .key;
        let mut totals = Vec::new();
        for maximum in [1, 128] {
            let mut vm = Vm::new(validated(&artifact), VmConfig::default());
            let mut natives = NativeServiceRegistry::for_artifact(&artifact);
            let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
            let mut total = 0;
            let mut completed = false;
            for _ in 0..2000 {
                let report = vm.run_slice(
                    &mut ReadyHost::default(),
                    &mut natives,
                    RunBudget {
                        maximum_instructions: maximum,
                        fiber_quantum: u32::try_from(maximum).unwrap(),
                        ..RunBudget::default()
                    },
                );
                assert!(report.instructions <= maximum, "{source}\n{report:?}");
                assert!(
                    !report
                        .events
                        .iter()
                        .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
                    "{source}\n{report:?}"
                );
                total += report.instructions;
                if matches!(vm.fiber_status(fiber), Some(FiberStatus::Completed(_))) {
                    completed = true;
                    break;
                }
            }
            assert!(
                completed,
                "CHECK must continue after a catchable scan error"
            );
            assert_eq!(
                match_result(&artifact, &vm, 10),
                VmValue::Integer(expected_check)
            );
            assert_eq!(
                match_result(&artifact, &vm, 11),
                VmValue::Integer(expected_out)
            );
            assert_eq!(match_result(&artifact, &vm, 12), VmValue::Integer(17));
            totals.push(total);
        }
        assert_eq!(
            totals[0], totals[1],
            "coalescing must count the same completed/failed rows: {source}"
        );
    }
}

// Fixed .NET 8 ICU72 name-casing regression candidate; not yet run.
// Expectations are source-derived for the .NET 8 ICU path, not captured oracle goldens.
#[test]
fn matchallex_unicode_name_casing_does_not_casefold_array_values() {
    for (declared, lookup, equal) in [
        ("ÉTAT", "état", true),
        ("ΣNAME", "ςname", true),
        ("ΜNAME", "µname", true),
        ("I_NAME", "ı_name", false),
        ("S_NAME", "ſ_name", false),
        ("K_NAME", "\u{212a}_name", false),
        ("ẞNAME", "ßname", false),
        ("ᾈNAME", "ᾀname", true),
        ("𐐀NAME", "𐐨name", true),
        ("ÉNAME", "E\u{301}name", false),
    ] {
        for ignore_case in [false, true] {
            let source = format!(
                "@SYSTEM_TITLE\n#DIMS DYNAMIC {declared}, 1\n{declared}:0 '= \"É\"\nRESULT:10 = STRFORMCHECK(\"{{MATCHALLEX(\\\"{lookup}\\\", \\\"é\\\", BEG(), 1)}}\")\nRESULT:11 = FLAG:8\nRETURN\n@BEG\n#FUNCTION\nFLAG:8 += 1\nRETURNF 0\n"
            );
            let mut options = match_options();
            options.ignore_case = ignore_case;
            let artifact = compile_source_with_options(&source, &options);
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
            let name_exists = ignore_case && equal;
            assert_eq!(
                match_result(&artifact, &vm, 10),
                VmValue::Integer(i64::from(name_exists))
            );
            assert_eq!(
                match_result(&artifact, &vm, 11),
                VmValue::Integer(i64::from(name_exists))
            );
        }
    }
    // Successful lookup must still compare array string elements ordinally.
    let (artifact, vm, report) = run_match_source(
        "@SYSTEM_TITLE\n#DIMS DYNAMIC ÉTAT, 1\nÉTAT:0 '= \"É\"\nRESULT:10 = MATCHALLEX(\"état\", \"é\", 0, 1)\nRETURN\n",
    );
    assert!(
        !report
            .events
            .iter()
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(match_result(&artifact, &vm, 10), VmValue::Integer(0));
}
