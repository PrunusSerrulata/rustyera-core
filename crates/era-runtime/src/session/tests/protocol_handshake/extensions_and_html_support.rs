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
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
    let (session, request, _) =
        start_html_query_with_messages(source, operation, operation_version);
    (session, request)
}

fn start_html_query_with_messages(
    source: &str,
    operation: &str,
    operation_version: ProtocolVersion,
) -> (RuntimeSession, ServiceRequest, Vec<RuntimeMessage>) {
    start_projection_service_with_messages(
        source,
        ServiceKind::PresentationQuery,
        operation,
        operation_version,
    )
}

fn start_projection_service_with_messages(
    source: &str,
    kind: ServiceKind,
    operation: &str,
    operation_version: ProtocolVersion,
) -> (RuntimeSession, ServiceRequest, Vec<RuntimeMessage>) {
    start_projection_service_with_profile(
        source,
        kind,
        operation,
        operation_version,
        era_runtime_protocol::CompatibilityIdentity::default(),
    )
}

fn start_projection_service_with_profile(
    source: &str,
    kind: ServiceKind,
    operation: &str,
    operation_version: ProtocolVersion,
    compatibility: era_runtime_protocol::CompatibilityIdentity,
) -> (RuntimeSession, ServiceRequest, Vec<RuntimeMessage>) {
    let profile = compatibility.profile;
    let mut client_capabilities = capabilities();
    client_capabilities.html = true;
    client_capabilities.graphics = kind == ServiceKind::Canvas;
    client_capabilities.services.push(ServiceCapability {
        kind,
        operation: operation.into(),
        versions: VersionRange::exact(operation_version),
    });
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "projection-query-test".into(),
            features: if kind == ServiceKind::Canvas {
                vec![
                    RuntimeFeature::Html,
                    RuntimeFeature::Graphics,
                    RuntimeFeature::ExternalServices,
                ]
            } else {
                vec![RuntimeFeature::Html, RuntimeFeature::ExternalServices]
            },
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
            compatibility,
            project_revision: 1,
            files: {
                let mut files = vec![SubmittedFile {
                    relative_path: "projection.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(source.into()),
                    content_hash: None,
                }];
                if profile == erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake {
                    files.insert(0, profile_configuration_file(profile));
                }
                files
            },
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
    let payload: era_runtime_protocol::HtmlMeasureRequestV2 =
        decode_canonical(request.payload.as_slice()).unwrap();
    assert_eq!(payload.probes.len(), 1);
    assert_eq!(
        payload.probes[0].mode,
        era_runtime_protocol::HtmlProbeModeV2::TextPart
    );
    assert!(!session.operations.is_snapshot_stable());
    submit(
        &mut session,
        3,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&html_test_measurement(&payload, 12_000)).unwrap(),
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
fn html_layout_query_commits_its_captured_context_after_projection_advances() {
    let (mut session, request) = start_html_query(
        "@SYSTEM_TITLE\nRESULT = HTML_STRINGLEN(\"<b>x</b>\", 1)\nWAIT\nRETURN\n",
        HTML_STRING_LEN_OPERATION,
        HTML_STRING_LEN_OPERATION_VERSION,
    );
    let payload: era_runtime_protocol::HtmlMeasureRequestV2 =
        decode_canonical(request.payload.as_slice()).unwrap();
    submit_projection_resize(&mut session, 3, payload.context);
    submit(
        &mut session,
        4,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&html_test_measurement(&payload, 12_000)).unwrap(),
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

// The service fixture measures each Unicode scalar at a fixed advance. It does not
// implement HTML parsing, line splitting or output serialization on the frontend.
fn html_test_measurement(
    request: &era_runtime_protocol::HtmlMeasureRequestV2,
    scalar_milli: i64,
) -> era_runtime_protocol::HtmlMeasureResponseV2 {
    use era_runtime_protocol::{
        HtmlCutAdvanceV2, HtmlProbeModeV2, HtmlProbeResponseV2, HtmlProbeResultV2,
    };
    fn text(nodes: &[erabasic_html::HtmlNode]) -> String {
        nodes
            .iter()
            .map(|node| match node {
                erabasic_html::HtmlNode::Text { text, .. } => text.clone(),
                erabasic_html::HtmlNode::Element { children, .. } => text(children),
            })
            .collect()
    }
    fn at<'a>(nodes: &'a [erabasic_html::HtmlNode], path: &[u32]) -> &'a str {
        match &nodes[usize::try_from(path[0]).unwrap()] {
            erabasic_html::HtmlNode::Text { text, .. } if path.len() == 1 => text,
            erabasic_html::HtmlNode::Element { children, .. } => at(children, &path[1..]),
            erabasic_html::HtmlNode::Text { .. } => panic!("invalid text path"),
        }
    }
    era_runtime_protocol::HtmlMeasureResponseV2 {
        context: request.context,
        probes: request
            .probes
            .iter()
            .map(|probe| HtmlProbeResponseV2 {
                id: probe.id,
                result: match probe.mode {
                    HtmlProbeModeV2::TextPart => HtmlProbeResultV2::TextMeasured {
                        advance_millipixels: i64::try_from(
                            text(&probe.document.nodes).chars().count(),
                        )
                        .unwrap()
                            * scalar_milli,
                        cuts: probe
                            .cuts
                            .iter()
                            .map(|cut| {
                                let source = at(&probe.document.nodes, &cut.text_node_path);
                                let prefix =
                                    &source[..usize::try_from(cut.decoded_utf8_offset).unwrap()];
                                assert_eq!(
                                    prefix.encode_utf16().count(),
                                    usize::try_from(cut.decoded_utf16_offset).unwrap()
                                );
                                HtmlCutAdvanceV2 {
                                    id: cut.id,
                                    advance_millipixels: i64::try_from(prefix.chars().count())
                                        .unwrap()
                                        * scalar_milli,
                                }
                            })
                            .collect(),
                    },
                    HtmlProbeModeV2::ImageSlot => HtmlProbeResultV2::ImageLoaded {
                        natural_width: 8,
                        natural_height: 8,
                    },
                    HtmlProbeModeV2::FixedSlot => HtmlProbeResultV2::FixedReady,
                },
            })
            .collect(),
    }
}

fn prepare_html_execution(
    source: &str,
    service_version: Option<ProtocolVersion>,
) -> RuntimeSession {
    prepare_html_execution_with_profile(
        source,
        service_version,
        erabasic_compat::CompatibilityIdentity::default(),
    )
}

fn prepare_html_execution_with_profile(
    source: &str,
    service_version: Option<ProtocolVersion>,
    compatibility: erabasic_compat::CompatibilityIdentity,
) -> RuntimeSession {
    let config = profile_configuration_file(compatibility.profile);
    let mut client = capabilities();
    client.html = true;
    if let Some(version) = service_version {
        for operation in [
            HTML_STRING_LEN_OPERATION,
            HTML_SUBSTRING_OPERATION,
            HTML_STRING_LINES_OPERATION,
        ] {
            client.services.push(ServiceCapability {
                kind: ServiceKind::PresentationQuery,
                operation: operation.into(),
                versions: VersionRange::exact(version),
            });
        }
    }
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "html-v2-fixture".into(),
            features: vec![
                RuntimeFeature::Html,
                RuntimeFeature::ExternalServices,
                RuntimeFeature::VmSnapshot,
            ],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client,
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
            compatibility,
            project_revision: 1,
            files: vec![
                config,
                SubmittedFile {
                    relative_path: "html-v2.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(source.into()),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Ready, "{messages:#?}");
    session
}

fn pump_html_execution(session: &mut RuntimeSession, sequence: &mut u64) -> Vec<RuntimeMessage> {
    let mut messages = Vec::new();
    for _ in 0..512 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let batch = drain(session);
        for message in &batch {
            if let RuntimeMessage::ServiceRequest(request) = message {
                assert_eq!(request.operation_version, ProtocolVersion::new(2, 0));
                let payload = decode_canonical(request.payload.as_slice()).unwrap();
                submit(
                    session,
                    *sequence,
                    RuntimeMessage::ServiceResponse(ServiceResponse {
                        request_id: request.request_id,
                        result: ServiceResult::Ready {
                            payload: ProtocolBytes::new(
                                encode_canonical(&html_test_measurement(&payload, 9000)).unwrap(),
                            ),
                        },
                    }),
                );
                *sequence += 1;
            }
        }
        messages.extend(batch);
        if matches!(
            session.phase(),
            RuntimePhase::WaitingInput | RuntimePhase::Faulted
        ) {
            return messages;
        }
    }
    panic!("HTML fixture did not reach a stable wait");
}

fn start_html_execution(session: &mut RuntimeSession) -> (u64, Vec<RuntimeMessage>) {
    submit(
        session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame {
                seed: Some(123_456),
            },
        }),
    );
    let mut sequence = 3;
    let messages = pump_html_execution(session, &mut sequence);
    (sequence, messages)
}
