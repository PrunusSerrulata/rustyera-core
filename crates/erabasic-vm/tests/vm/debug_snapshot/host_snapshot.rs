use super::*;
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

#[test]
fn runtime_host_call_plan_obeys_runnable_and_transient_snapshot_boundaries() {
    struct RuntimeInputHost;
    impl VmHost for RuntimeInputHost {
        fn call(&mut self, request: HostCallRequest) -> HostCallResult {
            if request.import.name.eq_ignore_ascii_case("__GETKEY_ACTIVE") {
                HostCallResult::Ready(HostReady {
                    value: Some(VmValue::Integer(1)),
                    writes: Vec::new(),
                })
            } else {
                assert!(request.import.name.eq_ignore_ascii_case("GETKEY"));
                HostCallResult::Deferred
            }
        }
    }
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nRESULTS:1 '= \"{GETKEY(7)}\"\nRESULTS:0 '= STRFORM(RESULTS:1)\nFLAG:0 = STRFORMCHECK(RESULTS:0)\nRETURN RESULT\n",
        &AnalyzerOptions {
            compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            ),
            ..AnalyzerOptions::default()
        },
    );
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm
        .spawn_entry(artifact.functions[0].key, Vec::new())
        .unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget {
            maximum_host_calls: 0,
            ..RunBudget::default()
        },
    );
    assert!(
        matches!(vm.fiber_status(fiber), Some(FiberStatus::Runnable)),
        "{report:?}"
    );
    assert!(matches!(
        vm.snapshot_eligibility(&natives),
        SnapshotEligibility::Ineligible(ref blockers)
            if blockers.contains(&SnapshotBlocker::RunnableFiber(fiber))
    ));
    assert!(vm.encode_snapshot(&natives).is_err());

    let mut pending = RuntimeInputHost;
    vm.run_slice(&mut pending, &mut natives, RunBudget::default());
    let Some(FiberStatus::WaitingHost(request)) = vm.fiber_status(fiber) else {
        panic!("runtime Host request was not issued");
    };
    assert!(matches!(
        vm.snapshot_eligibility(&natives),
        SnapshotEligibility::Ineligible(ref blockers)
            if blockers.contains(&SnapshotBlocker::TransientHostWait(fiber))
    ));
    assert!(vm.encode_snapshot(&natives).is_err());
    vm.resume_host(
        request,
        HostReady {
            value: Some(VmValue::Integer(42)),
            writes: Vec::new(),
        },
    )
    .unwrap();
    vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::Completed(_))
    ));
    let flag = artifact
        .globals
        .iter()
        .find(|value| value.name == "FLAG")
        .unwrap()
        .key;
    assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(1)));
}
