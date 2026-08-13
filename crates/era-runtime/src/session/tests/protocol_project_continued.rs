#[test]
fn configuration_update_is_validated_and_serialized_by_the_runtime() {
    let build = build_project(
        &ProjectManifest {
            project_revision: 7,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "emuera.config".into(),
                    category: FileCategory::Configuration,
                    payload: FilePayload::Utf8("フォントサイズ:18\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "_fixed.config".into(),
                    category: FileCategory::Configuration,
                    payload: FilePayload::Utf8("ウィンドウ幅:900\n".into()),
                    content_hash: None,
                },
            ],
        },
        None,
    );
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.epoch = SessionEpoch(1);
    session.project_snapshot = build.snapshot;
    let configuration = session
        .project_snapshot
        .as_ref()
        .unwrap()
        .configuration_snapshot();
    assert!(
        configuration.source_digest.as_slice().is_empty(),
        "legacy migration must retain the missing-file write precondition"
    );
    assert!(
        configuration
            .entries
            .iter()
            .any(|entry| entry.code == "WindowX" && entry.fixed)
    );
    assert!(
        configuration
            .entries
            .iter()
            .all(|entry| entry.code != "DebugWindowWidth" && entry.code != "DrawLineString")
    );

    session
        .handle_message(
            1,
            RuntimeMessage::PrepareConfigurationUpdate(PrepareConfigurationUpdate {
                project_revision: 7,
                expected_source_digest: configuration.source_digest,
                changes: vec![era_runtime_protocol::ConfigurationChange {
                    code: "FontSize".into(),
                    value: "22".into(),
                }],
            }),
        )
        .unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::ConfigurationUpdatePrepared(prepared)
            if prepared.contents.contains("font_size = 22")
                && prepared.contents.contains("width = 900")
                && prepared.restart_required
    )));

    session
        .handle_message(
            2,
            RuntimeMessage::PrepareConfigurationUpdate(PrepareConfigurationUpdate {
                project_revision: 7,
                expected_source_digest: session
                    .project_snapshot
                    .as_ref()
                    .unwrap()
                    .configuration_snapshot()
                    .source_digest,
                changes: vec![era_runtime_protocol::ConfigurationChange {
                    code: "WindowX".into(),
                    value: "1000".into(),
                }],
            }),
        )
        .unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(rejected)
            if rejected.code == CommandErrorCode::InvalidValue
    )));
}

#[test]
fn debug_channel_has_independent_sequence_and_cannot_widen_creator_policy() {
    let mut session = RuntimeSession::new(RuntimeOptions {
        debug_scope_mask: (1 << 2) | (1 << 5),
        ..RuntimeOptions::default()
    });
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "debug-test".into(),
            features: Vec::new(),
            capabilities: capabilities(),
            requested_limits: RuntimeOptions::default().limits,
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let _ = drain(&mut session);

    submit_debug(
        &mut session,
        0,
        &DebugMessage::Hello(DebugHello {
            versions: VersionRange::exact(DEBUG_PROTOCOL_VERSION),
            requested_scopes: vec![
                DebugScope::ExecutionControl,
                DebugScope::VariablesWrite,
                DebugScope::GameFieldsRead,
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let bytes = session.poll_envelope().expect("debug grant");
    let envelope = decode_envelope(&bytes, WireLimits::default()).unwrap();
    let DebugMessage::Grant(grant) = DebugMessage::from_envelope(&envelope).unwrap() else {
        panic!("expected debug grant");
    };
    assert_eq!(
        grant.scopes,
        vec![DebugScope::GameFieldsRead, DebugScope::ExecutionControl]
    );
    assert_eq!(grant.token.session_epoch, session.epoch.0);
}

#[test]
fn debugger_pause_freezes_frontend_time_until_resume() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::DebugPaused;
    session.logical_time_ns = 500;
    session.frontend_time_origin = Some((10, 500));
    session
        .handle_message(
            1,
            RuntimeMessage::AdvanceTime(AdvanceTime {
                monotonic_time_ns: 1_000,
            }),
        )
        .unwrap();
    assert_eq!(session.logical_time_ns, 500);
    session.resume_debug_time();
    assert_eq!(session.frontend_time_origin, Some((1_000, 500)));
}

#[test]
fn revoking_the_active_debugger_resumes_a_debug_paused_runtime() {
    let build = build_project(
        &ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nWAIT\nRETURN\n".into()),
                content_hash: None,
            }],
        },
        None,
    );
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::WaitingInput;
    session.epoch = SessionEpoch(1);
    session.vm = Some(RuntimeVm::new(
        build.artifact.expect("valid project"),
        VmConfig::default(),
    ));
    let grant = GrantToken {
        grant_id: SessionId { high: 7, low: 9 },
        session_epoch: session.epoch.0,
        program_generation: 0,
        issued_runtime_revision: session.revision,
    };
    session.active_debug_grant = Some(ActiveDebugGrant {
        token: grant,
        scopes: BTreeSet::from([DebugScope::ExecutionControl]),
    });

    session
        .handle_debug_message(
            1,
            DebugMessage::Request(AuthorizedDebugRequest {
                grant,
                command: DebugCommand::Pause,
            }),
        )
        .unwrap();
    assert_eq!(session.phase, RuntimePhase::DebugPaused);
    assert!(session.vm.as_ref().unwrap().stop_token().is_some());

    session
        .handle_debug_message(
            2,
            DebugMessage::Revoke(DebugRevoke {
                grant_id: grant.grant_id,
                reason: "frontend disabled debugging".into(),
            }),
        )
        .unwrap();

    assert_eq!(session.phase, RuntimePhase::WaitingInput);
    assert!(session.active_debug_grant.is_none());
    assert!(session.vm.as_ref().unwrap().stop_token().is_none());
}

#[test]
fn ready_project_reload_stages_and_commits_a_normalized_delta() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "reload-test".into(),
            features: vec![RuntimeFeature::ProjectReload],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::ReloadProject(ReloadProject {
            base_revision: 1,
            target_revision: 2,
            changes: vec![FileChange::Upsert {
                file: SubmittedFile {
                    relative_path: "./main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nPRINTL reloaded\nRETURN\n".into()),
                    content_hash: None,
                },
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Ready);
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .unwrap()
            .manifest
            .project_revision,
        2
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectLoadReport(era_runtime_protocol::ProjectLoadReport {
            project_revision: 2,
            success: true,
            ..
        })
    )));
    assert!(!messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Diagnostic(diagnostic)
            if diagnostic.code == "runtime.input_undo_invalidated"
    )));
    let replay_header = input_replay_records(&session).remove(0);
    assert_eq!(replay_header["origin"]["kind"], "hot_reload");
    assert_eq!(replay_header["origin"]["before_revision"], "1");
    assert_eq!(replay_header["origin"]["after_revision"], "2");
    assert_eq!(replay_header["step_count"], 0);

    submit(
        &mut session,
        3,
        RuntimeMessage::ReloadProject(ReloadProject {
            base_revision: 2,
            target_revision: 3,
            changes: Vec::new(),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let unchanged_replay = input_replay_records(&session).remove(0);
    assert_eq!(unchanged_replay["origin"], replay_header["origin"]);
}

#[test]
fn state_import_rejects_out_of_order_chunks_and_bad_digests() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: vec![RuntimeFeature::TraditionalSave],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);

    submit(
        &mut session,
        1,
        RuntimeMessage::StateImportBegin(StateImportBegin {
            kind: StateExportKind::TraditionalSave,
            total_bytes: 3,
            digest: ProtocolBytes::new([0; 32]),
            artifact_id: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let transfer_id = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateImportAccepted(accepted) => Some(accepted.transfer_id),
            _ => None,
        })
        .unwrap();

    submit(
        &mut session,
        2,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 1,
            data: ProtocolBytes::new([b'a']),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));

    submit(
        &mut session,
        3,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 0,
            data: ProtocolBytes::new(*b"abc"),
        }),
    );
    submit(
        &mut session,
        4,
        RuntimeMessage::StateImportCommit(StateImportCommit { transfer_id }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));
}

#[test]
fn host_staged_compiled_cache_reuses_the_owned_payload() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let payload = vec![7; 4096];

    let transfer_id = session
        .stage_compiled_project_cache(payload.clone())
        .expect("host cache staging should accept an in-limit payload");
    let staged = session
        .consume_state_import(1, transfer_id, StateExportKind::CompiledProjectCache)
        .unwrap()
        .expect("staged cache should be committed immediately");

    assert_eq!(staged, payload);
    assert!(session.inbound_transfer.is_none());
}

#[test]
fn training_reset_updates_shared_and_all_character_state_atomically() {
    let build = build_project(
        &ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@EVENTTRAIN\nRETURN\n".into()),
                content_hash: None,
            }],
        },
        None,
    );
    let artifact = build.artifact.expect("valid project");
    let source = artifact
        .artifact()
        .globals
        .iter()
        .find(|global| global.name == "SOURCE")
        .expect("SOURCE")
        .key;
    let tflag = artifact
        .artifact()
        .globals
        .iter()
        .find(|global| global.name == "TFLAG")
        .expect("TFLAG")
        .key;
    let mut vm = RuntimeVm::new(artifact, VmConfig::default());
    let dirty = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes: vec![
                VmRuntimeWrite {
                    variable: source,
                    indices: vec![0],
                    character: Some(0),
                    value: VmValue::Integer(9),
                },
                VmRuntimeWrite {
                    variable: tflag,
                    indices: vec![0],
                    character: None,
                    value: VmValue::Integer(7),
                },
            ],
            fills: Vec::new(),
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .expect("prepare dirty state");
    vm.commit_runtime_state(dirty).expect("commit dirty state");
    reset_training_state(&mut vm).expect("reset training state");
    assert_eq!(
        vm.read_runtime_state(&[
            erabasic_vm::VmRuntimeRead {
                variable: source,
                indices: vec![0],
                character: Some(0),
            },
            erabasic_vm::VmRuntimeRead {
                variable: tflag,
                indices: vec![0],
                character: None,
            },
        ]),
        Ok(vec![VmValue::Integer(0), VmValue::Integer(0)])
    );
}

#[test]
fn show_user_reset_clears_shared_and_character_deltas() {
    let build = build_project(
        &ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@EVENTTRAIN\nRETURN\n".into()),
                content_hash: None,
            }],
        },
        None,
    );
    let mut vm = RuntimeVm::new(build.artifact.expect("valid project"), VmConfig::default());
    for (name, character) in [
        ("UP", None),
        ("DOWN", None),
        ("LOSEBASE", None),
        ("DOWNBASE", Some(0)),
        ("CUP", Some(0)),
        ("CDOWN", Some(0)),
    ] {
        write_runtime_integer(&mut vm, name, &[0], character, 9).unwrap();
    }

    reset_after_show_user(&mut vm).unwrap();

    for (name, character) in [
        ("UP", None),
        ("DOWN", None),
        ("LOSEBASE", None),
        ("DOWNBASE", Some(0)),
        ("CUP", Some(0)),
        ("CDOWN", Some(0)),
    ] {
        assert_eq!(
            read_runtime_integer(&vm, name, &[0], character).unwrap(),
            0,
            "{name} was not reset"
        );
    }
}

#[test]
fn shop_purchase_validates_stock_and_commits_money_item_and_bought_together() {
    let build = build_project(
        &ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "ITEM.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8("5,potion,120\n".into()),
                    content_hash: None,
                },
            ],
        },
        None,
    );
    let artifact = build.artifact.expect("valid project");
    let mut vm = RuntimeVm::new(artifact, VmConfig::default());
    let sales = runtime_variable_key(&vm, "ITEMSALES").unwrap();
    let money = runtime_variable_key(&vm, "MONEY").unwrap();
    let dirty = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes: vec![
                VmRuntimeWrite {
                    variable: sales,
                    indices: vec![5],
                    character: None,
                    value: VmValue::Integer(1),
                },
                VmRuntimeWrite {
                    variable: money,
                    indices: Vec::new(),
                    character: None,
                    value: VmValue::Integer(200),
                },
            ],
            fills: Vec::new(),
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .unwrap();
    vm.commit_runtime_state(dirty).unwrap();
    assert_eq!(
        purchase_item(&mut vm, 5, 100).unwrap(),
        PurchaseResult::Purchased
    );
    assert_eq!(read_runtime_integer(&vm, "MONEY", &[], None).unwrap(), 80);
    assert_eq!(read_runtime_integer(&vm, "ITEM", &[5], None).unwrap(), 1);
    assert_eq!(read_runtime_integer(&vm, "BOUGHT", &[], None).unwrap(), 5);
    assert_eq!(
        purchase_item(&mut vm, 5, 100).unwrap(),
        PurchaseResult::NotEnoughMoney
    );
    assert_eq!(read_runtime_integer(&vm, "MONEY", &[], None).unwrap(), 80);
    assert_eq!(read_runtime_integer(&vm, "ITEM", &[5], None).unwrap(), 1);
}

#[test]
#[allow(clippy::too_many_lines)]
fn train_controller_consumes_runtime_button_intent_and_loops_after_eventcomend() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    // RESETDATA removes every character in the reference runtime, so a standalone
    // SYSTEM_TITLE fixture must explicitly create the character used by training.
    let source = "@SYSTEM_TITLE\nRESETDATA\nADDVOIDCHARA\nBEGIN TRAIN\n@EVENTTRAIN\nRETURN\n@SHOW_STATUS\nPRINT 抑鬱\nRETURN\n@COM_ABLE0\nRETURN 1\n@SHOW_USERCOM\nPRINT ▼[－][Look]----------\nRETURN\n@EVENTCOM\nRETURN\n@COM0\nFLAG:0 += 1\nRETURN 1\n@SOURCE_CHECK\nRETURN\n@EVENTCOMEND\nRETURN\n";
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(source.into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "TRAIN.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8("0,go\n".into()),
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
            mode: StartMode::NewGame { seed: Some(3) },
        }),
    );
    for _ in 0..12 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let pending = session
        .operations
        .active_input()
        .expect("training command wait");
    let snapshot = session.presentation.snapshot();
    let flattened_lines = snapshot
        .history
        .logical_lines
        .iter()
        .map(|line| {
            fn text(runs: &[DisplayRun], output: &mut String) {
                for run in runs {
                    match run {
                        DisplayRun::Text { text, .. } | DisplayRun::TextLayout { text, .. } => {
                            output.push_str(text);
                        }
                        DisplayRun::Button { runs, .. }
                        | DisplayRun::ColumnCell { content: runs, .. } => text(runs, output),
                        _ => {}
                    }
                }
            }
            let mut output = String::new();
            text(&line.runs, &mut output);
            output
        })
        .collect::<Vec<_>>();
    let status_line = flattened_lines
        .iter()
        .position(|line| line.contains("抑鬱"))
        .expect("SHOW_STATUS output");
    let look_line = flattened_lines
        .iter()
        .position(|line| line.contains("[Look]"))
        .expect("SHOW_USERCOM output");
    assert!(status_line < look_line, "{flattened_lines:#?}");
    let token = *pending.choices.keys().next().expect("PRINTBUTTON token");
    let wait_id = pending.wait.wait_id;
    let submission_token = pending.wait.submission_token;
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token: submission_token,
            intent: InputIntent::Activate(token),
            monotonic_time_ns: 0,
            message_skip: false,
        }),
    );
    for _ in 0..64 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.controller.step == SystemStep::TrainEventComEndWait {
            break;
        }
    }
    let (event_end_wait_id, event_end_token) = session
        .operations
        .active_input()
        .map(|pending| (pending.wait.wait_id, pending.wait.submission_token))
        .expect("EVENTCOMEND wait");
    submit(
        &mut session,
        4,
        RuntimeMessage::Input(FrontendInput {
            wait_id: event_end_wait_id,
            token: event_end_token,
            intent: InputIntent::Continue,
            monotonic_time_ns: 0,
            message_skip: false,
        }),
    );
    for _ in 0..64 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some()
            && session.controller.step == SystemStep::TrainShowUser
        {
            break;
        }
    }
    let output = drain(&mut session);
    assert_ne!(session.phase, RuntimePhase::Faulted, "{output:#?}");
    let vm = session.vm.as_ref().expect("running VM");
    assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 1);
    assert_eq!(
        read_runtime_integer(vm, "SOURCE", &[0], Some(0)).unwrap(),
        0
    );
    assert_eq!(
        session.controller.step,
        SystemStep::TrainShowUser,
        "phase={:?}",
        session.phase
    );
}
