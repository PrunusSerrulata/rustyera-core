use super::*;

fn location(instruction: usize) -> Location {
    Location {
        generation: GenerationId(1),
        function: SymbolKey::derive("window", b"function"),
        instruction,
        context: DispatchContext {
            fiber: FiberId(1),
            frame: FrameId(1),
        },
    }
}

fn sample(profile: &mut WindowProfile, dispatch: u64, instruction: usize) {
    profile.observe(
        dispatch,
        Some(location(instruction)),
        Opcode::PushInteger as u16,
    );
}

fn json(profile: &WindowProfile, dispatches: u64) -> serde_json::Value {
    serde_json::to_value(profile.snapshot(dispatches)).unwrap()
}

#[test]
fn instruction_profile_windows_record_eight_attempts_and_readonly_pending() {
    let mut profile = WindowProfile::default();
    profile.boundary(true, 1023);
    sample(&mut profile, 1024, 10);
    assert_eq!(json(&profile, 1024), json(&profile, 1024));
    assert_eq!(
        json(&profile, 1024)["pending"]["opcodes"],
        serde_json::json!([Opcode::PushInteger as u16])
    );
    assert!(profile.counts.is_empty());
    for offset in 1..8 {
        sample(&mut profile, 1024 + offset as u64, 10 + offset);
    }
    // Completion describes attempt count, even when the eighth operation faults.
    profile.finish_dispatch(Opcode::PushInteger as u16, false, true, false);
    let snapshot = json(&profile, 1031);
    assert!(snapshot["pending"].is_null());
    assert_eq!(snapshot["counts"][0]["termination"], "length_limit");
    assert_eq!(
        snapshot["counts"][0]["opcodes"].as_array().unwrap().len(),
        8
    );
    assert_eq!(snapshot["opportunities"], "1");
    assert_eq!(snapshot["startedWindows"], "1");
}

#[test]
fn instruction_profile_windows_do_not_join_frames_fibers_functions_or_pc_jumps() {
    let initial = location(10);
    for next in [
        Location {
            context: DispatchContext {
                frame: FrameId(2),
                ..initial.context
            },
            ..location(11)
        },
        Location {
            context: DispatchContext {
                fiber: FiberId(2),
                ..initial.context
            },
            ..location(11)
        },
        Location {
            generation: GenerationId(2),
            ..location(11)
        },
        Location {
            function: SymbolKey::derive("window", b"other"),
            ..location(11)
        },
        location(10),
        location(12),
    ] {
        let mut profile = WindowProfile::default();
        profile.boundary(true, 1023);
        profile.observe(1024, Some(initial), 0);
        profile.observe(1025, Some(next), 0);
        assert_eq!(
            json(&profile, 1025)["counts"][0]["termination"],
            "discontinuous"
        );
        assert!(profile.pending.is_none());
    }
    assert!(!location(0).follows(location(usize::MAX)));
}

#[test]
fn instruction_profile_windows_end_at_control_bulk_fault_diagnostic_and_slice() {
    for (opcode, ordinary, failed, diagnostic, expected) in [
        (Opcode::JumpIfFalse, true, false, false, "control_boundary"),
        (Opcode::PushInteger, false, false, false, "control_boundary"),
        (Opcode::PushInteger, false, true, false, "fault"),
        (Opcode::Binary, true, false, true, "diagnostic"),
    ] {
        let mut profile = WindowProfile::default();
        profile.boundary(true, 1023);
        profile.observe(1024, Some(location(1)), opcode as u16);
        profile.finish_dispatch(opcode as u16, ordinary, failed, diagnostic);
        assert_eq!(json(&profile, 1024)["counts"][0]["termination"], expected);
    }
    let mut profile = WindowProfile::default();
    profile.boundary(true, 1023);
    sample(&mut profile, 1024, 1);
    profile.finish_slice();
    sample(&mut profile, 1025, 2);
    assert_eq!(
        json(&profile, 1025)["counts"][0]["termination"],
        "slice_boundary"
    );
    assert!(profile.pending.is_none());
}

#[test]
fn instruction_profile_windows_exclude_continuations_and_reset_only_at_action_begin() {
    let mut profile = WindowProfile::default();
    sample(&mut profile, 1024, 1);
    assert_eq!(profile.opportunities, 0);
    profile.boundary(true, 1024);
    profile.observe(2048, None, 0);
    sample(&mut profile, 3072, 1);
    profile.boundary(false, 3072);
    let ended = json(&profile, 3072);
    profile.boundary(false, 4000);
    assert_eq!(ended, json(&profile, 4000));
    assert_eq!(ended["counts"][0]["termination"], "action_end");
    assert_eq!(ended["excludedContinuations"], "1");
    assert_eq!(ended["opportunities"], "2");
    profile.boundary(true, 4000);
    assert!(profile.counts.is_empty());
    assert!(!profile.incomplete);
    profile.boundary(true, 4001);
    assert!(profile.incomplete && profile.restarted_while_active);
}

#[test]
fn instruction_profile_windows_capacity_and_overflow_are_explicit() {
    let mut profile = WindowProfile::default();
    profile.boundary(true, 0);
    for index in 0..=MAXIMUM_PATTERNS {
        profile.observe(
            (index as u64 + 1) * 1024,
            Some(location(0)),
            u16::try_from(index).unwrap(),
        );
        profile.finish_slice();
    }
    assert_eq!(profile.counts.len(), MAXIMUM_PATTERNS);
    assert_eq!(profile.dropped_windows, 1);
    assert!(profile.incomplete);
    profile.observe((MAXIMUM_PATTERNS as u64 + 2) * 1024, Some(location(0)), 0);
    profile.finish_slice();
    assert_eq!(profile.counts.first_key_value().unwrap().1, &2);
    profile.boundary(true, 0);
    profile.started_windows = u64::MAX;
    sample(&mut profile, 1024, 0);
    assert!(profile.incomplete);
    assert_eq!(profile.started_windows, u64::MAX);
    profile.mark_incomplete();
    let ended = json(&profile, 1024);
    profile.mark_incomplete();
    assert_eq!(ended, json(&profile, 1024));
    assert_eq!(ended["counts"][0]["termination"], "overflow");
}

#[test]
fn instruction_profile_windows_clone_and_dispatch_overflow_do_not_reuse_pending() {
    let mut profile = super::super::InstructionProfile::default();
    profile.windows.boundary(true, 1023);
    profile.dispatches = 1023;
    profile.observe_dispatch(
        GenerationId(1),
        location(0).function,
        0,
        0,
        Some(location(0).context),
    );
    assert!(profile.windows.pending.is_some());
    let clone = profile.clone();
    assert!(clone.windows.pending.is_none());
    assert!(!clone.windows.active);
    profile.dispatches = u64::MAX;
    profile.observe_dispatch(
        GenerationId(1),
        location(1).function,
        1,
        0,
        Some(location(1).context),
    );
    assert!(profile.windows.pending.is_none());
    assert!(profile.windows.incomplete);
    assert_eq!(profile.windows.started_windows, 1);
}

pub(in crate::instruction_profile) fn maximum_snapshot() -> WindowSnapshot {
    let mut profile = WindowProfile::default();
    profile.boundary(true, 0);
    for index in 0..MAXIMUM_PATTERNS {
        let mut opcodes = [u16::MAX; MAXIMUM_LENGTH];
        opcodes[0] = u16::try_from(index).unwrap();
        profile.counts.insert(
            Pattern {
                opcodes,
                length: MAXIMUM_LENGTH,
                termination: Termination::LengthLimit,
            },
            u64::MAX,
        );
    }
    profile.incomplete = true;
    profile.snapshot(u64::MAX)
}

#[test]
fn instruction_profile_windows_real_scheduler_conserves_samples_and_closes_slices() {
    struct NoHost;
    impl crate::VmHost for NoHost {
        fn call(&mut self, _: crate::HostCallRequest) -> crate::HostCallResult {
            panic!("no Host expected");
        }
    }
    let source = format!("@SYSTEM_TITLE\n{}RETURN\n", "LOCAL += 1\n".repeat(600));
    let artifact = crate::interpreter::literal_groupmatch_tests::compile_cursor_fixture(source);
    let entry = artifact.functions[0].key;
    let validated = erabasic_validator::validate_bytecode(
        artifact.clone().into_unvalidated(),
        &erabasic_validator::ValidationContext::for_artifact(&artifact),
    )
    .value
    .unwrap();
    let mut vm = crate::Vm::new(validated, crate::VmConfig::default());
    let mut natives = crate::NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(entry, Vec::new()).unwrap();
    vm.instruction_profile_boundary(true);
    let mut finished = false;
    for _ in 0..20 {
        let report = vm.run_slice(
            &mut NoHost,
            &mut natives,
            crate::RunBudget {
                maximum_instructions: 1025,
                ..crate::RunBudget::default()
            },
        );
        assert!(
            vm.instruction_profile.windows.pending.is_none(),
            "slice must close pending window"
        );
        if report.stop == crate::VmRunStop::Idle {
            assert!(
                report
                    .events
                    .iter()
                    .any(|event| matches!(event, crate::VmEvent::FiberCompleted { .. }))
            );
            finished = true;
            break;
        }
    }
    assert!(finished);
    vm.instruction_profile_boundary(false);
    let profile = &vm.instruction_profile;
    assert!(profile.windows.started_windows > 0);
    assert_eq!(profile.windows.opportunities, profile.dispatches / 1024);
    assert_eq!(
        profile.windows.counts.values().sum::<u64>(),
        profile.windows.started_windows
    );
    assert_eq!(profile.windows.excluded_continuations, 0);
    assert!(
        profile
            .windows
            .counts
            .keys()
            .any(|key| key.termination == Termination::SliceBoundary)
    );
}
