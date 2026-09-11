fn preset_erd_runtime_manifest(
    profile: erabasic_compat::CompatibilityProfileId,
) -> ProjectManifest {
    let mut files = vec![profile_configuration_file(profile)];
    // These rows are the warning-free subset of the pinned C fixture. GETNUM
    // receives a runtime string so its result cannot be only a compiler fold.
    for (path, category, source) in [
        (
            "erb/main.erb",
            FileCategory::Erb,
            "@SYSTEM_TITLE\nRESULTS:20 '= \"added\"\nRESULT:10 = GETNUM(ABL, RESULTS:20)\nRESULT:11 = ITEMPRICE:0\nRESULT:12 = ITEMPRICE:1\nRESULT:13 = ITEMPRICE:2\nRESULTS:10 '= ABLNAME:1\nRESULTS:11 '= ITEMNAME:2\nWAIT\nRETURN\n",
        ),
        ("csv/ABL.CSV", FileCategory::Csv, "0,csv_name\n"),
        ("csv/ITEM.CSV", FileCategory::Csv, "0,zero,0\n1,missing\n"),
        ("erb/ABL.erd", FileCategory::Erd, "1,added\n"),
        (
            "erb/ITEMPRICE.erd",
            FileCategory::Erd,
            "0,zero,0\n1,missing,5\n2,new_item,9\n",
        ),
    ] {
        files.push(SubmittedFile {
            relative_path: path.into(),
            category,
            payload: FilePayload::Utf8(source.into()),
            content_hash: None,
        });
    }
    ProjectManifest {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(profile),
        project_revision: 1,
        files,
    }
}

fn assert_preset_erd_runtime_values(session: &mut RuntimeSession, snake: bool) {
    submit(
        session,
        1,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame {
                seed: Some(123_456),
            },
        }),
    );
    for _ in 0..24 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(session);
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    let vm = session.vm.as_ref().unwrap();
    let integers = runtime_variable_key(vm, "RESULT").unwrap();
    let strings = runtime_variable_key(vm, "RESULTS").unwrap();
    let reads = [
        (integers, 10),
        (integers, 11),
        (integers, 12),
        (integers, 13),
        (strings, 10),
        (strings, 11),
    ]
    .into_iter()
    .map(|(variable, index)| erabasic_vm::VmRuntimeRead {
        variable,
        indices: vec![index],
        character: None,
    })
    .collect::<Vec<_>>();
    let expected = if snake {
        vec![
            VmValue::Integer(1),
            VmValue::Integer(0),
            VmValue::Integer(5),
            VmValue::Integer(9),
            VmValue::String("added".into()),
            VmValue::String("new_item".into()),
        ]
    } else {
        vec![
            VmValue::Integer(-1),
            VmValue::Integer(0),
            VmValue::Integer(0),
            VmValue::Integer(0),
            VmValue::String(String::new()),
            VmValue::String(String::new()),
        ]
    };
    assert_eq!(vm.read_runtime_state(&reads).unwrap(), expected);
}

fn export_preset_erd_runtime_cache(session: &mut RuntimeSession) -> Vec<u8> {
    session
        .export_state(
            90,
            StateExportRequest {
                kind: StateExportKind::CompiledProjectCache,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.compiled_cache_task.is_some() {
        session.poll_compiled_cache_task().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "compiled cache export stalled"
        );
        std::thread::yield_now();
    }
    drain(session);
    session
        .compiled_project_cache
        .as_ref()
        .unwrap()
        .as_ref()
        .clone()
}

#[test]
fn preset_erd_runtime_values_survive_compiled_cache_round_trip_for_both_profiles() {
    use erabasic_compat::CompatibilityProfileId;
    for profile in [
        CompatibilityProfileId::EmueraEm,
        CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let manifest = preset_erd_runtime_manifest(profile);
        let identity = crate::compiled_cache::project_identity(&manifest);
        let mut cold = negotiated_session();
        cold.load_project(
            80,
            ProjectLoadRequest {
                identity: identity.clone(),
                manifest: Some(manifest),
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
        let loaded = drain(&mut cold);
        assert!(loaded.iter().any(|message| matches!(message,
            RuntimeMessage::ProjectLoadReport(report) if report.success && !report.payload_required
        )), "{loaded:#?}");
        // Configuration migration publishes a normalized project. Export and reload
        // bind that persisted identity, as in the existing warm-cache tests.
        let identity = crate::compiled_cache::project_identity(
            &cold.project_snapshot.as_ref().unwrap().manifest,
        );
        let cache = export_preset_erd_runtime_cache(&mut cold);
        let snake = profile == CompatibilityProfileId::EmueraSkiaSnake;
        assert_preset_erd_runtime_values(&mut cold, snake);

        let mut warm = negotiated_session();
        let transfer = warm.stage_compiled_project_cache(cache).unwrap();
        warm.load_project(
            81,
            ProjectLoadRequest {
                identity,
                manifest: None,
                compiled_cache_transfer_id: Some(transfer),
            },
        )
        .unwrap();
        let loaded = drain(&mut warm);
        assert!(
            loaded.iter().any(|message| matches!(message,
                RuntimeMessage::ProjectLoadReport(report) if report.success
                    && !report.payload_required && report.diagnostics.iter().any(|diagnostic|
                        diagnostic.code == "runtime.compiled_cache_hit")
            )),
            "{loaded:#?}"
        );
        assert_preset_erd_runtime_values(&mut warm, snake);
    }
}
