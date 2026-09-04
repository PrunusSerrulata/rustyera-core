use super::*;
#[derive(Default)]
struct DirectFormHost {
    requests: Vec<HostCallRequest>,
    transient: bool,
}
impl VmHost for DirectFormHost {
    fn call(&mut self, request: HostCallRequest) -> HostCallResult {
        assert!(request.import.name.eq_ignore_ascii_case("GETKEY"));
        self.requests.push(request);
        if self.transient {
            HostCallResult::Pending {
                stability: HostWaitStability::Transient,
                rebind_payload: Vec::new(),
            }
        } else {
            HostCallResult::Ready(HostReady {
                value: Some(VmValue::Integer(42)),
                writes: Vec::new(),
            })
        }
    }
}

#[test]
fn direct_runtime_host_without_static_import_uses_ready_and_single_completion_paths() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"{GETKEY(7)}\"\nRESULT = STRFORM(RESULTS:0) == \"42\"\nRETURN RESULT\n",
    );
    assert!(
        !artifact
            .host_imports
            .iter()
            .any(|import| import.import.name.eq_ignore_ascii_case("GETKEY"))
    );
    assert!(
        artifact
            .runtime_host_authorizations
            .iter()
            .any(|family| family.name == "getkey")
    );
    for transient in [false, true] {
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let fiber = vm
            .spawn_entry(artifact.functions[0].key, Vec::new())
            .unwrap();
        let mut host = DirectFormHost {
            requests: Vec::new(),
            transient,
        };
        let mut report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        assert_eq!(host.requests.len(), 1);
        assert_eq!(host.requests[0].arguments, vec![VmValue::Integer(7)]);
        assert!(host.requests[0].omitted_arguments.is_empty());
        assert_eq!(
            host.requests[0].origin.command.to_ascii_lowercase(),
            "getkey"
        );
        if transient {
            assert!(matches!(
                vm.fiber_status(fiber),
                Some(FiberStatus::WaitingHost(_))
            ));
            assert!(
                vm.snapshot(&natives).is_err(),
                "transient Host service is not a stable input"
            );
            let request = host.requests[0].id;
            vm.resume_host(
                request,
                HostReady {
                    value: Some(VmValue::Integer(42)),
                    writes: Vec::new(),
                },
            )
            .unwrap();
            assert!(
                vm.resume_host(
                    request,
                    HostReady {
                        value: Some(VmValue::Integer(99)),
                        writes: Vec::new()
                    }
                )
                .is_err()
            );
            report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        }
        assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
        assert!(
            report.events.iter().any(|event| matches!(
                event,
                VmEvent::FiberCompleted {
                    fiber: completed,
                    value: Some(VmValue::Integer(1))
                } if *completed == fiber
            )),
            "{:#?}",
            report.events
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
            host.requests.len(),
            1,
            "resumption must not call Host twice"
        );
        assert_eq!(
            vm.read_variable(named_key(&artifact, "RESULT"), &[0], None),
            Ok(VmValue::Integer(1))
        );
    }
}

#[test]
fn direct_runtime_html_lines_empty_source_skips_width_and_host_service() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"{HTML_STRINGLINES(\\\"\\\", WIDTH())}\"\nRESULTS:10 '= STRFORM(RESULTS:0)\nFLAG:9 = 1\nRETURN\n@WIDTH\n#FUNCTION\nFLAG:8 += 1\nRETURNF 0\n",
    );
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(artifact.functions[0].key, Vec::new())
        .unwrap();
    let mut host = DirectFormHost::default();
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:#?}"
    );
    assert!(host.requests.is_empty());
    assert_method_watch(&vm, &artifact, "RESULTS", 10, VmValue::String("0".into()));
    assert_method_watch(&vm, &artifact, "FLAG", 8, VmValue::Integer(0));
    assert_method_watch(&vm, &artifact, "FLAG", 9, VmValue::Integer(1));
}

#[test]
fn direct_host_authorization_is_not_inferred_from_catalog_or_forged_artifact() {
    let mut artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"{GETKEY(7)}\"\nRESULT = STRFORM(RESULTS:0) == \"42\"\nRETURN RESULT\n",
    );
    let context =
        erabasic_compiler::runtime_native_validation_context(&artifact, &default_host_registry());
    let family = artifact
        .runtime_host_authorizations
        .iter_mut()
        .find(|family| family.name == "getkey")
        .unwrap();
    family.prototype.capability = erabasic_bytecode::HostCapability::Network;
    family.key = family.canonical_key();
    artifact.refresh_ids().unwrap();
    assert!(
        validate_bytecode(artifact.into_unvalidated(), &context)
            .value
            .is_none()
    );
}

#[test]
fn direct_host_budget_stops_before_issue_without_repeating_argument_effects() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"{GETKEY(SIDE())}\"\nRESULT = STRFORM(RESULTS:0) == \"42\"\nRETURN\n@SIDE\n#FUNCTION\nFLAG:0 += 1\nRETURNF 7\n",
    );
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = DirectFormHost::default();
    let paused = vm.run_slice(
        &mut host,
        &mut natives,
        RunBudget {
            maximum_host_calls: 0,
            ..RunBudget::default()
        },
    );
    assert!(host.requests.is_empty(), "{paused:#?}");
    assert_eq!(
        paused.stop,
        erabasic_vm::VmRunStop::BudgetExhausted,
        "{paused:#?}"
    );
    assert!(
        matches!(vm.fiber_status(fiber), Some(FiberStatus::Runnable)),
        "{paused:#?}"
    );
    assert_eq!(
        vm.read_variable(named_key(&artifact, "FLAG"), &[0], None),
        Ok(VmValue::Integer(1))
    );
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    completed_without_fault(&report, fiber);
    assert_eq!(host.requests.len(), 1);
    assert_eq!(
        vm.read_variable(named_key(&artifact, "FLAG"), &[0], None),
        Ok(VmValue::Integer(1))
    );
}

#[test]
fn direct_host_wrong_ready_type_or_write_is_not_script_catchable() {
    struct WrongReady {
        variable: SymbolKey,
        wrong_type: bool,
    }
    impl VmHost for WrongReady {
        fn call(&mut self, _: HostCallRequest) -> HostCallResult {
            HostCallResult::Ready(if self.wrong_type {
                HostReady {
                    value: Some(VmValue::String("not-an-integer".into())),
                    writes: Vec::new(),
                }
            } else {
                HostReady {
                    value: Some(VmValue::Integer(42)),
                    writes: vec![
                        erabasic_vm::HostWrite {
                            target: erabasic_vm::PlaceDescriptor {
                                variable: self.variable,
                                indices: vec![8],
                                ..Default::default()
                            },
                            value: VmValue::Integer(99),
                        },
                        erabasic_vm::HostWrite {
                            target: erabasic_vm::PlaceDescriptor {
                                variable: SymbolKey::derive("test", b"missing-host-write"),
                                ..Default::default()
                            },
                            value: VmValue::Integer(2),
                        },
                    ],
                }
            })
        }
    }
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"{GETKEY(7)}\"\nFLAG:0 = 73\nFLAG:8 = 17\nFLAG:0 = STRFORMCHECK(RESULTS:0)\nFLAG:1 = 1\nRETURN RESULT\n",
        &method_options(true),
    );
    for wrong_type in [true, false] {
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        vm.spawn_entry(artifact.functions[0].key, Vec::new())
            .unwrap();
        let mut host = WrongReady {
            variable: named_key(&artifact, "FLAG"),
            wrong_type,
        };
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        assert_eq!(
            take_fault(report).category,
            erabasic_vm::FaultCategory::HostContract
        );
        assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(73));
        assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
        assert_method_watch(&vm, &artifact, "FLAG", 8, VmValue::Integer(17));
    }
}

#[test]
fn native_write_batch_follows_ref_backing_and_rejects_invalid_sibling_atomically() {
    struct RefWriter {
        invalid_sibling: bool,
    }
    impl NativeService for RefWriter {
        fn call(
            &mut self,
            request: NativeCallRequest,
        ) -> Result<NativeReady, erabasic_vm::ExecutionFailure> {
            let mut target = request.places[0].target.clone();
            target.indices.push(0);
            let mut writes = vec![erabasic_vm::HostWrite {
                target,
                value: VmValue::String("written".into()),
            }];
            if self.invalid_sibling {
                writes.push(erabasic_vm::HostWrite {
                    target: erabasic_vm::PlaceDescriptor {
                        variable: SymbolKey::derive("test", b"missing-ref-sibling"),
                        ..Default::default()
                    },
                    value: VmValue::String("invalid".into()),
                });
            }
            Ok(NativeReady {
                value: Some(VmValue::Integer(1)),
                writes,
            })
        }
    }

    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIMS OUT, 2
OUT:0 '= "before"
CALL WRITE_REF, OUT
RETURN
@WRITE_REF(VALUES)
#DIMS REF VALUES
RESULT = DT_COLUMN_NAMES("table", VALUES)
RETURN
"#,
        &method_options(true),
    );
    let key = artifact
        .native_imports
        .iter()
        .find(|native| native.import.name.eq_ignore_ascii_case("DT_COLUMN_NAMES"))
        .unwrap()
        .import
        .key;
    for invalid_sibling in [false, true] {
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        natives.register(key, RefWriter { invalid_sibling });
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let fiber = vm
            .spawn_entry(artifact.functions[0].key, Vec::new())
            .unwrap();
        let report = vm.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget::default(),
        );
        if invalid_sibling {
            assert!(matches!(
                vm.fiber_status(fiber),
                Some(FiberStatus::Faulted(_))
            ));
            assert_method_watch(&vm, &artifact, "OUT", 0, VmValue::String("before".into()));
        } else {
            completed_without_fault(&report, fiber);
            assert_method_watch(&vm, &artifact, "OUT", 0, VmValue::String("written".into()));
        }
    }
}

#[test]
fn direct_host_request_after_reload_uses_its_prior_generation_authorization() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"{GETKEY(7)}\"\nRESULT = STRFORM(RESULTS:0) == \"42\"\nRETURN RESULT\n",
    );
    let mut target = artifact.clone();
    target
        .runtime_host_authorizations
        .retain(|family| family.name != "getkey");
    target.refresh_ids().unwrap();
    let mut runtime = RuntimeVm::new(validated(&artifact), VmConfig::default());
    let fiber = runtime
        .spawn_entry(artifact.functions[0].key, Vec::new())
        .unwrap();
    let old_generation = runtime.current_generation();
    let report = runtime.drive(
        RunBudget {
            maximum_host_calls: 0,
            ..RunBudget::default()
        },
        VmDriveMode::Normal,
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmPortEvent::HostCall(_)))
    );
    runtime.prepare_hot_reload(validated(&target)).unwrap();
    runtime.commit_hot_reload().unwrap();
    assert_ne!(runtime.current_generation(), old_generation);
    let report = runtime.drive(RunBudget::default(), VmDriveMode::Normal);
    let request = report
        .events
        .into_iter()
        .find_map(|event| match event {
            VmPortEvent::HostCall(request) => Some(request),
            _ => None,
        })
        .expect("old frame must retain the grant missing from the current generation");
    assert_eq!(request.origin.generation, old_generation);
    assert_eq!(request.arguments, vec![VmValue::Integer(7)]);
    assert!(runtime.host_request_scope(request.id).is_some());
    let completion = runtime
        .validate_host_completion(
            request.id,
            VmHostCompletion::Ready(HostReady {
                value: Some(VmValue::Integer(42)),
                writes: Vec::new(),
            }),
        )
        .unwrap();
    runtime.commit_host_completion(completion).unwrap();
    runtime.drive(RunBudget::default(), VmDriveMode::Normal);
    assert_eq!(
        runtime.fiber_status(fiber),
        Some(FiberStatus::Completed(Some(VmValue::Integer(1))))
    );
}
