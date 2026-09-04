use super::*;
#[test]
fn runtime_data_calls_use_trusted_plan_grants_without_static_import_anchors() {
    for (name, form, expected) in [
        ("map_tostring", "%MAP_TOSTRING(\"m\")%", "a=v"),
        ("bitget", "{BITGET(FLAG, 0)}", "1"),
        ("matchall", "{MATCHALL(FLAG, 1, 0, 1)}", "1"),
        ("matchallex", "{MATCHALLEX(\"FLAG\", 1, 0, 1)}", "1"),
    ] {
        let source = format!(
            "@SYSTEM_TITLE\nRESULT = MAP_CREATE(\"m\")\nRESULT = MAP_SET(\"m\", \"a\", \"v\")\nFLAG:0 = 1\nRESULTS:10 '= STRFORM({})\nRETURN RESULT\n",
            serde_json::to_string(form).unwrap()
        );
        let artifact = compile_source_with_options(&source, &method_options(true));
        assert!(
            artifact
                .native_imports
                .iter()
                .all(|native| native.import.name != name)
        );
        if name.starts_with("map_") {
            assert!(
                artifact
                    .runtime_native_authorizations
                    .iter()
                    .any(|family| family.name == name)
            );
        } else {
            assert!(
                artifact
                    .runtime_native_authorizations
                    .iter()
                    .all(|family| family.name != name)
            );
            assert!(
                artifact
                    .runtime_staged_authorizations
                    .iter()
                    .any(|family| family.name == name)
            );
        }
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{name}: {report:?}"
        );
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{name}: {report:?}"
        );
        assert_method_watch(
            &vm,
            &artifact,
            "RESULTS",
            10,
            VmValue::String(expected.into()),
        );
    }
}

#[test]
fn runtime_staged_parse_symbols_do_not_grant_execution_or_become_check_failures() {
    for name in ["bitget", "matchall"] {
        let expression = if name == "bitget" {
            "BITGET(FLAG, 0)"
        } else {
            "MATCHALL(FLAG, 0, 0, 1)"
        };
        let source = format!(
            "@SYSTEM_TITLE\nRESULT:10 = STRFORMCHECK({})\nFLAG:9 = 1\nRETURN RESULT\n",
            serde_json::to_string(&format!("{{{expression}}}")).unwrap()
        );
        let mut artifact = compile_source_with_options(&source, &method_options(true));
        artifact
            .runtime_staged_authorizations
            .retain(|family| family.name != name);
        artifact.refresh_ids().unwrap();
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert_eq!(
            take_fault(report).category,
            erabasic_vm::FaultCategory::Permission
        );
        assert_method_watch(&vm, &artifact, "FLAG", 9, VmValue::Integer(0));
    }
}

#[test]
fn runtime_data_stage_snapshot_rejects_foreign_call_site_before_native_restore() {
    for (form, task_name) in [
        ("%MAP_TOSTRING(\"m\", WAIT_S())%", "MapFinish"),
        ("{BITSET(FLAG, WAIT_I())}", "BitFinish"),
        ("{MATCHALL(FLAG, WAIT_I(), 0, 1)}", "MatchNeedle"),
    ] {
        let source = format!(
            "@SYSTEM_TITLE\nRESULT = MAP_CREATE(\"m\")\nRESULT = MAP_SET(\"m\", \"a\", \"v\")\nRESULT:9 = RAND:1000000\nRESULTS:10 '= STRFORM({})\nRETURN RESULT\n@WAIT_S\n#FUNCTIONS\nINPUT\nRETURNF \"|\"\n@WAIT_I\n#FUNCTION\nINPUT\nRETURNF 0\n",
            serde_json::to_string(form).unwrap()
        );
        let artifact = compile_source_with_options(&source, &method_options(true));
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 1234);
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .unwrap()
            .key;
        let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
        let mut host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        assert!(
            matches!(vm.fiber_status(fiber), Some(FiberStatus::WaitingHost(_))),
            "{task_name}: {report:?}"
        );
        let saved = serde_json::to_value(vm.snapshot(&natives).unwrap()).unwrap();
        let mut bad = saved.clone();
        let work = bad["fibers"][fiber.0.to_string()]["frames"][0]["runtime_form"]["work"]
            .as_array_mut()
            .unwrap();
        let call = work
            .iter_mut()
            .find_map(|task| task.get_mut(task_name))
            .expect("pending stage owns a call site");
        call["site"]["plan"] = serde_json::json!(u64::MAX);
        let mut other = NativeServiceRegistry::for_artifact_with_seed(&artifact, 9876);
        let control = Vm::new(validated(&artifact), VmConfig::default());
        let before = control.encode_unrestricted_snapshot(&other).unwrap();
        let mut rebound = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        assert!(
            Vm::restore_snapshot(
                validated(&artifact),
                VmConfig::default(),
                serde_json::from_value(bad).unwrap(),
                &mut rebound,
                &mut other
            )
            .is_err(),
            "{task_name}"
        );
        assert!(rebound.rebound.is_empty());
        assert_eq!(
            control.encode_unrestricted_snapshot(&other).unwrap(),
            before
        );
        // The untouched evidence remains restorable with the same pending site.
        let Some(FiberStatus::WaitingHost(request)) = vm.fiber_status(fiber) else {
            unreachable!()
        };
        let mut restored = Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            serde_json::from_value(saved).unwrap(),
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
            report.events.iter().any(|event| matches!(
                event,
                VmEvent::FiberCompleted {
                    fiber: completed,
                    value: Some(VmValue::Integer(1))
                } if *completed == fiber
            )),
            "{task_name}: {report:#?}"
        );
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{task_name}: {report:#?}"
        );
    }
}

#[test]
fn staged_grants_roundtrip_but_cannot_be_forged_from_parse_metadata() {
    let artifact = compile_source_with_options("@SYSTEM_TITLE\nRETURN\n", &method_options(true));
    let context =
        erabasic_compiler::runtime_native_validation_context(&artifact, &default_host_registry());
    let bytes = erabasic_bytecode::encode_artifact(&artifact).unwrap();
    let decoded =
        erabasic_bytecode::decode_artifact(&bytes, &erabasic_bytecode::DecodeLimits::default())
            .unwrap();
    assert!(
        erabasic_validator::validate_bytecode(decoded, &context)
            .value
            .is_some()
    );
    for attack in ["name", "kind", "shape"] {
        let mut forged = artifact.clone();
        let family = forged
            .runtime_staged_authorizations
            .iter_mut()
            .find(|family| family.name == "bitset")
            .unwrap();
        match attack {
            "name" => family.name = "bitget".into(),
            "kind" => family.kind = erabasic_bytecode::RuntimeStagedKind::MatchAll,
            "shape" => {
                family.shapes[0].arguments[0] = erabasic_bytecode::RuntimeArgumentConstraint::Any;
            }
            _ => unreachable!(),
        }
        family.key = family.canonical_key();
        forged.refresh_ids().unwrap();
        let report = erabasic_validator::validate_bytecode(forged.into_unvalidated(), &context);
        assert!(report.value.is_none(), "{attack}");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code
                    == erabasic_validator::ValidationCode::HostAbiMismatch),
            "{attack}: {report:?}"
        );
    }
}
