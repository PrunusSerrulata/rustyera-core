use super::*;

#[test]
fn projection_observation_updates_draw_line_string_width() {
    let build = build_project(
        &ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                content_hash: None,
            }],
        },
        None,
    );
    let artifact = build.artifact.expect("valid project");
    let draw_line = artifact
        .artifact()
        .globals
        .iter()
        .find(|global| global.name == "DRAWLINESTR")
        .expect("DRAWLINESTR")
        .key;
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.vm = Some(RuntimeVm::new(artifact, VmConfig::default()));

    session
        .observe_projection(
            1,
            ProjectionObservation {
                environment_revision: 1,
                presentation_revision: session.presentation.revision(),
                client_size: ProjectionSize {
                    width: ProjectionLength(1_395),
                    height: ProjectionLength(768),
                },
                projection_space_revision: 1,
                line_columns: 198,
                text_box: String::new(),
                transform: ProjectionTransform {
                    x_numerator: 1,
                    x_denominator: 1,
                    y_numerator: 1,
                    y_denominator: 1,
                    origin_x: ProjectionLength(0),
                    origin_y: ProjectionLength(0),
                },
            },
        )
        .unwrap();

    assert_eq!(session.line_columns, 198);
    assert_eq!(
        session
            .vm
            .as_ref()
            .unwrap()
            .vm()
            .read_variable(draw_line, &[], None),
        Ok(VmValue::String("-".repeat(198)))
    );
}

#[test]
fn presentation_updates_are_coalesced_until_the_drive_boundary() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());

    for fragment in ["first", "second", "third"] {
        session
            .presentation
            .append_print_text(fragment.into(), false, false);
        session.emit_presentation().unwrap();
    }

    assert!(
        session.outbound.is_empty(),
        "intermediate current-line projections must not be serialized"
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    let updates = messages
        .iter()
        .filter(|message| {
            matches!(
                message,
                RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
            )
        })
        .count();

    assert_eq!(updates, 1);
    let snapshot = session.presentation.snapshot();
    let text = snapshot
        .history
        .logical_lines
        .iter()
        .flat_map(|line| &line.runs)
        .filter_map(|run| match run {
            DisplayRun::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(text, "firstsecondthird");
}

#[test]
fn handshake_selects_only_implemented_features() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: vec![RuntimeFeature::Audio, RuntimeFeature::TimedInput],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("drive");
    let messages = drain(&mut session);
    let RuntimeMessage::ServerHello(hello) = &messages[0] else {
        panic!("expected server hello");
    };
    assert_eq!(hello.selected_version, RUNTIME_PROTOCOL_VERSION);
    assert!(hello.features.contains(&RuntimeFeature::TimedInput));
    assert!(!hello.features.contains(&RuntimeFeature::Audio));
    assert_eq!(hello.selected_capabilities.storage, capabilities().storage);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Log(RuntimeLog {
            level: RuntimeLogLevel::Debug,
            message,
        }) if message.contains("handshake complete")
    )));
}

#[test]
fn key_macro_edits_emit_canonical_state_and_persist_through_frontend_storage() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.epoch = SessionEpoch(1);
    session.negotiated_features.insert(RuntimeFeature::Storage);
    session
        .negotiated_features
        .insert(RuntimeFeature::KeyMacros);
    session.storage_capabilities = capabilities().storage;
    session
        .apply_key_macro_command(
            7,
            KeyMacroCommand::Store {
                group: 1,
                slot: 2,
                text: "abc".into(),
            },
        )
        .unwrap();
    let messages = drain(&mut session);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::KeyMacroStateChanged(state)
            if state.entries[era_runtime_protocol::KEY_MACRO_SLOTS + 2] == "abc"
                && state.serialized.contains("G1:マクロキーF3:abc")
    )));
    let request_id = messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) if request.relative_path == "macro.txt" => {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("macro persistence request");
    assert_eq!(session.phase, RuntimePhase::WaitingExternal);
    session
        .complete_storage(
            8,
            StorageResponse {
                request_id,
                result: StorageResult::Written {
                    revision: Some("1".into()),
                },
            },
        )
        .unwrap();
    assert_eq!(session.phase, RuntimePhase::Ready);
}

#[test]
fn key_macro_activation_recalls_runtime_text_without_completing_the_wait() {
    let token = InteractionToken { epoch: 1, id: 4 };
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::WaitingInput;
    session.epoch = SessionEpoch(1);
    session
        .negotiated_features
        .insert(RuntimeFeature::KeyMacros);
    assert!(session.key_macros.store(2, 3, "(ab)*2\\nnext".into()));
    session.operations.activate_input(PendingInput {
        host_request: None,
        wait: InputWait {
            wait_id: 9,
            kind: WaitKind::StringValue,
            stability: WaitStability::StableInput,
            one_input: false,
            stop_message_skip: false,
            system_input: true,
            mouse_input: false,
            default_value: None,
            deadline_ns: None,
            display_time: false,
            timeout_message: None,
            submission_token: token,
            countdown_remaining_ms: None,
        },
        result_name: Some("RESULTS".into()),
        choices: BTreeMap::new(),
        timeout_duration_ns: None,
        post_input: None,
    });
    session
        .complete_input(
            7,
            FrontendInput {
                wait_id: 9,
                token,
                monotonic_time_ns: 0,
                intent: InputIntent::ActivateKeyMacro { group: 2, slot: 3 },
                message_skip: false,
            },
        )
        .unwrap();
    assert_eq!(session.text_box, "(ab)*2\\nnext");
    assert!(session.operations.active_input().is_some());
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectionState(state) if state.text_box == "(ab)*2\\nnext"
    )));
}

#[test]
fn project_analysis_is_one_shot_and_does_not_replace_loaded_state() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Negotiating;
    session.epoch = SessionEpoch(1);
    session
        .negotiated_features
        .insert(RuntimeFeature::ProjectAnalysis);
    session
        .analyze_project(
            3,
            &era_runtime_protocol::ProjectAnalysisRequest {
                manifest: ProjectManifest {
                    project_revision: 4,
                    files: vec![SubmittedFile {
                        relative_path: "unused.erb".into(),
                        category: FileCategory::Erb,
                        payload: FilePayload::Utf8("@UNUSED\nRETURN\n".into()),
                        content_hash: None,
                    }],
                },
                selected_erb_paths: Vec::new(),
                debug_mode: false,
            },
        )
        .unwrap();
    assert!(session.project_snapshot.is_none());
    assert_eq!(session.phase, RuntimePhase::Negotiating);
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectAnalysisReport(report) if report.success
    )));
}

#[test]
#[allow(clippy::too_many_lines)]
fn portable_extension_service_validates_return_and_mutable_writes() {
    let operation_version = ProtocolVersion::new(1, 0);
    let mut client_capabilities = capabilities();
    client_capabilities.services.push(ServiceCapability {
        kind: ServiceKind::Extension,
        operation: "example.mutate".into(),
        versions: VersionRange::exact(operation_version),
    });
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "extension-test".into(),
            features: vec![RuntimeFeature::ExternalServices],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client_capabilities,
            preferred_locales: vec!["en".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ExtensionRegistrySubmit(ExtensionRegistrySubmit {
            declarations: vec![era_runtime_protocol::ExtensionDeclaration {
                id: "example.mutate.v1".into(),
                era_name: "EXT_MUTATE".into(),
                kind: era_runtime_protocol::ExtensionCallableKind::Function,
                arguments: vec![era_runtime_protocol::ExtensionArgument {
                    value_type: era_runtime_protocol::ExtensionValueType::Integer,
                    mutable: true,
                    optional: false,
                }],
                variadic: false,
                return_type: era_runtime_protocol::ExtensionValueType::Integer,
                argument_style: era_runtime_protocol::ExtensionArgumentStyle::Normal,
                operation: "example.mutate".into(),
                operation_version,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "extension.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nRESULT = EXT_MUTATE(FLAG:0)\nWAIT\nRETURN\n".into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let load_messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Ready, "{load_messages:?}");
    submit(
        &mut session,
        3,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let mut request = None;
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        request = drain(&mut session)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::ServiceRequest(request)
                    if request.kind == ServiceKind::Extension =>
                {
                    Some(request)
                }
                _ => None,
            });
        if request.is_some() {
            break;
        }
    }
    let request = request.expect("extension service request");
    let invocation: era_runtime_protocol::ExtensionInvocation =
        decode_canonical(request.payload.as_slice()).unwrap();
    assert_eq!(invocation.extension_id, "example.mutate.v1");
    assert_eq!(
        invocation.arguments,
        vec![era_runtime_protocol::ProtocolValue::Integer(0)]
    );
    submit(
        &mut session,
        4,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&era_runtime_protocol::ExtensionResult {
                        value: Some(era_runtime_protocol::ProtocolValue::Integer(7)),
                        writes: vec![era_runtime_protocol::ExtensionWrite {
                            argument_ordinal: 0,
                            value: era_runtime_protocol::ProtocolValue::Integer(5),
                        }],
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    for _ in 0..4 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(read_runtime_integer(vm, "RESULT", &[], None).unwrap(), 7);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 5);
}

#[test]
fn html_layout_query_is_revision_bound_and_commits_after_service_response() {
    let mut client_capabilities = capabilities();
    client_capabilities.html = true;
    client_capabilities.services.push(ServiceCapability {
        kind: ServiceKind::PresentationQuery,
        operation: HTML_STRING_LEN_OPERATION.into(),
        versions: VersionRange::exact(HTML_STRING_LEN_OPERATION_VERSION),
    });
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "projection-test".into(),
            features: vec![RuntimeFeature::Html, RuntimeFeature::ExternalServices],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client_capabilities,
            preferred_locales: vec!["en".into()],
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
                relative_path: "projection.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nRESULT = HTML_STRINGLEN(\"<b>x</b>\", 1)\nWAIT\nRETURN\n"
                        .into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let load_messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Ready, "{load_messages:#?}");
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let mut observed = Vec::new();
    let request = (0..8)
        .find_map(|_| {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            let messages = drain(&mut session);
            observed.extend(messages.clone());
            messages.into_iter().find_map(|message| match message {
                RuntimeMessage::ServiceRequest(request)
                    if request.operation == HTML_STRING_LEN_OPERATION =>
                {
                    Some(request)
                }
                _ => None,
            })
        })
        .unwrap_or_else(|| {
            panic!(
                "HTML layout service request; phase={:?} {observed:#?}",
                session.phase()
            )
        });
    let payload: HtmlMeasureRequest = decode_canonical(request.payload.as_slice()).unwrap();
    assert_eq!(payload.markup, "<b>x</b>");
    assert_eq!(payload.argument, 1);
    assert!(!session.operations.is_snapshot_stable());
    submit(
        &mut session,
        3,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&ProjectionIntegerResponse {
                        context: payload.context,
                        value: 12,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    for _ in 0..4 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        12
    );
}

#[test]
fn ggetcolor_rejects_negative_y_without_frontend_raster_observation() {
    let mut client_capabilities = capabilities();
    client_capabilities.graphics = true;
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "canvas-bounds-test".into(),
            features: vec![RuntimeFeature::Graphics],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client_capabilities,
            preferred_locales: vec!["en".into()],
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
                relative_path: "canvas.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nRESULT = GCREATE(1, 2, 2)\nRESULT = GGETCOLOR(1, 0, -1)\nWAIT\nRETURN\n"
                        .into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let load_messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Ready, "{load_messages:#?}");
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let mut messages = Vec::new();
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        messages.extend(drain(&mut session));
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
    assert!(!messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::ServiceRequest(request)
            if request.operation == SAMPLE_CANVAS_PIXEL_OPERATION
    )));
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        -1
    );
}

#[test]
fn gsave_without_canvas_encoder_returns_failure_and_continues() {
    let mut client_capabilities = capabilities();
    client_capabilities.graphics = true;
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "canvas-save-fallback-test".into(),
            features: vec![RuntimeFeature::Graphics],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client_capabilities,
            preferred_locales: vec!["en".into()],
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
                relative_path: "canvas-save.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nRESULT = GCREATE(1, 2, 2)\nRESULT = GSAVE(1, 0)\nWAIT\nRETURN\n"
                        .into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let mut messages = Vec::new();
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        messages.extend(drain(&mut session));
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }

    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
    assert!(!messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::ServiceRequest(request)
            if request.operation == ENCODE_CANVAS_PNG_OPERATION
    )));
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        0
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn portable_graphics_and_textbox_compatibility_paths_are_runtime_owned() {
    let mut client_capabilities = capabilities();
    client_capabilities.graphics = true;
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "portable-presentation-test".into(),
            features: vec![RuntimeFeature::Graphics],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client_capabilities,
            preferred_locales: vec!["en".into()],
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
                relative_path: "portable.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nRESULT = GCREATE(1, 2, 2)\nRESULT = GSETCOLOR(1, 4294967295, 0, -1)\nRESULT:1 = BITMAP_CACHE_ENABLE(1)\nRESULTS:40 = %HTML_TOPLAINTEXT(\"a&nbsp;b\")%\nRESULT:41 = GCREATE(7, 2, 2)\nRESULT:42 = GSETBRUSH(7, 4294901760)\nRESULT:43 = GGETBRUSH(7)\nRESULT:44 = GSETPEN(7, 4278255360, 2)\nRESULT:45 = GGETPEN(7)\nRESULT:46 = GGETPENWIDTH(7)\nRESULT:47 = GFILLRECTANGLE(7, 0, 0, 2, 2)\nRESULT:48 = GDRAWLINE(7, 0, 0, 1, 1)\nRESULT:49 = GDISPOSE(7)\nRESULT:50 = CBGCLEAR()\nRESULT:51 = GCREATE(8, 2, 2)\nRESULT:52 = GCREATEFROMFILE(8, \"../outside.png\", 1)\nRESULT:53 = GDISPOSE(8)\nRESULT:54 = GCREATEFROMFILE(9, \"\")\nRESULT:55 = GCREATEFROMFILE(10, \"\\\\\")\nRESULT:2 = MOVETEXTBOX(10, 20, 30)\nWAIT\nRETURN\n"
                        .into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let mut messages = Vec::new();
    for _ in 0..32 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        messages.extend(drain(&mut session));
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        0
    );
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[1], None).unwrap(),
        0
    );
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[2], None).unwrap(),
        1
    );
    let vm = session.vm.as_ref().unwrap();
    let results = runtime_variable_key(vm, "RESULTS").unwrap();
    assert_eq!(
        vm.vm().read_variable(results, &[40], None),
        Ok(VmValue::String("a b".into()))
    );
    let expected_graphics = [
        (41, 1),
        (42, 1),
        (43, 4_294_901_760),
        (44, 1),
        (45, 4_278_255_360),
        (46, 2),
        (47, 1),
        (48, 1),
        (49, 1),
        (50, 1),
        (51, 1),
        (52, 0),
        (53, 1),
        (54, 0),
        (55, 0),
    ];
    for (index, expected) in expected_graphics {
        assert_eq!(
            read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[index], None).unwrap(),
            expected,
            "section 3 oracle differs at RESULT:{index}"
        );
    }
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectionState(state)
            if state.text_box_layout == TextBoxLayout { x: 10, y: 20, width: 30 }
    )));

    let pending = session.operations.active_input().unwrap();
    let wait_id = pending.wait.wait_id;
    let token = pending.wait.submission_token;
    session
        .complete_input(
            0,
            FrontendInput {
                wait_id,
                token,
                monotonic_time_ns: 1,
                intent: InputIntent::Continue,
                message_skip: false,
            },
        )
        .unwrap();
    assert_eq!(session.text_box_layout, TextBoxLayout::default());
}

#[test]
fn invalid_host_file_paths_return_reference_failure_values() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "invalid-host-file-path-test".into(),
            features: vec![RuntimeFeature::Storage],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
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
                relative_path: "invalid-path.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nRESULT:10 = SAVETEXT(\"x\", \"\")\nRESULTS:10 = %LOADTEXT(\"\")%\nRESULT:11 = EXISTFILE(\"\")\nRESULT:12 = ENUMFILES(\"../outside\")\nWAIT\nRETURN\n"
                        .into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let load_messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Ready, "{load_messages:#?}");
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let mut messages = Vec::new();
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        messages.extend(drain(&mut session));
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }

    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::StorageRequest(_)))
    );
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(read_runtime_integer(vm, "RESULT", &[10], None).unwrap(), 0);
    assert_eq!(read_runtime_integer(vm, "RESULT", &[11], None).unwrap(), 0);
    assert_eq!(read_runtime_integer(vm, "RESULT", &[12], None).unwrap(), -1);
    let results = runtime_variable_key(vm, "RESULTS").unwrap();
    assert_eq!(
        vm.vm().read_variable(results, &[10], None),
        Ok(VmValue::String(String::new()))
    );
}
