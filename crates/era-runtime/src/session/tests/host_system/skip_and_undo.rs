#[test]
fn skipdisp_silently_skips_wait_commands_like_the_reference() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "skipdisp-wait-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "skipdisp.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nSKIPDISP 1\nWAIT\nWAITANYKEY\nFORCEWAIT\nTWAIT 1, 0\nSKIPDISP 0\nPRINTL visible\nWAIT\nRETURN\n"
                        .into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    let loaded = drain(&mut session);
    assert!(
        loaded.iter().any(
            |message| matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
        ),
        "project load failed: {loaded:?}"
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).expect("run");
        if session.operations.active_input().is_some() || session.phase == RuntimePhase::Faulted {
            break;
        }
    }
    assert_ne!(session.phase, RuntimePhase::Faulted);
    assert_eq!(
        session
            .operations
            .active_input()
            .expect("only the final WAIT should open")
            .wait
            .kind,
        WaitKind::EnterKey
    );
    let visible = session
        .presentation
        .snapshot()
        .history
        .logical_lines
        .iter()
        .flat_map(|line| &line.runs)
        .filter_map(|run| match run {
            DisplayRun::Text { text, .. } | DisplayRun::TextLayout { text, .. } => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    assert!(visible.contains("visible"), "{visible}");
}

#[test]
#[allow(clippy::too_many_lines)]
fn input_undo_records_only_accepted_scalar_input_after_a_checkpoint() {
    for profile in [
        erabasic_compat::CompatibilityProfileId::EmueraEm,
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "undo-test".into(),
                features: vec![RuntimeFeature::InputUndo],
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["en".into()],
                configuration_profile: None,
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        let identity = erabasic_compat::CompatibilityIdentity::for_profile(profile);
        submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: identity.clone(),
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "reraconfig.toml".into(),
                    category: FileCategory::Configuration,
                    payload: FilePayload::Utf8(format!(
                            "[meta]\nschema_version = 4\n[compatibility]\nprofile = \"{}\"\n[save]\nbinary_format = true\n[input]\nundo_enabled = true\n",
                        identity.profile.as_str(),
                    )),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "input.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        concat!(
                            "@SYSTEM_TITLE\nDUMPRAND\nINPUT\nFLAG:20 = RAND:1000000000\nDUMPRAND\nWAIT\nRETURN\n",
                            "@SHOW_SHOP\nWAIT\nFLAG:20 = RAND:1000000000\nDUMPRAND\nWAIT\nRETURN\n",
                        ).into(),
                    ),
                    content_hash: None,
                },
            ],
        }),
    );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        submit(
            &mut session,
            2,
            RuntimeMessage::Start(StartRequest {
                mode: StartMode::NewGame { seed: Some(7) },
            }),
        );
        for _ in 0..8 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.operations.active_input().is_some() {
                break;
            }
        }
        let random = session.vm.as_ref().unwrap().export_random_state().unwrap();
        let baseline = {
            let vm = session.vm.as_ref().unwrap();
            encode_scoped_save(
                &vm.export_era_state(),
                vm.vm().artifact(),
                era_runtime_save::SaveFileKind::Normal,
                "checkpoint".into(),
                Vec::new(),
                session.traditional_save_format(),
            )
            .unwrap()
        };
        session
            .establish_input_undo_checkpoint(3, baseline, random.clone())
            .unwrap();
        let (wait_id, token) = session
            .operations
            .active_input()
            .map(|pending| (pending.wait.wait_id, pending.wait.submission_token))
            .unwrap();
        submit(
            &mut session,
            3,
            RuntimeMessage::Input(FrontendInput {
                wait_id,
                token,
                intent: InputIntent::CommitText("42".into()),
                monotonic_time_ns: 0,
                message_skip: false,
            }),
        );
        for _ in 0..8 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.operations.active_input().is_some() {
                break;
            }
        }
        assert_eq!(
            session
                .undo_checkpoint
                .as_ref()
                .unwrap()
                .inputs
                .iter()
                .map(|input| input.value.as_str())
                .collect::<Vec<_>>(),
            vec!["42"]
        );
        let vm = session.vm.as_ref().unwrap();
        let sample = read_runtime_integer(vm, "FLAG", &[20], None).unwrap();
        let advanced_random = vm.export_random_state().unwrap();
        assert_eq!(advanced_random.len(), 625);
        assert_ne!(
            advanced_random, random,
            "accepted input must advance the actual RAND stream"
        );
        let state = session.input_undo_state();
        assert!(state.enabled);
        assert_eq!(state.available_steps, 1);
        let undo_token = state.token.expect("undo token");
        submit(
            &mut session,
            4,
            RuntimeMessage::InputUndoRequest(InputUndoRequest { token: undo_token }),
        );
        for _ in 0..32 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.undo_replay.is_none() && session.operations.active_input().is_some() {
                break;
            }
        }
        assert_ne!(session.phase, RuntimePhase::Faulted);
        let vm = session.vm.as_ref().unwrap();
        assert_eq!(
            vm.export_random_state().unwrap(),
            random,
            "Ctrl-Z restores the separate native stream checkpoint"
        );
        assert_eq!(
            (0..625)
                .map(|index| read_runtime_integer(vm, "RANDDATA", &[index], None).unwrap())
                .collect::<Vec<_>>(),
            random,
            "the ordinary checkpoint also restores the saved RANDDATA variable",
        );
        assert!(session.undo_checkpoint.as_ref().unwrap().inputs.is_empty());
        assert_eq!(session.input_undo_state().available_steps, 0);
        let records = input_replay_records(&session);
        assert_eq!(records[0]["origin"]["kind"], "input_undo");
        assert_eq!(records[0]["origin"]["retained_input_count"], 0);
        assert_eq!(records[0]["step_count"], 0);
        assert_eq!(
            records.len(),
            1,
            "automatic Ctrl-Z replay must not write steps"
        );
        let wait = session
            .operations
            .active_input()
            .expect("post-undo wait")
            .wait
            .clone();
        submit(
            &mut session,
            5,
            RuntimeMessage::Input(FrontendInput {
                wait_id: wait.wait_id,
                token: wait.submission_token,
                monotonic_time_ns: 1,
                intent: InputIntent::Enter,
                message_skip: false,
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        assert!(
            session.undo_checkpoint.as_ref().unwrap().inputs.is_empty(),
            "non-value WAIT completions are not part of reference Ctrl-Z input history"
        );
        let records = input_replay_records(&session);
        assert_eq!(records[0]["step_count"], 1);
        assert_eq!(records[1]["action"], "enter");
        let vm = session.vm.as_ref().unwrap();
        assert_eq!(
            read_runtime_integer(vm, "FLAG", &[20], None).unwrap(),
            sample
        );
        assert_eq!(vm.export_random_state().unwrap(), advanced_random);
        assert_eq!(
            (0..625)
                .map(|index| read_runtime_integer(vm, "RANDDATA", &[index], None).unwrap())
                .collect::<Vec<_>>(),
            advanced_random,
            "post-undo execution must reproduce the full stream position",
        );
    }
}

#[test]
fn input_undo_keeps_the_next_scalar_queued_across_primitive_waits() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.undo_replay = Some(UndoReplay {
        remaining: VecDeque::from([RecordedInput {
            value: "12".to_owned(),
            source: None,
        }]),
        queued_repeats: 0,
    });
    let mut wait = session.system_wait(InteractionToken { epoch: 0, id: 1 });
    wait.kind = WaitKind::PrimitiveMouseKey;
    let mut pending = PendingInput {
        host_request: None,
        wait,
        result_name: None,
        choices: BTreeMap::new(),
        timeout_duration_ns: None,
        post_input: None,
    };
    assert_eq!(session.replay_submission(&pending).unwrap(), None);
    assert_eq!(
        session.undo_replay.as_ref().unwrap().remaining,
        VecDeque::from([RecordedInput {
            value: "12".to_owned(),
            source: None
        }])
    );

    pending.wait.kind = WaitKind::IntegerValue;
    assert_eq!(
        session.replay_submission(&pending).unwrap(),
        Some(InputSubmission::Value(VmValue::Integer(12)))
    );
    assert_eq!(session.undo_replay.as_ref().unwrap().remaining.len(), 1);
    session
        .verify_replayed_input(&VmValue::Integer(12))
        .unwrap();
    assert!(session.undo_replay.as_ref().unwrap().remaining.is_empty());
}
