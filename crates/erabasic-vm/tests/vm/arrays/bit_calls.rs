use super::*;
pub(super) fn bit_options() -> AnalyzerOptions {
    AnalyzerOptions {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        ..AnalyzerOptions::default()
    }
}

fn bit_result(artifact: &BytecodeArtifact, vm: &Vm, index: u64) -> VmValue {
    let key = artifact
        .globals
        .iter()
        .find(|v| v.name == "RESULT" && v.owner.is_none())
        .unwrap()
        .key;
    vm.read_variable(key, &[index], None).unwrap()
}

#[test]
fn bit_operations_cover_word_boundaries_omission_and_method_results() {
    let artifact = compile_source_with_options(
        r"@SYSTEM_TITLE
#DIM WORDS, 2
RESULT:10 = BITSET(WORDS, 63, , 2)
RESULT:11 = WORDS:0
RESULT:12 = WORDS:1
RESULT:13 = BITGET(WORDS, 63)
BITTOGGLE WORDS, 64
RESULT:14 = RESULT
RESULT:15 = BITGET(WORDS, 64)
RESULT:16 = BITGET(WORDS, 128)
RESULT:17 = BITTOGGLE(WORDS, -1)
RESULT:18 = BITINDEXOFFIRST(WORDS, 1)
RESULT:19 = BITINDEXOFFIRST(WORDS)
RESULT:20 = BITSET(WORDS, -2, 1, 0)
RESULT:21 = BITSET(WORDS, -1, 1, 2)
RESULT:22 = BITGET(WORDS, 0)
RETURN
",
        &bit_options(),
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
    assert!(
        !report
            .events
            .iter()
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    let expected = [1, i64::MIN, 1, 1, 1, 0, -1, 0, 63, 0, 1, 1, 1];
    for (index, expected) in expected.into_iter().enumerate() {
        assert_eq!(
            bit_result(&artifact, &vm, index as u64 + 10),
            VmValue::Integer(expected),
            "slot {index}"
        );
    }
}

#[test]
fn bit_first_token_index_is_not_evaluated_and_ancestor_local_ref_is_retained() {
    for form in [false, true] {
        let expression = "BITSET(ITEMS:SIDE(), 64)";
        let expression = if form {
            format!("TOINT(STRFORM(\"{{{expression}}}\"))")
        } else {
            expression.into()
        };
        let source = format!(
            "@SYSTEM_TITLE\n#DIM WORDS, 2\nRESULT:10 = MUTATE(WORDS)\nRESULT:11 = WORDS:1\nRETURN\n@MUTATE(ITEMS)\n#FUNCTION\n#DIM REF ITEMS\nRETURNF {expression}\n@SIDE\n#FUNCTION\nFLAG:8 += 1\nRETURNF 999999\n"
        );
        let artifact = compile_source_with_options(&source, &bit_options());
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
        assert_eq!(bit_result(&artifact, &vm, 11), VmValue::Integer(1));
        let flag = artifact
            .globals
            .iter()
            .find(|v| v.name == "FLAG" && v.owner.is_none())
            .unwrap()
            .key;
        assert_eq!(
            vm.read_variable(flag, &[8], None).unwrap(),
            VmValue::Integer(0)
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)] // Corruptions share one suspended-call fixture and restore proof.
fn bit_tail_wait_snapshot_rejects_missing_forged_lease_before_host_rebind() {
    for form in [false, true] {
        let expression = "BITSET(FLAG, INDEX(), 1)";
        let expression = if form {
            format!("TOINT(STRFORM(\"{{{expression}}}\"))")
        } else {
            expression.into()
        };
        let artifact = compile_source_with_options(
            &format!(
                "@SYSTEM_TITLE\nRESULT:10 = {expression}\nRETURN\n@INDEX\n#FUNCTION\nFLAG:8 += 1\nINPUT\nRETURNF 64\n"
            ),
            &bit_options(),
        );
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        let mut host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        let fiber = vm
            .spawn_entry(
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
            .find_map(|e| match e {
                VmEvent::HostPending { request, .. } => Some(*request),
                _ => None,
            })
            .expect("tail waits");
        let snapshot = vm.snapshot(&natives).unwrap();
        let original = serde_json::to_value(&snapshot).unwrap();
        for corruption in ["lease", "owner", "length"] {
            let mut corrupt = original.clone();
            let entries = corrupt["memory"]["array_leases"]["entries"]
                .as_object_mut()
                .unwrap();
            match corruption {
                "lease" => entries.clear(),
                "owner" => {
                    entries.values_mut().next().unwrap()["owner"]["frame"] =
                        serde_json::json!(999_999);
                }
                "length" => entries.values_mut().next().unwrap()["length"] = serde_json::json!(0),
                _ => unreachable!(),
            }
            let mut rejected_host = PendingHost {
                stability: HostWaitStability::StableInput,
                rebound: Vec::new(),
            };
            let corrupt: VmSnapshot =
                serde_json::from_value(corrupt).expect("corruption must deserialize");
            assert!(
                Vm::restore_snapshot(
                    validated(&artifact),
                    VmConfig::default(),
                    corrupt,
                    &mut rejected_host,
                    &mut NativeServiceRegistry::for_artifact(&artifact)
                )
                .is_err(),
                "{corruption}"
            );
            assert!(rejected_host.rebound.is_empty());
        }
        if !form {
            let mut corrupt = original.clone();
            let frame = &mut corrupt["fibers"]
                .as_object_mut()
                .unwrap()
                .values_mut()
                .next()
                .unwrap()["frames"][0];
            frame["bit_calls"] = serde_json::json!([]);
            assert!(
                Vm::restore_snapshot(
                    validated(&artifact),
                    VmConfig::default(),
                    serde_json::from_value(corrupt).unwrap(),
                    &mut host,
                    &mut NativeServiceRegistry::for_artifact(&artifact)
                )
                .is_err()
            );
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
        let flag = artifact
            .globals
            .iter()
            .find(|v| v.name == "FLAG" && v.owner.is_none())
            .unwrap()
            .key;
        assert_eq!(
            restored.read_variable(flag, &[1], None).unwrap(),
            VmValue::Integer(1)
        );
        assert_eq!(
            restored.read_variable(flag, &[8], None).unwrap(),
            VmValue::Integer(1)
        );
        vm.cancel_fiber(fiber).unwrap();
        let cancelled = serde_json::to_value(vm.snapshot(&natives).unwrap()).unwrap();
        assert!(
            cancelled["memory"]["array_leases"]["entries"]
                .as_object()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn bit_opaque_wire_and_original_identity_are_rejected() {
    let base = compile_source_with_options(
        "@SYSTEM_TITLE\nRESULT:10 = BITSET(FLAG, 0)\nRETURN\n",
        &bit_options(),
    );
    for corruption in ["pop", "identity", "origin"] {
        let mut artifact = base.clone();
        if corruption == "identity" {
            artifact.manifest.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraEm,
            );
        } else {
            let function = &mut artifact.functions[0];
            let at = function
                .code
                .iter()
                .position(|op| Opcode::try_from(op.opcode) == Ok(Opcode::FinishBitCall))
                .unwrap();
            function.code[at] = if corruption == "pop" {
                erabasic_bytecode::EncodedInstruction::new(Opcode::Pop, Vec::new())
            } else {
                erabasic_bytecode::EncodedInstruction::new(
                    Opcode::FinishBitCall,
                    u32::try_from(at).unwrap().to_le_bytes().to_vec(),
                )
            };
        }
        artifact.refresh_ids().unwrap();
        assert!(
            validate_bytecode(
                artifact.clone().into_unvalidated(),
                &erabasic_compiler::runtime_native_validation_context(
                    &artifact,
                    &default_host_registry()
                )
            )
            .value
            .is_none(),
            "{corruption}"
        );
    }
}

#[test]
fn bit_candidate_reset_rejection_preserves_parent_backing_and_frames() {
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
            .any(|e| matches!(e, VmPortEvent::HostCall(_))),
        "{report:?}"
    );
    let before = live.encode_unrestricted_snapshot().unwrap();
    let candidate = live.fork_isolated().unwrap();
    assert!(matches!(
        candidate.prepare_runtime_state(VmRuntimeStateTransaction::ResetNewGame),
        Err(VmError::InvalidState(_))
    ));
    assert_eq!(live.encode_unrestricted_snapshot().unwrap(), before);
    // A failed prepare leaves the candidate committable with exactly its inherited roots.
    live.commit_candidate_state(candidate.into_candidate_state().unwrap())
        .unwrap();
    assert_eq!(live.encode_unrestricted_snapshot().unwrap(), before);
    let ordinary = RuntimeVm::new(validated(&artifact), VmConfig::default())
        .fork_isolated()
        .unwrap();
    ordinary
        .prepare_runtime_state(VmRuntimeStateTransaction::ResetNewGame)
        .unwrap();
}

#[test]
fn bit_unbound_ref_fails_before_index_side_effect_and_checker_continues() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIM REF ITEMS
RESULT:10 = STRFORMCHECK("{BITGET(ITEMS, SIDE())}")
RESULT:11 = 7
RETURN
@SIDE
#FUNCTION
FLAG:8 += 1
RETURNF 0
"#,
        &bit_options(),
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
    assert_eq!(bit_result(&artifact, &vm, 10), VmValue::Integer(0));
    assert_eq!(bit_result(&artifact, &vm, 11), VmValue::Integer(7));
    let flag = artifact
        .globals
        .iter()
        .find(|v| v.name == "FLAG" && v.owner.is_none())
        .unwrap()
        .key;
    assert_eq!(
        vm.read_variable(flag, &[8], None).unwrap(),
        VmValue::Integer(0)
    );
}
