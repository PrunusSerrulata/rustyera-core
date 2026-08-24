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
            configuration_profile: None,
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

fn start_html_query(
    source: &str,
    operation: &str,
    operation_version: ProtocolVersion,
) -> (RuntimeSession, ServiceRequest) {
    let (session, request, _) = start_html_query_with_messages(source, operation, operation_version);
    (session, request)
}

fn start_html_query_with_messages(
    source: &str,
    operation: &str,
    operation_version: ProtocolVersion,
) -> (RuntimeSession, ServiceRequest, Vec<RuntimeMessage>) {
    let mut client_capabilities = capabilities();
    client_capabilities.html = true;
    client_capabilities.services.push(ServiceCapability {
        kind: ServiceKind::PresentationQuery,
        operation: operation.into(),
        versions: VersionRange::exact(operation_version),
    });
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "html-query-test".into(),
            features: vec![RuntimeFeature::Html, RuntimeFeature::ExternalServices],
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
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "projection.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(source.into()),
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
    let request = (0..8)
        .find_map(|_| {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            let batch = drain(&mut session);
            let request = batch.iter().find_map(|message| match message {
                RuntimeMessage::ServiceRequest(request) if request.operation == operation => {
                    Some(request.clone())
                }
                _ => None,
            });
            messages.extend(batch);
            request
        })
        .unwrap_or_else(|| panic!("{operation} service request; phase={:?}", session.phase()));
    (session, request, messages)
}

fn submit_projection_resize(
    session: &mut RuntimeSession,
    sequence: u64,
    context: ProjectionQueryContext,
) {
    submit(
        session,
        sequence,
        RuntimeMessage::ProjectionObservation(ProjectionObservation {
            environment_revision: context.environment_revision + 1,
            presentation_revision: context.presentation_revision,
            client_size: ProjectionSize {
                width: ProjectionLength(1_600),
                height: ProjectionLength(900),
            },
            projection_space_revision: context.projection_space_revision + 1,
            line_columns: 100,
            text_box: String::new(),
            transform: ProjectionTransform {
                x_numerator: 1,
                x_denominator: 1,
                y_numerator: 1,
                y_denominator: 1,
                origin_x: ProjectionLength(0),
                origin_y: ProjectionLength(0),
            },
        }),
    );
}

fn assert_service_failure(session: &mut RuntimeSession) {
    for _ in 0..4 {
        if session.phase() == RuntimePhase::Faulted {
            break;
        }
        session.drive(RuntimeDriveBudget::default()).unwrap();
    }
    let messages = drain(session);
    assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:#?}");
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::ServiceFailure,
            ..
        })
    )));
}

#[test]
fn html_layout_query_is_revision_bound_and_commits_after_service_response() {
    let (mut session, request) = start_html_query(
        "@SYSTEM_TITLE\nRESULT = HTML_STRINGLEN(\"<b>x</b>\", 1)\nWAIT\nRETURN\n",
        HTML_STRING_LEN_OPERATION,
        HTML_STRING_LEN_OPERATION_VERSION,
    );
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
fn printed_html_query_survives_a_concurrent_projection_resize() {
    let (mut session, request) = start_html_query(
        "@SYSTEM_TITLE\nPRINTL title\nRESULTS '= HTML_GETPRINTEDSTR(0)\nWAIT\nRETURN\n",
        HTML_GET_PRINTED_STR_OPERATION,
        HTML_GET_PRINTED_STR_OPERATION_VERSION,
    );
    let payload: ProjectionStringIndexRequest =
        decode_canonical(request.payload.as_slice()).unwrap();
    submit_projection_resize(&mut session, 3, payload.context);
    submit(
        &mut session,
        4,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&ProjectionStringResponse {
                        context: payload.context,
                        value: "<p>title</p>".into(),
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
        read_runtime_string(session.vm.as_ref().unwrap(), "RESULTS").unwrap(),
        "<p>title</p>"
    );
}

#[test]
fn presentation_query_flushes_skipped_output_before_request() {
    let (session, request, messages) = start_html_query_with_messages(
        "@SYSTEM_TITLE\nSKIPLOG 1\nREDRAW 0\nHTML_PRINT \"<nobr><nonbutton><img src='portrait'></nonbutton></nobr>\"\nRESULTS '= HTML_GETPRINTEDSTR(0)\nRETURN\n",
        HTML_GET_PRINTED_STR_OPERATION,
        HTML_GET_PRINTED_STR_OPERATION_VERSION,
    );
    let payload: ProjectionStringIndexRequest =
        decode_canonical(request.payload.as_slice()).unwrap();
    let request_index = messages
        .iter()
        .position(|message| matches!(
            message,
            RuntimeMessage::ServiceRequest(candidate) if candidate.request_id == request.request_id
        ))
        .expect("presentation query request");
    let update_revision = messages[..request_index]
        .iter()
        .rev()
        .find_map(|message| match message {
            RuntimeMessage::PresentationSnapshot(snapshot) => Some(snapshot.revision),
            RuntimeMessage::PresentationDelta(delta) => Some(delta.new_revision),
            _ => None,
        })
        .expect("presentation update before query request");

    assert!(session.message_skip);
    assert_eq!(update_revision, payload.context.presentation_revision);
}

#[test]
fn printed_html_query_rejects_a_changed_canonical_presentation() {
    let (mut session, request) = start_html_query(
        "@SYSTEM_TITLE\nPRINTL title\nRESULTS '= HTML_GETPRINTEDSTR(0)\nWAIT\nRETURN\n",
        HTML_GET_PRINTED_STR_OPERATION,
        HTML_GET_PRINTED_STR_OPERATION_VERSION,
    );
    let payload: ProjectionStringIndexRequest =
        decode_canonical(request.payload.as_slice()).unwrap();
    session
        .presentation
        .append_print_text("changed".into(), false, false);
    submit(
        &mut session,
        3,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&ProjectionStringResponse {
                        context: payload.context,
                        value: "<p>title</p>".into(),
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    assert_service_failure(&mut session);
}

#[test]
fn html_layout_query_rejects_a_concurrent_projection_resize() {
    let (mut session, request) = start_html_query(
        "@SYSTEM_TITLE\nRESULT = HTML_STRINGLEN(\"<b>x</b>\", 1)\nWAIT\nRETURN\n",
        HTML_STRING_LEN_OPERATION,
        HTML_STRING_LEN_OPERATION_VERSION,
    );
    let payload: HtmlMeasureRequest = decode_canonical(request.payload.as_slice()).unwrap();
    submit_projection_resize(&mut session, 3, payload.context);
    submit(
        &mut session,
        4,
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
    assert_service_failure(&mut session);
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
