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
fn html_layout_query_rejects_a_concurrent_projection_resize() {
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
    assert_service_failure(&mut session);
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

#[test]
fn snake_html_pixel_columns_preserve_alignment_width_and_normalized_markup() {
    let source = "@SYSTEM_TITLE\nHTML_PRINTC \"\", 40\nHTML_PRINTLC \"\"\nHTML_PRINTC \"<b>R</b>\", 40\nHTML_PRINTLC \"<i>L</i>\", 40\nHTML_PRINTC \"D\"\nHTML_PRINTLC \"wide\", 999\nPRINTL\nWAIT\nRETURN\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        None,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
    let project = session.project_snapshot.as_ref().unwrap();
    let default_pixel_width = project.print_c_length.saturating_mul(project.font_size) / 2;
    let snapshot = session.presentation.snapshot();
    let line = snapshot
        .history
        .logical_lines
        .iter()
        .find(|line| {
            line.runs
                .iter()
                .filter(|run| matches!(run, DisplayRun::ColumnCell { .. }))
                .count()
                == 4
        })
        .expect("HTML pixel cells share the current logical line");
    let cells = line
        .runs
        .iter()
        .filter_map(|run| match run {
            DisplayRun::ColumnCell {
                content,
                alignment,
                width,
            } => Some((content, *alignment, *width)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(cells[0].1, era_runtime_protocol::CellAlignment::Right);
    assert_eq!(cells[1].1, era_runtime_protocol::CellAlignment::Left);
    assert_eq!(
        cells.iter().map(|cell| cell.2).collect::<Vec<_>>(),
        [
            era_runtime_protocol::CellWidthIntent::LogicalPixels(40),
            era_runtime_protocol::CellWidthIntent::LogicalPixels(40),
            era_runtime_protocol::CellWidthIntent::LogicalPixels(default_pixel_width),
            era_runtime_protocol::CellWidthIntent::LogicalPixels(999),
        ]
    );
    assert!(matches!(
        cells[0].0.as_slice(),
        [DisplayRun::HtmlDocument { document }]
            if erabasic_html::serialize_document(document) == "<b>R</b>"
    ));
}

#[test]
fn rejected_html_attribute_emits_identity_scoped_diagnostic_without_raw_markup() {
    let source = "@SYSTEM_TITLE\nHTML_PRINT \"<img src='x' arbitrary='secret'>\"\nWAIT\nRETURN\n";
    let identity = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let mut session = prepare_html_execution_with_profile(source, None, identity.clone());
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    let diagnostic = messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::Diagnostic(diagnostic)
                if diagnostic.code == "runtime.html.profile_attribute_rejected" =>
            {
                Some(diagnostic)
            }
            _ => None,
        })
        .expect("profile-scoped HTML diagnostic");
    assert_eq!(
        diagnostic
            .context
            .as_ref()
            .and_then(|context| context.identity.as_ref()),
        Some(&identity)
    );
    assert!(!diagnostic.message.contains("secret"));
    assert!(session.presentation.snapshot().history.logical_lines.is_empty());
}

#[test]
fn snake_html_image_matrix_is_resolved_to_fixed_protocol_values() {
    let source = "@SYSTEM_TITLE\n#DIM MATRIX, 5, 5\nMATRIX:0:0 = 256\nMATRIX:4:4 = -7\nHTML_PRINT \"<img src='face' cm='MATRIX'>\"\nPRINTL\nWAIT\nRETURN\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        None,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
    let snapshot = session.presentation.snapshot();
    let matrix = snapshot
        .history
        .logical_lines
        .iter()
        .flat_map(|line| &line.runs)
        .find_map(|run| {
            let DisplayRun::HtmlDocument { document } = run else {
                return None;
            };
            let erabasic_html::HtmlNode::Element {
                semantic:
                    erabasic_html::HtmlElementSemantic::Image {
                        color_matrix: Some(erabasic_html::HtmlColorMatrix::Fixed(matrix)),
                        ..
                    },
                ..
            } = document.nodes.first()?
            else {
                return None;
            };
            Some(matrix)
        })
        .expect("resolved image color matrix");
    assert_eq!(matrix[0], 256);
    assert_eq!(matrix[24], -7);
}

#[test]
fn original_profile_keeps_rejecting_snake_only_html_attributes() {
    let source = "@SYSTEM_TITLE\nHTML_PRINT \"<font size='12'>x</font>\"\nWAIT\nRETURN\n";
    let identity = erabasic_compat::CompatibilityIdentity::reference();
    let mut session = prepare_html_execution_with_profile(source, None, identity.clone());
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:#?}");
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Diagnostic(diagnostic)
            if diagnostic.code == "runtime.html.profile_attribute_rejected"
                && diagnostic.context.as_ref().and_then(|context| context.identity.as_ref())
                    == Some(&identity)
    )));
}

#[test]
fn original_profile_rejects_snake_html_query_before_provider_observes_it() {
    let source = "@SYSTEM_TITLE\nRESULT = HTML_STRINGLEN(\"<font size='12'>x</font>\", 1)\nWAIT\nRETURN\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::reference(),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:#?}");
    assert!(!messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::ServiceRequest(request)
            if request.kind == ServiceKind::PresentationQuery
    )));
}

#[test]
fn snake_html_query_preserves_nested_font_render_intent_in_provider_probe() {
    let source = "@SYSTEM_TITLE\nRESULT = HTML_STRINGLEN(\"<font size='12' valign='middle' render='skia' edging='subpixel' hinting='full'><b>x</b></font>\", 1)\nWAIT\nRETURN\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
    let payload = messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.kind == ServiceKind::PresentationQuery =>
            {
                decode_canonical::<era_runtime_protocol::HtmlMeasureRequestV2>(
                    request.payload.as_slice(),
                )
                .ok()
            }
            _ => None,
        })
        .expect("presentation query probe");
    let font = payload.probes.iter().find_map(|probe| {
        fn find_font(
            nodes: &[erabasic_html::HtmlNode],
        ) -> Option<&erabasic_html::HtmlElementSemantic> {
            nodes.iter().find_map(|node| match node {
                erabasic_html::HtmlNode::Text { .. } => None,
                erabasic_html::HtmlNode::Element {
                    semantic, children, ..
                } => matches!(semantic, erabasic_html::HtmlElementSemantic::Font { .. })
                    .then_some(semantic)
                    .or_else(|| find_font(children)),
            })
        }
        find_font(&probe.document.nodes)
    });
    assert!(matches!(
        font,
        Some(erabasic_html::HtmlElementSemantic::Font {
            size_millipixels: Some(12_000),
            vertical_alignment: Some(erabasic_html::HtmlVerticalAlignment::Middle),
            render_intent: erabasic_html::HtmlTextRenderIntent {
                renderer: Some(erabasic_html::HtmlTextRenderer::Skia),
                edging: Some(erabasic_html::HtmlFontEdging::SubpixelAntiAlias),
                hinting: Some(erabasic_html::HtmlFontHinting::Full),
            },
            ..
        })
    ));
}

#[test]
fn snake_html_query_never_exposes_color_matrix_variable_addresses() {
    let source = "@SYSTEM_TITLE\n#DIM MATRIX, 5, 5\nRESULT = HTML_STRINGLEN(\"<img src='missing' cm='MATRIX'>\", 1)\nWAIT\nRETURN\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    let (_, messages) = start_html_execution(&mut session);
    let payload = messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.kind == ServiceKind::PresentationQuery =>
            {
                decode_canonical::<era_runtime_protocol::HtmlMeasureRequestV2>(
                    request.payload.as_slice(),
                )
                .ok()
            }
            _ => None,
        })
        .expect("image measurement probe");
    let erabasic_html::HtmlNode::Element {
        attributes,
        semantic: erabasic_html::HtmlElementSemantic::Image { color_matrix, .. },
        ..
    } = &payload.probes[0].document.nodes[0]
    else {
        panic!("image probe root");
    };
    assert!(attributes.iter().all(|attribute| attribute.name != "cm"));
    assert!(color_matrix.is_none());
}

fn html_flag(session: &RuntimeSession, index: u64) -> i64 {
    read_runtime_integer(session.vm.as_ref().unwrap(), "FLAG", &[index], None).unwrap()
}

#[test]
fn html_lazy_evaluation_preserves_flag_width_order_and_nested_flows() {
    let source = "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLEN(\"ab\", HTML_FLAG())\nFLAG:2 = HTML_STRINGLINES(\"abc\", HTML_WIDTH())\nFLAG:3 = HTML_STRINGLINES(\"\", HTML_WIDTH())\nWAIT\nRETURN\n@HTML_FLAG\n#FUNCTION\nFLAG:0 += 1\nRETURNF 1\n@HTML_WIDTH\n#FUNCTION\nFLAG:4 += 1\nFLAG:5 += HTML_STRINGLINES(\"x\", 1)\nRETURNF FLAG:4\n";
    let mut session = prepare_html_execution(source, Some(ProtocolVersion::new(2, 0)));
    start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!((html_flag(&session, 0), html_flag(&session, 1)), (1, 18));
    assert_eq!((html_flag(&session, 2), html_flag(&session, 3)), (2, 0));
    assert_eq!((html_flag(&session, 4), html_flag(&session, 5)), (2, 2));
    assert!(session.operations.html_lines.is_empty());
}

#[test]
fn html_malformed_source_fails_before_flag_and_empty_lines_still_requires_v2() {
    let mut session = prepare_html_execution(
        "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLEN(\"<not-supported>\", HTML_FLAG())\nWAIT\nRETURN\n@HTML_FLAG\n#FUNCTION\nFLAG:0 += 1\nRETURNF 1\n",
        Some(ProtocolVersion::new(2, 0)),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:#?}");
    assert_eq!(html_flag(&session, 0), 0);
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::ServiceRequest(_)))
    );
    for version in [None, Some(ProtocolVersion::new(1, 0))] {
        let mut session = prepare_html_execution(
            "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLINES(\"\", HTML_WIDTH())\nWAIT\nRETURN\n@HTML_WIDTH\n#FUNCTION\nFLAG:0 += 1\nRETURNF 1\n",
            version,
        );
        let (_, messages) = start_html_execution(&mut session);
        assert_eq!(html_flag(&session, 0), 0);
        assert!(messages.iter().any(|message| matches!(message, RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::UnsupportedRuntimeFeature, context: Some(context), ..
        }) if context.required_capability.as_ref().is_some_and(|required|
            required.operation == HTML_STRING_LINES_OPERATION && required.version == ProtocolVersion::new(2, 0)))));
    }
}

fn html_result(vm: &RuntimeVm, index: u64) -> VmValue {
    vm.read_runtime_state(&[erabasic_vm::VmRuntimeRead {
        variable: runtime_variable_key(vm, "RESULTS").unwrap(),
        indices: vec![index],
        character: None,
    }])
    .unwrap()
    .remove(0)
}

#[test]
fn html_substring_keeps_results_atomic_and_rejects_bad_probe_response() {
    let source = "@SYSTEM_TITLE\nRESULTS:0 = old-head\nRESULTS:1 = old-tail\nSTR:0 '= HTML_SUBSTRING(\"a😀b\", 2)\nWAIT\nRETURN\n";
    let (mut session, first) = start_html_query(
        source,
        HTML_SUBSTRING_OPERATION,
        HTML_SUBSTRING_OPERATION_VERSION,
    );
    let mut payload: era_runtime_protocol::HtmlMeasureRequestV2 =
        decode_canonical(first.payload.as_slice()).unwrap();
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(html_result(vm, 0), VmValue::String("old-head".into()));
    payload.probes[0].id += 1;
    submit(
        &mut session,
        3,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: first.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&html_test_measurement(&payload, 9000)).unwrap(),
                ),
            },
        }),
    );
    assert_service_failure(&mut session);
    assert_eq!(
        html_result(session.vm.as_ref().unwrap(), 0),
        VmValue::String("old-head".into())
    );
    assert_eq!(
        html_result(session.vm.as_ref().unwrap(), 1),
        VmValue::String("old-tail".into())
    );
    let mut session = prepare_html_execution(source, Some(ProtocolVersion::new(2, 0)));
    start_html_execution(&mut session);
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(html_result(vm, 0), VmValue::String("a😀".into()));
    assert_eq!(html_result(vm, 1), VmValue::String("b".into()));
}

#[test]
fn html_lines_input_snapshot_restores_exact_flow_and_rejects_tampered_owner() {
    for dynamic in [false, true] {
        let source = "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLINES(\"abc\", HTML_WIDTH())\nWAIT\nRETURN\n@HTML_WIDTH\n#FUNCTION\nFLAG:0 += 1\nIF FLAG:0 == 1\nINPUT\nENDIF\nRETURNF 2\n";
        let source = if dynamic {
            "@SYSTEM_TITLE\nRESULTS:10 = {HTML_STRINGLINES(\"abc\", HTML_WIDTH())}\nFLAG:1 = TOINT(STRFORM(RESULTS:10))\nWAIT\nRETURN\n@HTML_WIDTH\n#FUNCTION\nFLAG:0 += 1\nIF FLAG:0 == 1\nINPUT\nENDIF\nRETURNF 2\n"
        } else {
            source
        };
        let mut session = prepare_html_execution(source, Some(ProtocolVersion::new(2, 0)));
        let (mut sequence, _) = start_html_execution(&mut session);
        assert_eq!(session.operations.html_lines.len(), 1);
        assert!(session.operations.is_snapshot_stable());
        session
            .export_state(
                100,
                StateExportRequest {
                    kind: StateExportKind::VmSnapshot,
                    snapshot_purpose: SnapshotExportPurpose::Normal,
                },
            )
            .unwrap();
        let bytes = session
            .outbound_transfer
            .take()
            .expect("stable HTML width INPUT snapshot")
            .bytes;
        drain(&mut session);
        let before_epoch = session.epoch;
        let before_wait = session.operations.active_input().unwrap().wait.clone();
        for field in [
            "epoch",
            "frame",
            "generation",
            "depth",
            "count",
            "in_flight",
        ] {
            let mut snapshot = runtime_snapshot::decode(&bytes, usize::MAX).unwrap();
            let mut json = serde_json::to_value(&snapshot.operations).unwrap();
            let flow = json["html_lines"]["entries"]
                .as_object_mut()
                .unwrap()
                .values_mut()
                .next()
                .unwrap();
            flow[field] = if field == "in_flight" {
                serde_json::json!(true)
            } else {
                serde_json::json!(999_999)
            };
            snapshot.operations = serde_json::from_value(json).unwrap();
            session
                .start_vm_snapshot(101, &runtime_snapshot::encode(&snapshot).unwrap())
                .unwrap();
            let messages = drain(&mut session);
            assert!(
                messages
                    .iter()
                    .any(|message| matches!(message, RuntimeMessage::CommandRejected(_))),
                "{field}: {messages:#?}"
            );
            assert_eq!(session.epoch, before_epoch);
            assert_eq!(session.operations.active_input().unwrap().wait, before_wait);
        }
        session.start_vm_snapshot(102, &bytes).unwrap();
        drain(&mut session);
        assert_ne!(session.epoch, before_epoch);
        assert_eq!(session.operations.html_lines.len(), 1);
        session
            .operations
            .html_lines
            .validate_snapshot(session.vm.as_ref().unwrap(), session.epoch.0)
            .unwrap();
        let wait = session.operations.active_input().unwrap().wait.clone();
        submit(
            &mut session,
            sequence,
            RuntimeMessage::Input(FrontendInput {
                wait_id: wait.wait_id,
                token: wait.submission_token,
                monotonic_time_ns: 1,
                intent: InputIntent::CommitText("7".into()),
                message_skip: false,
            }),
        );
        sequence += 1;
        let messages = pump_html_execution(&mut session, &mut sequence);
        assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
        assert_eq!((html_flag(&session, 0), html_flag(&session, 1)), (2, 2));
        assert!(session.operations.html_lines.is_empty());
    }
}

#[test]
fn html_lines_reload_blocks_active_flow_and_cancellation_clears_it() {
    for source in [
        "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLINES(\"abc\", HTML_WIDTH())\nWAIT\nRETURN\n@HTML_WIDTH\n#FUNCTION\nINPUT\nRETURNF 1\n",
        "@SYSTEM_TITLE\nRESULTS:9 = {HTML_STRINGLINES(\"abc\", HTML_WIDTH())}\nFLAG:1 = TOINT(STRFORM(RESULTS:9))\nWAIT\nRETURN\n@HTML_WIDTH\n#FUNCTION\nINPUT\nRETURNF 1\n",
    ] {
        let mut session = prepare_html_execution(source, Some(ProtocolVersion::new(2, 0)));
        let (sequence, _) = start_html_execution(&mut session);
        let before_epoch = session.epoch;
        session
            .reload_project(
                100,
                &ReloadProject {
                    base_revision: 1,
                    target_revision: 2,
                    changes: Vec::new(),
                },
            )
            .unwrap();
        let messages = drain(&mut session);
        assert!(messages.iter().any(|message| matches!(message,
        RuntimeMessage::CommandRejected(CommandRejected { message, .. }) if message.contains("ActiveBlocks"))));
        assert_eq!(session.epoch, before_epoch);
        assert_eq!(session.operations.html_lines.len(), 1);
        submit(
            &mut session,
            sequence,
            RuntimeMessage::ReturnToTitle(era_runtime_protocol::ReturnToTitleRequest {}),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        assert!(session.operations.html_lines.is_empty());
        assert_ne!(session.epoch, before_epoch);
    }
}

#[test]
fn html_lines_no_progress_fault_clears_flows_without_results_side_effects() {
    let source =
        "@SYSTEM_TITLE\nRESULTS:0 = unchanged\nFLAG:1 = HTML_STRINGLINES(\"x\", 0)\nWAIT\nRETURN\n";
    let mut session = prepare_html_execution(source, Some(ProtocolVersion::new(2, 0)));
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert!(messages.iter().any(|message| matches!(message,
        RuntimeMessage::Fault(RuntimeFault { message, .. }) if message.contains("NoProgress"))));
    assert_eq!(
        html_result(session.vm.as_ref().unwrap(), 0),
        VmValue::String("unchanged".into())
    );
    assert!(session.operations.html_lines.is_empty());
}

#[test]
fn html_later_measurement_round_is_cancelled_and_old_reply_cannot_resume_it() {
    for source in [
        "@SYSTEM_TITLE\nSTR:0 '= HTML_SUBSTRING(\"abc\", 1)\nWAIT\nRETURN\n",
        "@SYSTEM_TITLE\nRESULTS:9 = %HTML_SUBSTRING(\"abc\", 1)%\nSTR:0 '= STRFORM(RESULTS:9)\nWAIT\nRETURN\n",
    ] {
        let (mut session, first) = start_html_query(
            source,
            HTML_SUBSTRING_OPERATION,
            HTML_SUBSTRING_OPERATION_VERSION,
        );
        let payload = decode_canonical(first.payload.as_slice()).unwrap();
        submit(
            &mut session,
            3,
            RuntimeMessage::ServiceResponse(ServiceResponse {
                request_id: first.request_id,
                result: ServiceResult::Ready {
                    payload: ProtocolBytes::new(
                        encode_canonical(&html_test_measurement(&payload, 9000)).unwrap(),
                    ),
                },
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let second = drain(&mut session)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::ServiceRequest(request)
                    if request.operation == HTML_SUBSTRING_OPERATION =>
                {
                    Some(request)
                }
                _ => None,
            })
            .expect("second measurement under the original VM wait");
        assert_ne!(first.request_id, second.request_id);
        session.return_to_title(100).unwrap();
        let cancelled = drain(&mut session);
        assert!(
            cancelled.iter().any(|message| matches!(message,
        RuntimeMessage::CancelExternalRequest(request) if request.request_id == second.request_id))
        );
        let after_epoch = session.epoch;
        let payload = decode_canonical(second.payload.as_slice()).unwrap();
        // Even an attacker rebinding the old payload to the current envelope epoch has
        // no pending request to complete. The genuine old-epoch envelope is rejected earlier.
        session
            .complete_service(
                101,
                ServiceResponse {
                    request_id: second.request_id,
                    result: ServiceResult::Ready {
                        payload: ProtocolBytes::new(
                            encode_canonical(&html_test_measurement(&payload, 9000)).unwrap(),
                        ),
                    },
                },
            )
            .unwrap();
        let messages = drain(&mut session);
        assert!(messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::CommandRejected(CommandRejected {
                code: CommandErrorCode::StaleRequest,
                ..
            })
        )));
        assert_eq!(session.epoch, after_epoch);
    }
}

#[test]
fn html_length_flag_is_not_evaluated_while_measurements_are_pending() {
    let source = "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLEN(\"ab\", HTML_FLAG())\nWAIT\nRETURN\n@HTML_FLAG\n#FUNCTION\nFLAG:0 += 1\nRETURNF 1\n";
    let (mut session, request) = start_html_query(
        source,
        HTML_STRING_LEN_OPERATION,
        HTML_STRING_LEN_OPERATION_VERSION,
    );
    assert_eq!(html_flag(&session, 0), 0);
    let payload = decode_canonical(request.payload.as_slice()).unwrap();
    submit(
        &mut session,
        3,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&html_test_measurement(&payload, 9000)).unwrap(),
                ),
            },
        }),
    );
    let mut sequence = 4;
    pump_html_execution(&mut session, &mut sequence);
    assert_eq!((html_flag(&session, 0), html_flag(&session, 1)), (1, 18));
}

#[test]
fn html_nested_flow_limit_fails_before_allocating_a_seventeenth_flow() {
    let source = "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLINES(\"x\", HTML_WIDTH())\nWAIT\nRETURN\n@HTML_WIDTH\n#FUNCTION\nFLAG:0 += 1\nRETURNF HTML_STRINGLINES(\"x\", HTML_WIDTH())\n";
    let mut session = prepare_html_execution(source, Some(ProtocolVersion::new(2, 0)));
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::ResourceLimit,
            ..
        })
    )));
    assert_eq!(html_flag(&session, 0), 16);
    assert!(session.operations.html_lines.is_empty());
}

#[test]
fn html_response_checks_all_three_revisions_and_preserves_backend_categories() {
    let source = "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLEN(\"x\", 1)\nWAIT\nRETURN\n";
    for field in ["presentation", "environment", "space"] {
        let (mut session, request) = start_html_query(
            source,
            HTML_STRING_LEN_OPERATION,
            HTML_STRING_LEN_OPERATION_VERSION,
        );
        let payload = decode_canonical(request.payload.as_slice()).unwrap();
        let mut response = html_test_measurement(&payload, 9000);
        match field {
            "presentation" => response.context.presentation_revision += 1,
            "environment" => response.context.environment_revision += 1,
            _ => response.context.projection_space_revision += 1,
        }
        submit(
            &mut session,
            3,
            RuntimeMessage::ServiceResponse(ServiceResponse {
                request_id: request.request_id,
                result: ServiceResult::Ready {
                    payload: ProtocolBytes::new(encode_canonical(&response).unwrap()),
                },
            }),
        );
        assert_service_failure(&mut session);
        assert_eq!(html_flag(&session, 1), 0, "{field}");
    }
    for (category, code) in [
        ("unsupported", FaultCode::UnsupportedRuntimeFeature),
        ("invalid_request", FaultCode::ServiceFailure),
        ("stale_projection", FaultCode::ServiceFailure),
        ("resource_limit", FaultCode::ResourceLimit),
        ("backend_failure", FaultCode::ServiceFailure),
        (
            "frontend.unsupported_service",
            FaultCode::UnsupportedRuntimeFeature,
        ),
        ("frontend.resource_limit", FaultCode::ResourceLimit),
        ("frontend.invalid_request", FaultCode::ServiceFailure),
        ("frontend.stale_projection", FaultCode::ServiceFailure),
        ("frontend.backend_failure", FaultCode::ServiceFailure),
        ("unknown_backend_error", FaultCode::ServiceFailure),
    ] {
        let (mut session, request) = start_html_query(
            source,
            HTML_STRING_LEN_OPERATION,
            HTML_STRING_LEN_OPERATION_VERSION,
        );
        submit(
            &mut session,
            3,
            RuntimeMessage::ServiceResponse(ServiceResponse {
                request_id: request.request_id,
                result: ServiceResult::Error {
                    error: era_runtime_protocol::ServiceError {
                        code: category.into(),
                        message: "fixture failure".into(),
                    },
                },
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let messages = drain(&mut session);
        assert!(
            messages.iter().any(|message| matches!(message,
            RuntimeMessage::Fault(RuntimeFault { code: actual, message, .. })
            if *actual == code && message.contains(category))),
            "{messages:#?}"
        );
        assert_eq!(html_flag(&session, 1), 0);
    }
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

fn complete_projection_reply(
    session: &mut RuntimeSession,
    request: &ServiceRequest,
    payload: Vec<u8>,
) {
    submit(
        session,
        3,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(payload),
            },
        }),
    );
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if matches!(
            session.phase(),
            RuntimePhase::WaitingInput | RuntimePhase::Faulted
        ) {
            break;
        }
    }
}

#[test]
fn pointer_query_flushes_prints_and_returns_each_canonical_value() {
    for (expression, integer, string) in [
        ("MOUSEX()", Some(37), None),
        ("MOUSEY()", Some(-91), None),
        ("MOUSEB()", None, Some("script-value")),
    ] {
        let assignment = if integer.is_some() {
            format!("RESULT = {expression}")
        } else {
            format!("RESULTS '= {expression}")
        };
        let source =
            format!("@SYSTEM_TITLE\nREDRAW 0\nPRINTL before-pointer\n{assignment}\nWAIT\nRETURN\n");
        let (mut session, request, messages) = start_projection_service_with_messages(
            &source,
            ServiceKind::InputState,
            POINTER_STATE_OPERATION,
            POINTER_STATE_OPERATION_VERSION,
        );
        let query: PointerStateRequest = decode_canonical(request.payload.as_slice()).unwrap();
        let service_index = messages.iter().position(|message| matches!(message, RuntimeMessage::ServiceRequest(value) if value.request_id == request.request_id)).unwrap();
        assert!(
            messages[..service_index].iter().any(|message| matches!(
                message,
                RuntimeMessage::PresentationDelta(_) | RuntimeMessage::PresentationSnapshot(_)
            )),
            "{messages:?}"
        );
        assert_eq!(query.presentation_revision, session.presentation.revision());
        complete_projection_reply(
            &mut session,
            &request,
            encode_canonical(&PointerStateResponse {
                x: ProjectionLength(37),
                y: ProjectionLength(-91),
                button_value: "script-value".into(),
                presentation_revision: query.presentation_revision,
                environment_revision: query.environment_revision,
                projection_space_revision: query.projection_space_revision,
            })
            .unwrap(),
        );
        assert_eq!(session.phase(), RuntimePhase::WaitingInput);
        let vm = session.vm.as_ref().unwrap();
        if let Some(value) = integer {
            assert_eq!(
                read_runtime_integer(vm, "RESULT", &[], None).unwrap(),
                value
            );
        }
        if let Some(value) = string {
            assert_eq!(
                vm.vm()
                    .read_variable(runtime_variable_key(vm, "RESULTS").unwrap(), &[0], None)
                    .unwrap(),
                VmValue::String(value.into())
            );
        }
    }
}

#[test]
fn snake_getliney_resolves_a_display_index_to_revision_bound_stable_geometry() {
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let (mut session, request, messages) = start_projection_service_with_profile(
        "@SYSTEM_TITLE\nPRINTL first\nPRINTL second\nRESULT = GETLINEY(0)\nWAIT\nRETURN\n",
        ServiceKind::PresentationQuery,
        GET_LINE_GEOMETRY_OPERATION,
        GET_LINE_GEOMETRY_OPERATION_VERSION,
        snake,
    );
    let query: GetLineGeometryV1Request =
        decode_canonical(request.payload.as_slice()).unwrap();
    assert_eq!(query.line_id, 1);
    assert_eq!(query.context.presentation_revision, session.presentation.revision());
    let request_index = messages
        .iter()
        .position(|message| matches!(
            message,
            RuntimeMessage::ServiceRequest(value) if value.request_id == request.request_id
        ))
        .unwrap();
    assert!(messages[..request_index].iter().any(|message| matches!(
        message,
        RuntimeMessage::PresentationDelta(_) | RuntimeMessage::PresentationSnapshot(_)
    )));

    complete_projection_reply(
        &mut session,
        &request,
        encode_canonical(&GetLineGeometryV1Response {
            context: query.context,
            line_id: query.line_id,
            top: ProjectionLength(100),
            height: ProjectionLength(20),
            viewport_height: ProjectionLength(80),
        })
        .unwrap(),
    );
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        40
    );
}

#[test]
fn snake_getliney_rejects_stale_geometry_without_committing_the_assignment() {
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let (mut session, request, _) = start_projection_service_with_profile(
        "@SYSTEM_TITLE\nPRINTL first\nRESULT = 77\nRESULT = GETLINEY(0)\nWAIT\nRETURN\n",
        ServiceKind::PresentationQuery,
        GET_LINE_GEOMETRY_OPERATION,
        GET_LINE_GEOMETRY_OPERATION_VERSION,
        snake,
    );
    let query: GetLineGeometryV1Request =
        decode_canonical(request.payload.as_slice()).unwrap();
    let mut stale = query.context;
    stale.projection_space_revision = stale.projection_space_revision.saturating_add(1);
    complete_projection_reply(
        &mut session,
        &request,
        encode_canonical(&GetLineGeometryV1Response {
            context: stale,
            line_id: query.line_id,
            top: ProjectionLength(100),
            height: ProjectionLength(20),
            viewport_height: ProjectionLength(80),
        })
        .unwrap(),
    );
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        77
    );
}

#[test]
fn canvas_sampling_flushes_new_draws_before_query_and_returns_argb() {
    let (mut session, request, messages) = start_projection_service_with_messages(
        "@SYSTEM_TITLE\nREDRAW 0\nRESULT = GCREATE(1, 2, 2)\nRESULT = GCLEAR(1, 4279312947)\nRESULT = GGETCOLOR(1, 0, 0)\nWAIT\nRETURN\n",
        ServiceKind::Canvas,
        SAMPLE_CANVAS_PIXEL_OPERATION,
        SAMPLE_CANVAS_PIXEL_OPERATION_VERSION,
    );
    let query: CanvasPixelRequest = decode_canonical(request.payload.as_slice()).unwrap();
    let service_index = messages.iter().position(|message| matches!(message, RuntimeMessage::ServiceRequest(value) if value.request_id == request.request_id)).unwrap();
    let resources = messages[..service_index]
        .iter()
        .rev()
        .find_map(|message| match message {
            RuntimeMessage::PresentationSnapshot(snapshot) => Some(&snapshot.resources),
            RuntimeMessage::PresentationDelta(delta) => {
                delta
                    .operations
                    .iter()
                    .find_map(|operation| match operation {
                        PresentationOperation::SetResources { resources } => Some(resources),
                        _ => None,
                    })
            }
            _ => None,
        })
        .expect("current replay must precede the sample request");
    let canvas = resources
        .canvases
        .iter()
        .find(|canvas| canvas.canvas_id == query.canvas_id)
        .expect("new canvas must be present even without a mounted display");
    assert_eq!(canvas.revision, query.canvas_revision);
    assert!(canvas.commands.iter().any(|command| matches!(
        command,
        era_runtime_protocol::CanvasReplayCommand::Clear {
            argb: 0xff11_2233,
            rectangle: None
        }
    )));
    assert_eq!(
        query.context.presentation_revision,
        session.presentation.revision()
    );
    complete_projection_reply(
        &mut session,
        &request,
        encode_canonical(&CanvasPixelResponse {
            context: query.context,
            canvas_revision: query.canvas_revision,
            argb: 0xff11_2233,
        })
        .unwrap(),
    );
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        0xff11_2233
    );
}

#[test]
fn canvas_sampling_rejects_matching_reply_after_the_current_canvas_changes() {
    for remove in [false, true] {
        let (mut session, request, _) = start_projection_service_with_messages(
            "@SYSTEM_TITLE\nRESULT = GCREATE(1, 2, 2)\nRESULT = GGETCOLOR(1, 0, 0)\nWAIT\nRETURN\n",
            ServiceKind::Canvas,
            SAMPLE_CANVAS_PIXEL_OPERATION,
            SAMPLE_CANVAS_PIXEL_OPERATION_VERSION,
        );
        let query: CanvasPixelRequest = decode_canonical(request.payload.as_slice()).unwrap();
        let graph = &mut session.project_snapshot.as_mut().unwrap().resource_graph;
        if remove {
            assert!(graph.dispose_canvas(1));
        } else {
            assert!(graph.clear_canvas(1, 0, None));
        }
        // Simulate an independent resource-generation change without a projection revision.
        // Matching the outstanding reply alone must not authorize this old raster.
        complete_projection_reply(
            &mut session,
            &request,
            encode_canonical(&CanvasPixelResponse {
                context: query.context,
                canvas_revision: query.canvas_revision,
                argb: 0,
            })
            .unwrap(),
        );
        assert_service_failure(&mut session);
    }
}

#[test]
fn malformed_pointer_and_canvas_replies_fault_without_losing_the_host_wait() {
    let queries = [
        (
            ServiceKind::InputState,
            POINTER_STATE_OPERATION,
            POINTER_STATE_OPERATION_VERSION,
            "",
            "RESULT = MOUSEX()",
        ),
        (
            ServiceKind::InputState,
            POINTER_STATE_OPERATION,
            POINTER_STATE_OPERATION_VERSION,
            "",
            "RESULTS '= MOUSEB()",
        ),
        (
            ServiceKind::Canvas,
            SAMPLE_CANVAS_PIXEL_OPERATION,
            SAMPLE_CANVAS_PIXEL_OPERATION_VERSION,
            "RESULT = GCREATE(1, 2, 2)\n",
            "RESULT = GGETCOLOR(1, 0, 0)",
        ),
    ];
    for (kind, operation, version, setup, assignment) in queries {
        for payload in [
            vec![0xa1, 0x00],                   // Truncated field value.
            vec![0xa1, 0x00, 0x61, b'x'],       // Wrong type for the first field.
            vec![0xa2, 0x00, 0x00, 0x00, 0x00], // Duplicate deterministic map key.
            vec![0xbf, 0xff],                   // Indefinite maps are not deterministic.
            vec![0xa0, 0x00],                   // Trailing data after a complete map.
        ] {
            let source = format!(
                "@SYSTEM_TITLE\n{setup}RESULT = 777\nRESULTS '= \"kept\"\nRESULT:9 = 991\n{assignment}\nRESULT:9 = 0\nWAIT\nRETURN\n"
            );
            let (mut session, request, _) =
                start_projection_service_with_messages(&source, kind, operation, version);
            complete_projection_reply(&mut session, &request, payload.clone());
            assert_service_failure(&mut session);
            let vm = session.vm.as_ref().unwrap();
            assert_eq!(read_runtime_integer(vm, "RESULT", &[], None).unwrap(), 777);
            assert_eq!(read_runtime_integer(vm, "RESULT", &[9], None).unwrap(), 991);
            assert_eq!(
                vm.vm()
                    .read_variable(runtime_variable_key(vm, "RESULTS").unwrap(), &[0], None)
                    .unwrap(),
                VmValue::String("kept".into()),
            );

            // Exercise the public drive path again: the failed request cannot be
            // rebound, and the session must remain faulted rather than waiting.
            submit(
                &mut session,
                4,
                RuntimeMessage::ServiceResponse(ServiceResponse {
                    request_id: request.request_id,
                    result: ServiceResult::Ready {
                        payload: ProtocolBytes::new(payload),
                    },
                }),
            );
            session.drive(RuntimeDriveBudget::default()).unwrap();
            let messages = drain(&mut session);
            assert_eq!(session.phase(), RuntimePhase::Faulted);
            assert!(
                messages.iter().any(|message| matches!(
                    message,
                    RuntimeMessage::CommandRejected(CommandRejected {
                        code: CommandErrorCode::StaleRequest,
                        ..
                    })
                )),
                "{operation}: {messages:#?}"
            );
        }
    }
}

#[test]
fn snake_strformcheck_catches_later_html_parser_fault_after_service_completion() {
    let source = "@SYSTEM_TITLE\nRESULTS:0 = old-head\nRESULTS:1 = old-tail\nFLAG:0 = STRFORMCHECK(\"%BAD_HTML()%\")\nFLAG:1 = 1\nWAIT\nRETURN\n@BAD_HTML\n#FUNCTIONS\nRETURNF HTML_SUBSTRING(\"a</b>\", 100)\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!((html_flag(&session, 0), html_flag(&session, 1)), (0, 1));
    assert!(
        messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::ServiceRequest(_)))
    );
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_)))
    );
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(html_result(vm, 0), VmValue::String("old-head".into()));
    assert_eq!(html_result(vm, 1), VmValue::String("old-tail".into()));
    assert!(session.operations.html_lines.is_empty());
}

#[test]
fn snake_strformcheck_cannot_catch_frontend_claimed_script_failure() {
    let source = "@SYSTEM_TITLE\nFLAG:0 = STRFORMCHECK(\"%MEASURE_HTML()%\")\nFLAG:1 = 1\nWAIT\nRETURN\n@MEASURE_HTML\n#FUNCTIONS\nRETURNF HTML_SUBSTRING(\"abc\", 100)\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame {
                seed: Some(123_456),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request) => Some(request),
            _ => None,
        })
        .expect("measurement request");
    submit(
        &mut session,
        3,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Error {
                error: era_runtime_protocol::ServiceError {
                    code: "script.parse".into(),
                    message: "frontend cannot declare ScriptInput".into(),
                },
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert_eq!(html_flag(&session, 1), 0);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::ServiceFailure,
            ..
        })
    )));
}

#[test]
fn direct_html_host_failure_catches_and_abandons_only_its_live_flow_scope() {
    let source = "@SYSTEM_TITLE\nRESULTS:0 '= \"{HTML_STRINGLINES(\\\"abc\\\", WIDTH())}\"\nFLAG:0 = STRFORMCHECK(RESULTS:0)\nFLAG:2 = 1\nWAIT\nRETURN\n@WIDTH\n#FUNCTION\nFLAG:1 += 1\nTHROW width-failed\nRETURNF 1\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:?}");
    assert_eq!(
        (
            html_flag(&session, 0),
            html_flag(&session, 1),
            html_flag(&session, 2)
        ),
        (0, 1, 1)
    );
    assert!(session.operations.html_lines.is_empty());
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_)))
    );
}

#[test]
fn direct_html_outer_scope_survives_inner_check_and_repeated_width_evaluation() {
    let source = "@SYSTEM_TITLE\nRESULTS:0 '= \"{HTML_STRINGLINES(\\\"abc\\\", WIDTH())}\"\nFLAG:0 = STRFORMCHECK(RESULTS:0)\nWAIT\nRETURN\n@WIDTH\n#FUNCTION\nFLAG:1 += 1\nFLAG:2 = STRFORMCHECK(\"{FAIL()}\")\nRETURNF 1\n@FAIL\n#FUNCTION\nTHROW inner-failed\nRETURNF 0\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:?}");
    assert_eq!(html_flag(&session, 0), 1);
    assert!(
        html_flag(&session, 1) > 1,
        "width must run again for each nonempty tail"
    );
    assert_eq!(html_flag(&session, 2), 0);
    assert!(session.operations.html_lines.is_empty());
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_)))
    );
}

#[test]
fn direct_html_host_cannot_catch_frontend_claimed_script_failure() {
    let source = "@SYSTEM_TITLE\nRESULTS:0 '= \"%HTML_SUBSTRING(\\\"abc\\\",100)%\"\nFLAG:0 = STRFORMCHECK(RESULTS:0)\nFLAG:1 = 1\nWAIT\nRETURN\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame {
                seed: Some(123_456),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request) => Some(request),
            _ => None,
        })
        .expect("direct measurement request");
    submit(
        &mut session,
        3,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Error {
                error: era_runtime_protocol::ServiceError {
                    code: "script.parse".into(),
                    message: "untrusted frontend failure".into(),
                },
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert_eq!(html_flag(&session, 1), 0);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::ServiceFailure,
            ..
        })
    )));
    assert!(session.operations.html_lines.is_empty());
}
