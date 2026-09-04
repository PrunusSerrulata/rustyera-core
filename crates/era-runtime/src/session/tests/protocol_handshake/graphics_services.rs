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
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "portable.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nRESULT = GCREATE(1, 2, 2)\nRESULT = GSETCOLOR(1, 4294967295, 0, -1)\nRESULT:1 = BITMAP_CACHE_ENABLE(1)\nRESULTS:40 = %HTML_TOPLAINTEXT(\"a&nbsp;b\")%\nRESULT:41 = GCREATE(7, 2, 2)\nRESULT:42 = GSETBRUSH(7, 4294901760)\nRESULT:43 = GGETBRUSH(7)\nRESULT:44 = GSETPEN(7, 4278255360, 2)\nRESULT:45 = GGETPEN(7)\nRESULT:46 = GGETPENWIDTH(7)\nRESULT:47 = GFILLRECTANGLE(7, 0, 0, 2, 2)\nRESULT:48 = GDRAWLINE(7, 0, 0, 1, 1)\nRESULT:49 = GDISPOSE(7)\nRESULT:50 = CBGCLEAR()\nRESULT:51 = GCREATE(8, 2, 2)\nRESULT:52 = GCREATEFROMFILE(8, \"../outside.png\", 1)\nRESULT:53 = GDISPOSE(8)\nRESULT:54 = GCREATEFROMFILE(9, \"\")\nRESULT:55 = GCREATEFROMFILE(10, \"\\\\\")\nRESULT:2 = MOVETEXTBOX(10, 20, 30)\nWAIT\nRESULT:56 = GDISPOSE(9999)\nWAIT\nRETURN\n"
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
    let mut no_op_messages = Vec::new();
    for _ in 0..4 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        no_op_messages.extend(drain(&mut session));
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[56], None).unwrap(),
        0
    );
    assert!(no_op_messages.iter().all(|message| {
        match message {
            RuntimeMessage::PresentationDelta(delta) => !delta
                .operations
                .iter()
                .any(|operation| matches!(operation, PresentationOperation::SetResources { .. })),
            _ => true,
        }
    }));
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
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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

#[test]
fn pointer_service_negotiates_only_the_existing_operation_version() {
    let selected = crate::session::selected_service_capabilities(&[
        ServiceCapability {
            kind: ServiceKind::InputState,
            operation: POINTER_STATE_OPERATION.into(),
            versions: VersionRange::exact(POINTER_STATE_OPERATION_VERSION),
        },
        ServiceCapability {
            kind: ServiceKind::InputState,
            operation: "unknown_pointer".into(),
            versions: VersionRange::exact(POINTER_STATE_OPERATION_VERSION),
        },
    ]);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].operation, POINTER_STATE_OPERATION);
    assert!(
        crate::session::selected_service_capabilities(&[ServiceCapability {
            kind: ServiceKind::InputState,
            operation: POINTER_STATE_OPERATION.into(),
            versions: VersionRange::exact(ProtocolVersion::new(2, 0))
        },])
        .is_empty()
    );
}

#[test]
fn line_geometry_service_negotiates_only_the_pinned_v1_operation() {
    let selected = crate::session::selected_service_capabilities(&[
        ServiceCapability {
            kind: ServiceKind::PresentationQuery,
            operation: GET_LINE_GEOMETRY_OPERATION.into(),
            versions: VersionRange::exact(GET_LINE_GEOMETRY_OPERATION_VERSION),
        },
        ServiceCapability {
            kind: ServiceKind::PresentationQuery,
            operation: "get_line_geometry_native".into(),
            versions: VersionRange::exact(GET_LINE_GEOMETRY_OPERATION_VERSION),
        },
    ]);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].operation, GET_LINE_GEOMETRY_OPERATION);
    assert!(
        crate::session::selected_service_capabilities(&[ServiceCapability {
            kind: ServiceKind::PresentationQuery,
            operation: GET_LINE_GEOMETRY_OPERATION.into(),
            versions: VersionRange::exact(ProtocolVersion::new(2, 0)),
        }])
        .is_empty()
    );
}

#[test]
fn sql_service_negotiates_only_the_pinned_v1_operation() {
    let selected = crate::session::selected_service_capabilities(&[
        ServiceCapability {
            kind: ServiceKind::Sql,
            operation: SQL_OPERATION.into(),
            versions: VersionRange::exact(SQL_OPERATION_VERSION),
        },
        ServiceCapability {
            kind: ServiceKind::Sql,
            operation: "rustyera.sql.native".into(),
            versions: VersionRange::exact(SQL_OPERATION_VERSION),
        },
    ]);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].kind, ServiceKind::Sql);
    assert_eq!(selected[0].operation, SQL_OPERATION);
    assert_eq!(
        selected[0].versions,
        VersionRange::exact(SQL_OPERATION_VERSION)
    );
    assert!(
        crate::session::selected_service_capabilities(&[ServiceCapability {
            kind: ServiceKind::Sql,
            operation: SQL_OPERATION.into(),
            versions: VersionRange::exact(ProtocolVersion::new(2, 0)),
        }])
        .is_empty()
    );
}
