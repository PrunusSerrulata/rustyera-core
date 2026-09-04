use super::*;
#[test]
fn ref_unwind_retires_child_capture_and_jump_keeps_live_ancestor_backing() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIM DYNAMIC ITEMS, 2
ITEMS:0 = 10
RESULT:0 = STRFORMCHECK("{FAIL_REF(ITEMS)}")
CALL JUMP_REF, ITEMS
FLAG:9 = ITEMS:0
RETURN RESULT
@FAIL_REF(VALUES)
#FUNCTION
#DIM REF VALUES
VALUES:0 += 1
RETURNF VALUES:999999
@JUMP_REF(VALUES)
#DIM REF VALUES
JUMP INCREMENT_REF, VALUES
@INCREMENT_REF(VALUES)
#DIM REF VALUES
VALUES:0 += 2
RETURN
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
    assert_method_watch(&vm, &artifact, "FLAG", 9, VmValue::Integer(13));
}

#[test]
fn runtime_variable_metadata_includes_private_ref_and_survives_codec_patch() {
    let artifact = compile_source_with_options(
        r"@SYSTEM_TITLE
#DIM REF PRIVATE_ARRAY
RETURN
",
        &method_options(true),
    );
    let variable = artifact
        .globals
        .iter()
        .find(|variable| variable.name == "PRIVATE_ARRAY")
        .unwrap();
    assert!(
        artifact
            .runtime_variables
            .iter()
            .find(|metadata| metadata.key == variable.key)
            .unwrap()
            .reference
    );
    let bytes = erabasic_bytecode::encode_artifact(&artifact).unwrap();
    let decoded =
        erabasic_bytecode::decode_artifact(&bytes, &erabasic_bytecode::DecodeLimits::default())
            .unwrap();
    let decoded = erabasic_validator::validate_bytecode(
        decoded,
        &erabasic_compiler::runtime_native_validation_context(&artifact, &default_host_registry()),
    )
    .value
    .unwrap();
    assert_eq!(
        decoded.artifact().runtime_variables,
        artifact.runtime_variables
    );
    let mut changed = artifact.clone();
    changed
        .runtime_variables
        .iter_mut()
        .find(|metadata| metadata.key == variable.key)
        .unwrap()
        .reference = false;
    changed.refresh_ids().unwrap();
    assert_ne!(
        changed.manifest.program_version.execution_id,
        artifact.manifest.program_version.execution_id
    );
    let patch = erabasic_bytecode::create_patch(&artifact, &changed);
    assert_eq!(
        erabasic_bytecode::apply_patch(&artifact, &patch)
            .unwrap()
            .runtime_variables,
        changed.runtime_variables
    );
    for corruption in ["missing", "duplicate", "wrong_disposal"] {
        let mut broken = artifact.clone();
        match corruption {
            "missing" => {
                broken.runtime_variables.pop();
            }
            "duplicate" => {
                broken
                    .runtime_variables
                    .push(broken.runtime_variables[0].clone());
            }
            _ => {
                let entry = broken
                    .runtime_variables
                    .iter_mut()
                    .find(|metadata| metadata.key == variable.key)
                    .unwrap();
                entry.character_disposal = erabasic_bytecode::CharacterArrayDisposal::ClearSparse;
            }
        }
        assert!(
            validate_bytecode(
                broken.into_unvalidated(),
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
fn public_zero_length_user_name_metadata_is_not_omitted_by_schema_lookup() {
    let mut data = project_data();
    data.schema.variables.insert(
        "ZERO_USER".into(),
        erabasic_data::VariableSchema {
            id: erabasic_data::VariableId::user("ZERO_USER"),
            value_type: erabasic_data::ValueType::Integer,
            storage: erabasic_data::StorageScope::Normal,
            dimensions: vec![0],
            mutable: true,
            persistence: erabasic_data::Persistence::None,
            can_forbid: false,
        },
    );
    let artifact = compile_source_with_data_and_options(
        "@SYSTEM_TITLE\nRETURN\n",
        data,
        &method_options(true),
    );
    let variable = artifact
        .globals
        .iter()
        .find(|variable| variable.name == "ZERO_USER")
        .unwrap();
    let metadata = artifact
        .runtime_variables
        .iter()
        .find(|metadata| metadata.key == variable.key)
        .unwrap();
    assert_eq!(
        metadata.match_name_rejection,
        Some(erabasic_bytecode::MatchNameRejectionKind::Internal)
    );
}

#[test]
fn ref_to_bit_keeps_detached_character_backing_after_a_later_argument_deletes_it() {
    for dynamic in [false, true] {
        let operation = if dynamic {
            "RESULTS:10 '= STRFORM(\"{BITSET(VALUES, DELETE_SELECTED())}\")"
        } else {
            "RESULT:10 = BITSET(VALUES, DELETE_SELECTED())"
        };
        let source = format!(
            r"@SYSTEM_TITLE
DELALLCHARA
ADDVOIDCHARA
ADDVOIDCHARA
CFLAG:0:0 = 17
CFLAG:1:0 = 99
RESULT:11 = MUTATE(CFLAG:0:0)
RESULT:12 = CFLAG:0:0
RETURN
@MUTATE(VALUES)
#FUNCTION
#DIM REF VALUES
{operation}
RETURNF VALUES:0
@DELETE_SELECTED
#FUNCTION
FLAG:8 += 1
DELCHARA 0
RETURNF 1
"
        );
        let artifact = compile_source_with_options(&source, &method_options(true));
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{dynamic}: {report:?}"
        );
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{dynamic}: {report:?}"
        );
        if dynamic {
            assert_method_watch(&vm, &artifact, "RESULTS", 10, VmValue::String("1".into()));
        } else {
            assert_method_watch(&vm, &artifact, "RESULT", 10, VmValue::Integer(1));
        }
        // Snake disposal clears this built-in sparse array first. BITSET bit1
        // then writes2 to the detached object shared by the REF and BIT leases.
        assert_method_watch(&vm, &artifact, "RESULT", 11, VmValue::Integer(2));
        assert_method_watch(&vm, &artifact, "RESULT", 12, VmValue::Integer(99));
        assert_method_watch(&vm, &artifact, "FLAG", 8, VmValue::Integer(1));
    }
}

#[test]
#[allow(clippy::too_many_lines)] // One matrix proves rank, storage and profile behavior together.
fn user_character_string_and_two_dimensional_ref_preserve_deleted_backing() {
    for snake in [false, true] {
        for dynamic in [false, true] {
            for string in [false, true] {
                let (header, initialize, expression, method, after, expected) = if string {
                    (
                        "#DIMS CHARADATA USER_TEXT, 2\n",
                        "USER_TEXT:0:0 '= \"kept\"\nUSER_TEXT:1:0 '= \"other\"",
                        "MUTATE(USER_TEXT:0:0, DELETE_SELECTED())",
                        "@MUTATE(VALUES, DUMMY)\n#FUNCTIONS\n#DIMS REF VALUES\n#DIM DUMMY\nVALUES:0 '= VALUES:0 + \"!\"\nRETURNF VALUES:0",
                        "RESULTS:12 '= USER_TEXT:0:0",
                        VmValue::String("kept!".into()),
                    )
                } else {
                    (
                        "#DIM CHARADATA USER_GRID, 2, 2\n",
                        "USER_GRID:0:1:1 = 17\nUSER_GRID:1:1:1 = 99",
                        "MUTATE(USER_GRID:0:0:0, DELETE_SELECTED())",
                        "@MUTATE(VALUES, DUMMY)\n#FUNCTION\n#DIM REF VALUES, 0, 0\n#DIM DUMMY\nVALUES:1:1 += 3\nRETURNF VALUES:1:1",
                        "RESULT:12 = USER_GRID:0:1:1",
                        VmValue::Integer(20),
                    )
                };
                let call = if dynamic {
                    let form = if string {
                        format!("%{expression}%")
                    } else {
                        format!("{{{expression}}}")
                    };
                    format!(
                        "RESULTS:11 '= STRFORM({})",
                        serde_json::to_string(&form).unwrap()
                    )
                } else if string {
                    format!("RESULTS:11 '= {expression}")
                } else {
                    format!("RESULT:11 = {expression}")
                };
                let source = format!(
                    r"@SYSTEM_TITLE
DELALLCHARA
ADDVOIDCHARA
ADDVOIDCHARA
{initialize}
{call}
{after}
RETURN
{method}
@DELETE_SELECTED
#FUNCTION
FLAG:8 += 1
DELCHARA 0
RETURNF 0
"
                );
                let mut options = method_options(snake);
                options.system_save_in_binary = true;
                let artifact = compile_with_header(header, &source, &options);
                let (vm, report) = run_entry(&artifact, VmConfig::default());
                assert!(
                    report
                        .events
                        .iter()
                        .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
                    "{snake}/{dynamic}/{string}: {report:?}"
                );
                assert!(
                    !report
                        .events
                        .iter()
                        .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                    "{snake}/{dynamic}/{string}: {report:?}"
                );
                if dynamic {
                    let text = match expected {
                        VmValue::Integer(value) => value.to_string(),
                        VmValue::String(value) => value,
                        _ => unreachable!(),
                    };
                    assert_method_watch(&vm, &artifact, "RESULTS", 11, VmValue::String(text));
                } else {
                    assert_method_watch(
                        &vm,
                        &artifact,
                        if string { "RESULTS" } else { "RESULT" },
                        11,
                        expected,
                    );
                }
                assert_method_watch(
                    &vm,
                    &artifact,
                    if string { "RESULTS" } else { "RESULT" },
                    12,
                    if string {
                        VmValue::String("other".into())
                    } else {
                        VmValue::Integer(99)
                    },
                );
                assert_method_watch(&vm, &artifact, "FLAG", 8, VmValue::Integer(1));
            }
        }
    }
}

#[test]
#[allow(clippy::too_many_lines)] // The reload lifecycle must retain both nested REF frames.
fn shape_reload_keeps_forwarded_ref_backing_until_aliases_and_old_frames_return() {
    let source = r"@SYSTEM_TITLE
SHARED_VALUES:0 = 11
SHARED_VALUES:1 = 22
RESULT:10 = OUTER(SHARED_VALUES)
INPUT
RETURN
@OUTER(VALUES)
#FUNCTION
#DIM REF VALUES
RETURNF INNER(VALUES)
@INNER(VALUES)
#FUNCTION
#DIM REF VALUES
INPUT
VALUES:1 = 44
RETURNF VALUES:1
";
    let base = compile_with_header("#DIM SHARED_VALUES, 2\n", source, &method_options(true));
    let target = compile_with_header("#DIM SHARED_VALUES, 3\n", source, &method_options(true));
    let key = named_key(&base, "SHARED_VALUES");
    let entry = base
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&base), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&base);
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let first = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        !first
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{first:?}"
    );
    let Some(FiberStatus::WaitingHost(first_request)) = vm.fiber_status(fiber) else {
        panic!("INNER did not wait: {first:?}");
    };
    let before = inspect_snapshot(
        &vm.encode_unrestricted_snapshot(&natives).unwrap(),
        VmConfig::default().maximum_snapshot_bytes,
    )
    .unwrap()
    .state;
    assert_eq!(
        before["memory"]["array_leases"]["entries"]
            .as_object()
            .unwrap()
            .len(),
        2,
        "direct capture plus forwarding capture"
    );
    let patch = create_patch(&base, &target);
    vm.prepare_hot_reload(
        &patch,
        &erabasic_compiler::runtime_native_validation_context(&target, &default_host_registry()),
    )
    .unwrap();
    let reload = vm.commit_hot_reload().unwrap();
    assert_eq!(reload.retained_generations, 2);
    let old_key = reload.old_generation.0.to_string();
    let migrated = inspect_snapshot(
        &vm.encode_unrestricted_snapshot(&natives).unwrap(),
        VmConfig::default().maximum_snapshot_bytes,
    )
    .unwrap()
    .state;
    assert!(migrated["memory"]["legacy"].get(&old_key).is_some());
    let entries = migrated["memory"]["array_leases"]["entries"]
        .as_object()
        .unwrap();
    assert_eq!(entries.len(), 2);
    assert!(
        entries
            .values()
            .all(|lease| lease["location"]["Shared"]["legacy"]
                == serde_json::json!(reload.old_generation.0))
    );
    vm.write_variable(key, &[1], None, VmValue::Integer(77))
        .unwrap();
    assert_eq!(
        vm.read_variable(key, &[2], None).unwrap(),
        VmValue::Integer(0)
    );
    vm.resume_host(first_request, HostReady::empty()).unwrap();
    let resumed = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        !resumed
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{resumed:?}"
    );
    let Some(FiberStatus::WaitingHost(last_request)) = vm.fiber_status(fiber) else {
        panic!("caller did not reach final INPUT: {resumed:?}");
    };
    assert_method_watch(&vm, &target, "RESULT", 10, VmValue::Integer(44));
    assert_eq!(
        vm.read_variable(key, &[1], None).unwrap(),
        VmValue::Integer(77)
    );
    let after_aliases = inspect_snapshot(
        &vm.encode_unrestricted_snapshot(&natives).unwrap(),
        VmConfig::default().maximum_snapshot_bytes,
    )
    .unwrap()
    .state;
    assert!(
        after_aliases["memory"]["array_leases"]["entries"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    // The old caller is still executing. It legitimately pins its generation
    // even though both REF aliases and their backing leases have returned.
    assert!(after_aliases["memory"]["legacy"].get(&old_key).is_some());
    vm.resume_host(last_request, HostReady::empty()).unwrap();
    let finished = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    completed_without_fault(&finished, fiber);
    let final_state = inspect_snapshot(
        &vm.encode_unrestricted_snapshot(&natives).unwrap(),
        VmConfig::default().maximum_snapshot_bytes,
    )
    .unwrap()
    .state;
    assert!(
        final_state["memory"]["legacy"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert!(
        final_state["memory"]["array_leases"]["entries"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        vm.read_variable(key, &[1], None).unwrap(),
        VmValue::Integer(77)
    );
}
