use era_debug_protocol::{
    AuthorizedDebugRequest, DEBUG_PROTOCOL_VERSION, DebugCommand, DebugHello, DebugMessage,
    DebugRevoke, DebugScope, GrantToken,
};
use era_protocol::{Channel, Envelope, ProtocolBytes, decode_envelope, encode_envelope};
use era_runtime_protocol::{
    DisplayRun, FileCategory, FileChange, FilePayload, ProjectIdentity, ProjectManifest,
    SubmittedFile,
};
use erabasic_vm::VmDebugInspect;

use super::*;

fn capabilities() -> ClientCapabilities {
    ClientCapabilities {
        input_modalities: vec![era_runtime_protocol::InputModality::Keyboard],
        rich_text: false,
        html: false,
        graphics: false,
        audio: false,
        video: false,
        font_metrics: false,
        column_cells: true,
        separators: true,
        available_fonts: vec!["sans-serif".into()],
        services: vec![
            ServiceCapability {
                kind: ServiceKind::Clock,
                operation: LOCAL_DATE_TIME_OPERATION.into(),
                versions: VersionRange::exact(LOCAL_DATE_TIME_OPERATION_VERSION),
            },
            ServiceCapability {
                kind: ServiceKind::Entropy,
                operation: RANDOM_SEED_OPERATION.into(),
                versions: VersionRange::exact(RANDOM_SEED_OPERATION_VERSION),
            },
            ServiceCapability {
                kind: ServiceKind::InputState,
                operation: GET_KEY_STATE_OPERATION.into(),
                versions: VersionRange::exact(GET_KEY_STATE_OPERATION_VERSION),
            },
        ],
        storage: StorageCapabilities {
            revisions: true,
            atomic_replace: true,
            missing_precondition: true,
            delete: true,
        },
    }
}

#[allow(clippy::needless_pass_by_value)]
fn submit(session: &mut RuntimeSession, sequence: u64, message: RuntimeMessage) {
    let mut envelope = Envelope::new(
        Channel::Runtime,
        RUNTIME_PROTOCOL_VERSION,
        sequence,
        sequence + 1,
        message.tag(),
        ProtocolBytes::new(message.encode_payload().expect("encode message")),
    );
    if sequence != 0 {
        envelope.session = Some(session.options.session_id);
        envelope.session_epoch = Some(session.epoch);
    }
    let bytes = encode_envelope(&envelope, WireLimits::default()).expect("encode envelope");
    session.submit_envelope(&bytes).expect("submit envelope");
}

fn drain(session: &mut RuntimeSession) -> Vec<RuntimeMessage> {
    let mut messages = Vec::new();
    while let Some(bytes) = session.poll_envelope() {
        let envelope = decode_envelope(&bytes, WireLimits::default()).expect("decode envelope");
        messages.push(RuntimeMessage::from_envelope(&envelope).expect("decode message"));
    }
    messages
}

fn submit_debug(session: &mut RuntimeSession, sequence: u64, message: &DebugMessage) {
    let envelope = message
        .envelope(
            Some(session.options.session_id),
            Some(session.epoch),
            sequence,
            10_000 + sequence,
            None,
        )
        .expect("debug envelope");
    let bytes = encode_envelope(&envelope, WireLimits::default()).expect("encode debug");
    session.submit_envelope(&bytes).expect("submit debug");
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
                    "@SYSTEM_TITLE\nRESULT = GCREATE(1, 2, 2)\nRESULT = GSETCOLOR(1, 4294967295, 0, -1)\nRESULT:1 = BITMAP_CACHE_ENABLE(1)\nRESULTS:40 = %HTML_TOPLAINTEXT(\"a&nbsp;b\")%\nRESULT:41 = GCREATE(7, 2, 2)\nRESULT:42 = GSETBRUSH(7, 4294901760)\nRESULT:43 = GGETBRUSH(7)\nRESULT:44 = GSETPEN(7, 4278255360, 2)\nRESULT:45 = GGETPEN(7)\nRESULT:46 = GGETPENWIDTH(7)\nRESULT:47 = GFILLRECTANGLE(7, 0, 0, 2, 2)\nRESULT:48 = GDRAWLINE(7, 0, 0, 1, 1)\nRESULT:49 = GDISPOSE(7)\nRESULT:50 = CBGCLEAR()\nRESULT:2 = MOVETEXTBOX(10, 20, 30)\nWAIT\nRETURN\n"
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
fn html_pop_matches_the_reference_fixture_and_writes_the_string_result() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "html-pop-oracle".into(),
            features: Vec::new(),
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
                relative_path: "html-pop.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nPRINTPLAIN A<&\nPRINTBUTTON \"choose\", 42\nRESULTS:30 = %HTML_POPPRINTINGSTR()%\nPRINT [0x10] hex [1e2] exponent\nRESULTS:31 = %HTML_POPPRINTINGSTR()%\nPRINT_IMG \"missing\", \"hover\", \"mask\", 20, 10 px, 7 px\nRESULTS:32 = %HTML_POPPRINTINGSTR()%\nPRINT_RECT 1 px, 2, 3 px, 4\nRESULTS:33 = %HTML_POPPRINTINGSTR()%\nPRINT_SPACE 5 px\nRESULTS:34 = %HTML_POPPRINTINGSTR()%\nWAIT\nRETURN\n".into(),
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
    for _ in 0..24 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    let vm = session.vm.as_ref().unwrap();
    let results = runtime_variable_key(vm, "RESULTS").unwrap();
    let values = vm
        .read_runtime_state(
            &(30..=34)
                .map(|index| erabasic_vm::VmRuntimeRead {
                    variable: results,
                    indices: vec![index],
                    character: None,
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    assert_eq!(
        values,
        [
            VmValue::String("A&lt;&amp;<button value='42'>choose</button>".into()),
            VmValue::String(
                "<button value='16'>[0x10] hex </button><button value='100'>[1e2] exponent</button>".into()
            ),
            VmValue::String(
                "<img src='missing' srcb='hover' srcm='mask' height='10px' width='3' ypos='7px'>".into()
            ),
            VmValue::String("<shape type='rect' param='1px, 2, 3px, 4'>".into()),
            VmValue::String("<shape type='space' param='5px'>".into()),
        ]
    );
}

#[test]
fn safe_at_commands_use_runtime_lifecycle_effects_and_keep_debug_separate() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::WaitingInput;
    session.epoch = SessionEpoch(1);
    session.handle_system_input_command(1, "@CONFIG").unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::EffectBatch(batch)
            if matches!(batch.effects[0].kind, EffectKind::OpenConfiguration)
    )));
    session.handle_system_input_command(2, "@DEBUG").unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::Diagnostic(diagnostic)
            if diagnostic.code == "runtime.debug_command_requires_debug_channel"
    )));
    session.handle_system_input_command(3, "@REBOOT").unwrap();
    assert_eq!(
        session.exit_requested.map(|exit| exit.reason),
        Some(ExitReason::Restart)
    );
    assert_eq!(session.phase, RuntimePhase::Stopping);
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
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    // RESETDATA removes every character in the reference runtime, so a standalone
    // SYSTEM_TITLE fixture must explicitly create the character used by training.
    let source = "@SYSTEM_TITLE\nRESETDATA\nADDVOIDCHARA\nBEGIN TRAIN\n@EVENTTRAIN\nRETURN\n@SHOW_STATUS\nPRINT 抑鬱\nRETURN\n@COM_ABLE0\nRESULT = 1\nRETURN\n@SHOW_USERCOM\nPRINT ▼[－][Look]----------\nRETURN\n@EVENTCOM\nRETURN\n@COM0\nFLAG:0 += 1\nRESULT = 1\nRETURN\n@SOURCE_CHECK\nRETURN\n@EVENTCOMEND\nRETURN\n";
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
                        DisplayRun::Text { text, .. } => output.push_str(text),
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

#[test]
fn project_load_start_and_print_cross_the_message_boundary() {
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
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);

    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nSKIPDISP 1\nSKIPDISP 0\nPRINTFORML TITLE_CHARANUM={CHARANUM}\nPRINTL ORACLE_READY\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "CHARA0.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8("番号,0\n名前,initial\n".into()),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    let loaded = drain(&mut session);
    assert!(loaded.iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectLoadReport(report) if report.success
    )));
    assert_eq!(session.phase(), RuntimePhase::Ready);

    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let initial = session
        .drive(RuntimeDriveBudget {
            maximum_vm_instructions: 100_000,
            maximum_runtime_transitions: 2,
        })
        .expect("start");
    assert_eq!(initial.runtime_transitions, 2);
    let mut output = drain(&mut session);
    let yielded = session
        .drive(RuntimeDriveBudget {
            maximum_vm_instructions: 100_000,
            maximum_runtime_transitions: 1,
        })
        .expect("bounded ready host call");
    assert_eq!(yielded.state, RuntimeDriveState::MoreWork);
    let report = session.drive(RuntimeDriveBudget::default()).expect("run");
    assert!(
        report.runtime_transitions >= 3,
        "ready host calls should be batched in one bounded runtime drive: {report:?}"
    );
    assert_eq!(session.random_seed(), Some(1));
    output.extend(drain(&mut session));
    assert!(output.iter().any(|message| matches!(
        message,
        RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
    )));
    let snapshot = session.presentation.snapshot();
    assert!(snapshot.history.logical_lines.iter().any(|line| {
        line.runs.iter().any(|run| {
            matches!(
                run,
                era_runtime_protocol::DisplayRun::Text { text, .. } if text.contains("ORACLE_READY")
            )
        })
    }));
    assert!(snapshot.history.logical_lines.iter().any(|line| {
        line.runs.iter().any(|run| {
            matches!(
                run,
                era_runtime_protocol::DisplayRun::Text { text, .. }
                    if text.contains("TITLE_CHARANUM=0")
            )
        })
    }));
}

#[test]
fn linecount_drives_clearline_and_bounded_padding_loops() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "linecount-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nCALL ORACLE_LINECOUNT\nWAIT\nRETURN\n".into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "linecount.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        include_str!(
                            "../../../../reference/emuera.em/emuera-reference-cli/tests/fixture/erb/linecount.erb"
                        )
                        .into(),
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
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..20 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(session.presentation.logical_line_count(), 3);
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(read_runtime_integer(vm, "RESULT", &[50], None).unwrap(), 2);
    assert_eq!(read_runtime_integer(vm, "RESULT", &[51], None).unwrap(), 1);
    assert_eq!(read_runtime_integer(vm, "RESULT", &[52], None).unwrap(), 3);
    let snapshot = session.presentation.snapshot();
    assert_eq!(snapshot.history.logical_lines.len(), 3);
    assert!(snapshot.history.logical_lines.iter().any(|line| {
        line.runs
            .iter()
            .any(|run| matches!(run, DisplayRun::Text { text, .. } if text == "one"))
    }));
}

#[test]
fn nested_begin_returns_current_frame_then_applies_the_deferred_flow() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "begin-test".into(),
            features: Vec::new(),
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
                relative_path: "begin.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nCALL GO\nFLAG:0 = 7\nRETURN\n@GO\nFONTBOLD\nSKIPDISP 1\nBEGIN FIRST\nFLAG:0 = 99\nRETURN\n@EVENTFIRST\nPRINTL entered\nWAIT\nRETURN\n"
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
            mode: StartMode::NewGame { seed: Some(9) },
        }),
    );
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "FLAG", &[0], None).unwrap(),
        7
    );
    assert!(!session.skip_print);
    let snapshot = session.presentation.snapshot();
    let run = snapshot
        .history
        .logical_lines
        .iter()
        .flat_map(|line| &line.runs)
        .find(|run| matches!(run, DisplayRun::Text { text, .. } if text == "entered"))
        .expect("EVENTFIRST output");
    assert!(matches!(run, DisplayRun::Text { style, .. } if !style.bold));
}

#[test]
#[allow(clippy::too_many_lines)]
fn builtin_title_precedes_reset_data_and_initial_character_insertion() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "title-test".into(),
            features: Vec::new(),
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
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@EVENTFIRST\nWAIT\nRETURN\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "GAMEBASE.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8(
                        "バージョン,1001\nタイトル,Demo\n作者,Author\n製作年,2024\n追加情報,Info\n"
                            .into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "CHARA0.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8("番号,0\n名前,Initial\n".into()),
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
            mode: StartMode::NewGame { seed: Some(11) },
        }),
    );
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "CHARANUM", &[], None).unwrap(),
        0
    );
    assert_eq!(
        session
            .vm
            .as_ref()
            .unwrap()
            .vm()
            .artifact()
            .project_data
            .new_game_seed()
            .initial_characters
            .len(),
        1
    );
    let snapshot = session.presentation.snapshot();
    assert!(snapshot.history.logical_lines.iter().any(|line| {
        line.alignment == LineAlignment::Center
            && line
                .runs
                .iter()
                .any(|run| matches!(run, DisplayRun::Text { text, .. } if text == "Demo"))
    }));
    assert!(snapshot.history.logical_lines.iter().any(|line| {
        line.runs.iter().any(|run| {
            matches!(run, DisplayRun::Button { runs, .. }
            if matches!(&runs[0], DisplayRun::Text { text, .. } if text.starts_with("[0]")))
        })
    }));
    let pending = session.operations.active_input().unwrap();
    let wait_id = pending.wait.wait_id;
    let submission_token = pending.wait.submission_token;
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token: submission_token,
            intent: InputIntent::CommitText("0".into()),
            monotonic_time_ns: 0,
            message_skip: false,
        }),
    );
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
    }
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "CHARANUM", &[], None).unwrap(),
        1
    );
}

#[test]
fn runtime_metadata_queries_use_the_active_artifact_and_fiber() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "metadata-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
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
                    relative_path: "metadata.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\n#DIMS VALUES, 3, 4\n#DIMS CHOICES, 5\nVARSIZE VALUES\nPRINTFORML statement={RESULT},{RESULT:1}\nCALL SIZE_OF, CHOICES\nPRINTFORML meta={VARSIZE(\"VALUES\")},{EXISTFUNCTION(\"SYSTEM_TITLE\")},{EXISTVAR(\"VALUES\")},%GETDOINGFUNCTION()%,{RESULT},%CHOICES:2%\nPRINTFORML funcs={ENUMFUNCWITH(\"SIZE\", CHOICES)},%CHOICES:0%\nPRINTFORML vars={ENUMVARWITH(\"SAVEDATA_TEXT\", CHOICES)},%CHOICES:0%\nCALL ORACLE_REFLECTION\nPRINTFORML reflection={RESULT:12},{RESULT:13},%RESULTS:8%,%RESULTS:9%\nRETURN\n@SIZE_OF(refChoices)\n#DIMS REF refChoices, 0\nrefChoices:2 '= \"bound\"\nRESULT = VARSIZE(\"refChoices\")\nRETURN\n@ORACLE_REFLECTION\n#DIMS NAMES, 4\nRESULT:12 = ENUMFUNCWITH(\"ORACLE_REFLECTION\", NAMES)\nRESULTS:8 = %NAMES:0%\nRESULT:13 = ENUMVARWITH(\"SAVEDATA_TEXT\", NAMES)\nRESULTS:9 = %NAMES:0%\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                }],
            }),
        );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let loaded = drain(&mut session);
    assert!(
        loaded.iter().any(|message| {
            matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
        }),
        "{loaded:#?}"
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..24 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
    }
    let output = drain(&mut session);
    let snapshot = session.presentation.snapshot();
    assert!(
        snapshot.history.logical_lines.iter().any(|line| {
            line.runs.iter().any(|run| {
                matches!(
                    run,
                    era_runtime_protocol::DisplayRun::Text { text, .. }
                        if text.contains("meta=3,1,0,SYSTEM_TITLE,5,bound")
                )
            })
        }),
        "{output:#?}"
    );
    let rendered = snapshot
        .history
        .logical_lines
        .iter()
        .flat_map(|line| line.runs.iter())
        .filter_map(|run| match run {
            era_runtime_protocol::DisplayRun::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(
        rendered.contains("statement=3,4"),
        "{rendered}\n{output:#?}"
    );
    assert!(rendered.contains("funcs=1,SIZE_OF"), "{rendered}");
    assert!(rendered.contains("vars=1,SAVEDATA_TEXT"), "{rendered}");
    assert!(
        rendered.contains("reflection=1,1,ORACLE_REFLECTION,SAVEDATA_TEXT"),
        "{rendered}\n{output:#?}"
    );
}

#[test]
fn reference_presentation_fixture_preserves_logical_intent() {
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
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);

    // Keep this body identical to ORACLE_PRESENTATION in the C# fixture so
    // the oracle and Rust tests exercise the same EraBasic commands.
    let source = "@SYSTEM_TITLE\nPRINTBUTTON \"A\", 1\nPRINTBUTTONC \"B\", 2\nPRINTBUTTONLC \"C\", 3\nPRINTL\nDRAWLINE\nNOSKIP\nPRINTL VISIBLE\nENDNOSKIP\nRETURN\n";
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(source.into()),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).expect("run");
        if session.phase() == RuntimePhase::Ready {
            break;
        }
    }
    drain(&mut session);
    let snapshot = session.presentation.snapshot();

    assert!(snapshot.history.logical_lines.iter().any(|line| {
        line.runs.iter().any(|run| {
            matches!(
                run,
                era_runtime_protocol::DisplayRun::Button { runs, .. }
                    if runs.iter().any(|run| matches!(
                        run,
                        era_runtime_protocol::DisplayRun::Text { text, .. } if text == "A"
                    ))
            )
        })
    }));
    assert_eq!(
        snapshot
            .history
            .logical_lines
            .iter()
            .flat_map(|line| &line.runs)
            .filter(|run| matches!(run, era_runtime_protocol::DisplayRun::ColumnCell { .. }))
            .count(),
        2
    );
    assert!(snapshot.history.logical_lines.iter().any(|line| {
        line.runs
            .iter()
            .any(|run| matches!(run, era_runtime_protocol::DisplayRun::Separator { .. }))
    }));
    assert!(snapshot.history.logical_lines.iter().any(|line| {
        line.runs.iter().any(|run| {
            matches!(
                run,
                era_runtime_protocol::DisplayRun::Text { text, .. } if text == "VISIBLE"
            )
        })
    }));
}

fn flattened_display_text(runs: &[DisplayRun]) -> String {
    runs.iter()
        .map(|run| match run {
            DisplayRun::Text { text, .. } => text.clone(),
            DisplayRun::Button { runs, .. } | DisplayRun::ColumnCell { content: runs, .. } => {
                flattened_display_text(runs)
            }
            _ => String::new(),
        })
        .collect()
}

#[test]
#[allow(clippy::too_many_lines)]
fn printform_and_printc_family_preserve_reference_semantics() {
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
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);

    let source = "@SYSTEM_TITLE\nCALL ORACLE_PRINT_FAMILY\nWAIT\nRETURN\n";
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
                    relative_path: "print-family.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        include_str!(
                            "../../../../reference/emuera.em/emuera-reference-cli/tests/fixture/erb/print-family.erb"
                        )
                        .into(),
                    ),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    let load = drain(&mut session);
    assert!(
        load.iter().any(|message| matches!(
            message,
            RuntimeMessage::ProjectLoadReport(report) if report.success
        )),
        "{load:#?}"
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let mut run_messages = Vec::new();
    for _ in 0..32 {
        session.drive(RuntimeDriveBudget::default()).expect("run");
        run_messages.extend(drain(&mut session));
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(
        session.phase(),
        RuntimePhase::WaitingInput,
        "{run_messages:#?}"
    );
    let snapshot = session.presentation.snapshot();

    let rendered = snapshot
        .history
        .logical_lines
        .iter()
        .map(|line| flattened_display_text(&line.runs))
        .collect::<Vec<_>>();
    assert!(
        rendered.contains(&"|  7|7  |界  |Target|Call|Call|Target|Call| X".into()),
        "{rendered:#?}"
    );
    assert!(rendered.contains(&"ヒラガナ".into()), "{rendered:#?}");

    let cell_line = snapshot
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
        .expect("four script PRINTC cells must remain on one line");
    let cells = cell_line
        .runs
        .iter()
        .filter_map(|run| match run {
            DisplayRun::ColumnCell {
                content,
                alignment,
                preferred_columns,
            } => Some((content, alignment, preferred_columns)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(cells.len(), 4);
    assert_eq!(*cells[0].1, era_runtime_protocol::CellAlignment::Right);
    assert_eq!(*cells[1].1, era_runtime_protocol::CellAlignment::Left);
    assert!(cells.iter().all(|cell| *cell.2 == 25));
    assert!(
        cells
            .iter()
            .all(|cell| matches!(cell.0.as_slice(), [DisplayRun::Button { .. }]))
    );
    let DisplayRun::Button { runs, .. } = &cells[0].0[0] else {
        unreachable!()
    };
    let DisplayRun::Text { style, .. } = &runs[0] else {
        unreachable!()
    };
    assert_eq!(style.foreground.red, 0xc0);
    assert_eq!(session.command_intents.len(), 4);
}

#[test]
fn typed_input_updates_result_and_sixth_argument_honors_message_skip() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: vec![RuntimeFeature::TimedInput],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);
    submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                project_revision: 1,
                files: vec![SubmittedFile {
                    relative_path: "input.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nTINPUT 1000, 7, 1, \"timeout\", 0, 0\nTINPUT 1000, 9, 1, \"timeout\", 0, 0\nPRINTFORML got={RESULT}\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                }],
            }),
        );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..4 {
        session.drive(RuntimeDriveBudget::default()).expect("wait");
    }
    let opened = drain(&mut session);
    let wait = opened
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::WaitChanged(WaitChange::Opened(wait)) => Some(wait.clone()),
            _ => None,
        })
        .expect("runtime should publish the input wait");
    assert_eq!(
        wait.default_value,
        Some(era_runtime_protocol::ProtocolValue::Integer(7))
    );
    assert_eq!(wait.stability, WaitStability::Transient);

    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id: wait.wait_id,
            token: wait.submission_token,
            monotonic_time_ns: 10,
            intent: InputIntent::CommitText("42".into()),
            message_skip: true,
        }),
    );
    for _ in 0..4 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("resume");
    }
    drain(&mut session);
    let snapshot = session.presentation.snapshot();
    assert!(snapshot.history.logical_lines.iter().any(|line| {
        line.runs.iter().any(|run| {
            matches!(
                run,
                era_runtime_protocol::DisplayRun::Text { text, .. } if text.contains("got=9")
            )
        })
    }));
}

#[test]
fn untimed_one_input_message_skip_keeps_the_complete_default() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: vec![RuntimeFeature::TimedInput],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "input.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nINPUT\nONEINPUTS LONG, 0, 0\nPRINTFORML got=%RESULTS%\nWAIT\nRETURN\n"
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
    let mut started = Vec::new();
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).expect("wait");
        started.extend(drain(&mut session));
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let wait = session
        .operations
        .active_input()
        .unwrap_or_else(|| {
            panic!(
                "input wait was not opened in state {:?}: {started:?}",
                session.state
            )
        })
        .wait
        .clone();
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id: wait.wait_id,
            token: wait.submission_token,
            monotonic_time_ns: 10,
            intent: InputIntent::CommitText("1".into()),
            message_skip: true,
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("resume");
    }
    drain(&mut session);
    let snapshot = session.presentation.snapshot();
    assert!(snapshot.history.logical_lines.iter().any(|line| {
        line.runs.iter().any(|run| {
            matches!(
                run,
                era_runtime_protocol::DisplayRun::Text { text, .. } if text.contains("got=LONG")
            )
        })
    }));
}

#[test]
#[allow(clippy::too_many_lines)]
fn restart_redraws_string_and_integer_button_menus_in_the_current_function() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "restart-menu-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);
    let source = concat!(
        "@SYSTEM_TITLE\nCALL ORACLE_RESTART_FLOW\nWAIT\nRETURN\n",
        include_str!(
            "../../../../reference/emuera.em/emuera-reference-cli/tests/fixture/erb/restart.erb"
        )
    );
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "restart.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(source.into()),
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
        session
            .drive(RuntimeDriveBudget::default())
            .expect("move wait");
        if session.operations.active_input().is_some() {
            break;
        }
    }

    let (wait_id, submission_token, c_button) = {
        let pending = session.operations.active_input().expect("move menu wait");
        let button = pending
            .choices
            .iter()
            .find_map(|(token, value)| (*value == VmValue::String("C".into())).then_some(*token))
            .expect("C button");
        (pending.wait.wait_id, pending.wait.submission_token, button)
    };
    assert!(
        session
            .presentation
            .snapshot()
            .history
            .logical_lines
            .last()
            .is_some_and(|line| line.line_end),
        "INPUTS must flush the button row before opening its wait"
    );
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token: submission_token,
            monotonic_time_ns: 0,
            intent: InputIntent::Activate(c_button),
            message_skip: false,
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("restart move menu");
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let (wait_id, submission_token, zero_button) = {
        let pending = session
            .operations
            .active_input()
            .expect("restarted move menu wait");
        assert_eq!(pending.wait.kind, WaitKind::StringValue);
        assert!(
            pending
                .choices
                .values()
                .any(|value| *value == VmValue::String("C".into()))
        );
        let button = pending
            .choices
            .iter()
            .find_map(|(token, value)| (*value == VmValue::String("0".into())).then_some(*token))
            .expect("move return button");
        (pending.wait.wait_id, pending.wait.submission_token, button)
    };
    assert!(
        session
            .presentation
            .snapshot()
            .history
            .logical_lines
            .last()
            .is_some_and(|line| line.line_end),
        "restarted INPUTS must not reuse the previous menu row"
    );
    submit(
        &mut session,
        4,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token: submission_token,
            monotonic_time_ns: 0,
            intent: InputIntent::Activate(zero_button),
            message_skip: false,
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("ability wait");
        if session
            .operations
            .active_input()
            .is_some_and(|pending| pending.wait.kind == WaitKind::IntegerValue)
        {
            break;
        }
    }

    let (wait_id, submission_token, next_page_button) = {
        let pending = session
            .operations
            .active_input()
            .expect("ability menu wait");
        let button = pending
            .choices
            .iter()
            .find_map(|(token, value)| (*value == VmValue::Integer(6)).then_some(*token))
            .expect("next page button");
        (pending.wait.wait_id, pending.wait.submission_token, button)
    };
    submit(
        &mut session,
        5,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token: submission_token,
            monotonic_time_ns: 0,
            intent: InputIntent::Activate(next_page_button),
            message_skip: false,
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("restart ability menu");
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let pending = session
        .operations
        .active_input()
        .expect("restarted ability menu wait");
    assert_eq!(pending.wait.kind, WaitKind::IntegerValue);
    assert!(
        pending
            .choices
            .values()
            .any(|value| *value == VmValue::Integer(6))
    );

    let snapshot = session.presentation.snapshot();
    let visible_text = snapshot
        .history
        .logical_lines
        .iter()
        .flat_map(|line| &line.runs)
        .filter_map(|run| match run {
            DisplayRun::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(visible_text.contains("move display=1"), "{visible_text}");
    assert!(visible_text.contains("ability page=1"), "{visible_text}");
    assert!(!visible_text.contains("invalid move"), "{visible_text}");
    assert!(!visible_text.contains("invalid ability"), "{visible_text}");
}

#[test]
fn inputs_accepts_an_automatic_button_from_the_pending_print_buffer() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "pending-auto-button-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "pending-button.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    concat!(
                        "@SYSTEM_TITLE\nCALL ORACLE_PENDING_AUTO_BUTTON\nWAIT\nRETURN\n",
                        include_str!(
                            "../../../../reference/emuera.em/emuera-reference-cli/tests/fixture/erb/restart.erb"
                        )
                    )
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
        session
            .drive(RuntimeDriveBudget::default())
            .expect("input wait");
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let (wait_id, submission_token, back_button) = {
        let pending = session.operations.active_input().expect("INPUTS wait");
        assert_eq!(pending.wait.kind, WaitKind::StringValue);
        let token = pending
            .choices
            .iter()
            .find_map(|(token, value)| (*value == VmValue::Integer(58)).then_some(*token))
            .expect("pending automatic button must belong to the active wait");
        (pending.wait.wait_id, pending.wait.submission_token, token)
    };
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token: submission_token,
            monotonic_time_ns: 0,
            intent: InputIntent::Activate(back_button),
            message_skip: false,
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("accept back button");
    }
    assert!(session.presentation.snapshot().history.logical_lines.iter().any(
        |line| line.runs.iter().any(
            |run| matches!(run, era_runtime_protocol::DisplayRun::Text { text, .. } if text.contains("pending auto=58"))
        )
    ));
}

#[test]
#[allow(clippy::too_many_lines)]
fn input_undo_records_only_accepted_scalar_input_after_a_checkpoint() {
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
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "emuera.config".into(),
                    category: FileCategory::Configuration,
                    payload: FilePayload::Utf8("Enable undo with ctrl-z:YES\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "input.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nINPUT\nWAIT\nRETURN\n@SHOW_SHOP\nWAIT\nRETURN\n".into(),
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
        .establish_input_undo_checkpoint(3, baseline, random)
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
    assert_eq!(session.undo_checkpoint.as_ref().unwrap().inputs, vec!["42"]);
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
    assert!(session.undo_checkpoint.as_ref().unwrap().inputs.is_empty());
    assert_eq!(session.input_undo_state().available_steps, 0);
}

#[test]
fn input_undo_keeps_the_next_scalar_queued_across_primitive_waits() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.undo_replay = Some(UndoReplay {
        remaining: VecDeque::from(["12".to_owned()]),
        queued_repeats: 0,
    });
    let mut wait = session.system_wait(InteractionToken { epoch: 0, id: 1 });
    wait.kind = WaitKind::PrimitiveMouseKey;
    assert_eq!(session.replay_submission(&wait), None);
    assert_eq!(
        session.undo_replay.as_ref().unwrap().remaining,
        VecDeque::from(["12".to_owned()])
    );

    wait.kind = WaitKind::IntegerValue;
    assert_eq!(
        session.replay_submission(&wait),
        Some(InputSubmission::Value(VmValue::Integer(12)))
    );
    assert!(session.undo_replay.as_ref().unwrap().remaining.is_empty());
}

#[test]
fn autosave_failure_prints_both_reference_messages_and_waits_before_shop() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.selected_locale = "en".into();
    session.stage_builtin_autosave_failure().unwrap();
    assert_eq!(session.controller.step, SystemStep::ShopAutosaveFailureWait);
    assert_eq!(session.phase, RuntimePhase::WaitingInput);
    assert_eq!(
        session.operations.active_input().unwrap().wait.kind,
        WaitKind::EnterKey
    );
    let keys = session
        .presentation
        .snapshot()
        .history
        .logical_lines
        .into_iter()
        .flat_map(|line| line.runs)
        .filter_map(|run| match run {
            era_runtime_protocol::DisplayRun::Text {
                system_text: Some(reference),
                ..
            } => Some(reference.key),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            SystemTextKey::AutoSaveFailed,
            SystemTextKey::AutoSaveSkipped
        ]
    );
}

#[test]
fn stopcalltrain_discards_its_caller_and_resumes_the_train_system_phase() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "continuous-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nBEGIN TRAIN\n@EVENTTRAIN\nRETURN\n@SHOW_STATUS\nRETURN\n@COM_ABLE0\nRESULT = 1\nRETURN RESULT\n@SHOW_USERCOM\n#DIM ONCE\nIF ONCE == 0\nSELECTCOM:1 = 0\nONCE = 1\nCALLTRAIN 1\nENDIF\nRETURN\n@COM0\nSTOPCALLTRAIN\nRESULT:30 = 1\nRETURN\n@SOURCE_CHECK\nRETURN\n@CALLTRAINEND\nRESULT:31 = 1\nRETURN\n"
                            .into(),
                    ),
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
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..64 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let output = drain(&mut session);
    assert_ne!(session.phase, RuntimePhase::Faulted, "{output:?}");
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[30], None).unwrap(),
        0,
        "the STOPCALLTRAIN caller must not resume"
    );
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[31], None).unwrap(),
        1
    );
    assert_eq!(session.controller.step, SystemStep::TrainEventComEndWait);
    assert_eq!(
        session.operations.active_input().unwrap().wait.kind,
        WaitKind::EnterKey
    );
}

#[test]
fn continuous_train_reports_progress_and_routes_unavailable_commands_to_usercom() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "continuous-output-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let source = "@SYSTEM_TITLE\nBEGIN TRAIN\n@EVENTTRAIN\nRETURN\n@SHOW_STATUS\nRETURN\n@COM_ABLE0\nRESULT = 1\nRETURN\n@SHOW_USERCOM\n#DIM ONCE\nIF ONCE == 0\nSELECTCOM:1 = 1\nCALLTRAIN 1\nONCE = 1\nENDIF\nRETURN\n@USERCOM\nRETURN\n@CALLTRAINEND\nRETURN\n";
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
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..64 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let output = drain(&mut session);
    assert_ne!(session.phase, RuntimePhase::Faulted, "{output:?}");
    let keys = session
        .presentation
        .snapshot()
        .history
        .logical_lines
        .into_iter()
        .flat_map(|line| line.runs)
        .filter_map(|run| match run {
            era_runtime_protocol::DisplayRun::Text {
                system_text: Some(reference),
                ..
            } => Some(reference.key),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(keys.contains(&SystemTextKey::ContinuousTrainProgress));
    assert!(keys.contains(&SystemTextKey::ContinuousTrainCommandFailed));
    assert!(!session.controller.continuous_train);
}

#[test]
#[allow(clippy::too_many_lines)]
fn traditional_save_export_and_restore_are_atomic_runtime_operations() {
    fn prepare() -> RuntimeSession {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "save-test".into(),
                features: vec![RuntimeFeature::TraditionalSave, RuntimeFeature::VmSnapshot],
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["en-US".into()],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        submit(
                &mut session,
                1,
                RuntimeMessage::ProjectManifest(ProjectManifest {
                    project_revision: 1,
                    files: vec![
                        SubmittedFile {
                            relative_path: "variables.erh".into(),
                            category: FileCategory::Erh,
                            payload: FilePayload::Utf8("#DIM SAVEDATA ZZZSAVE\n".into()),
                            content_hash: None,
                        },
                        SubmittedFile {
                            relative_path: "save.erb".into(),
                            category: FileCategory::Erb,
                            payload: FilePayload::Utf8(
                                "@SYSTEM_TITLE\nINPUT\nZZZSAVE = RESULT\nINPUT\nRETURN\n@SYSTEM_LOADEND\nPRINTFORML loadend={ZZZSAVE}\nRETURN\n@EVENTLOAD\nPRINTL eventload\nRETURN\n@SHOW_SHOP\nPRINTL shop\nWAIT\nRETURN\n@SAVEINFO\nPRINTL unexpected-autosave\nRETURN\n"
                                    .into(),
                            ),
                            content_hash: None,
                        },
                    ],
                }),
            );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let load_messages = drain(&mut session);
        assert_eq!(session.phase(), RuntimePhase::Ready, "{load_messages:#?}");
        session
    }

    let mut source = prepare();
    submit(
        &mut source,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..4 {
        source.drive(RuntimeDriveBudget::default()).unwrap();
    }
    let wait = drain(&mut source)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::WaitChanged(WaitChange::Opened(wait)) => Some(wait),
            _ => None,
        })
        .expect("first INPUT wait");
    submit(
        &mut source,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id: wait.wait_id,
            token: wait.submission_token,
            monotonic_time_ns: 1,
            intent: InputIntent::CommitText("37".into()),
            message_skip: false,
        }),
    );
    for _ in 0..4 {
        source.drive(RuntimeDriveBudget::default()).unwrap();
    }
    drain(&mut source);
    assert_eq!(source.phase(), RuntimePhase::WaitingInput);
    submit(
        &mut source,
        4,
        RuntimeMessage::StateExportRequest(StateExportRequest {
            kind: StateExportKind::TraditionalSave,
        }),
    );
    source.drive(RuntimeDriveBudget::default()).unwrap();
    let descriptor = drain(&mut source)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateExportReady(StateExportReady {
                result: StateExportResult::Ready { transfer },
                ..
            }) => Some(transfer),
            _ => None,
        })
        .expect("traditional save descriptor");
    submit(
        &mut source,
        5,
        RuntimeMessage::StateExportChunkRequest(StateExportChunkRequest {
            transfer_id: descriptor.transfer_id,
            offset: 0,
            maximum_bytes: u32::MAX,
        }),
    );
    source.drive(RuntimeDriveBudget::default()).unwrap();
    let bytes = drain(&mut source)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateExportChunk(chunk) => Some(chunk.data.as_slice().to_vec()),
            _ => None,
        })
        .expect("traditional save bytes");

    let mut restored = prepare();
    submit(
        &mut restored,
        2,
        RuntimeMessage::StateImportBegin(StateImportBegin {
            kind: StateExportKind::TraditionalSave,
            total_bytes: u64::try_from(bytes.len()).unwrap(),
            digest: descriptor.digest,
            artifact_id: None,
        }),
    );
    restored.drive(RuntimeDriveBudget::default()).unwrap();
    let transfer_id = drain(&mut restored)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateImportAccepted(accepted) => Some(accepted.transfer_id),
            _ => None,
        })
        .expect("accepted import");
    submit(
        &mut restored,
        3,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 0,
            data: ProtocolBytes::new(bytes),
        }),
    );
    submit(
        &mut restored,
        4,
        RuntimeMessage::StateImportCommit(StateImportCommit { transfer_id }),
    );
    restored.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut restored);
    submit(
        &mut restored,
        5,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::TraditionalSave { transfer_id },
        }),
    );
    for _ in 0..5 {
        restored.drive(RuntimeDriveBudget::default()).unwrap();
    }
    drain(&mut restored);
    let snapshot = restored.presentation.snapshot();
    let display = snapshot
        .history
        .logical_lines
        .iter()
        .flat_map(|line| &line.runs)
        .filter_map(|run| match run {
            era_runtime_protocol::DisplayRun::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("|");
    let loadend = display.find("loadend=37").expect("SYSTEM_LOADEND output");
    let eventload = display.find("eventload").expect("EVENTLOAD output");
    let shop = display.find("shop").expect("SHOW_SHOP output");
    assert!(loadend < eventload && eventload < shop, "{display}");
    assert!(!display.contains("unexpected-autosave"), "{display}");

    let old_wait = source
        .operations
        .active_input()
        .expect("snapshot wait")
        .wait
        .clone();
    submit(
        &mut source,
        6,
        RuntimeMessage::StateExportRequest(StateExportRequest {
            kind: StateExportKind::VmSnapshot,
        }),
    );
    source.drive(RuntimeDriveBudget::default()).unwrap();
    let snapshot_descriptor = drain(&mut source)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateExportReady(StateExportReady {
                result: StateExportResult::Ready { transfer },
                ..
            }) => Some(transfer),
            _ => None,
        })
        .expect("runtime snapshot descriptor");
    let mut snapshot_bytes = Vec::new();
    let mut source_sequence = 7;
    loop {
        submit(
            &mut source,
            source_sequence,
            RuntimeMessage::StateExportChunkRequest(StateExportChunkRequest {
                transfer_id: snapshot_descriptor.transfer_id,
                offset: u64::try_from(snapshot_bytes.len()).unwrap(),
                maximum_bytes: 1024 * 1024,
            }),
        );
        source_sequence += 1;
        source.drive(RuntimeDriveBudget::default()).unwrap();
        let chunk = drain(&mut source)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::StateExportChunk(chunk) => Some(chunk),
                _ => None,
            })
            .expect("runtime snapshot chunk");
        snapshot_bytes.extend_from_slice(chunk.data.as_slice());
        if chunk.complete {
            break;
        }
    }

    let mut exact = prepare();
    submit(
        &mut exact,
        2,
        RuntimeMessage::StateImportBegin(StateImportBegin {
            kind: StateExportKind::VmSnapshot,
            total_bytes: u64::try_from(snapshot_bytes.len()).unwrap(),
            digest: snapshot_descriptor.digest,
            artifact_id: snapshot_descriptor.artifact_id,
        }),
    );
    exact.drive(RuntimeDriveBudget::default()).unwrap();
    let transfer_id = drain(&mut exact)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateImportAccepted(accepted) => Some(accepted.transfer_id),
            _ => None,
        })
        .unwrap();
    let mut exact_sequence = 3;
    for (index, chunk) in snapshot_bytes.chunks(1024 * 1024).enumerate() {
        submit(
            &mut exact,
            exact_sequence,
            RuntimeMessage::StateImportChunk(StateImportChunk {
                transfer_id,
                offset: u64::try_from(index * 1024 * 1024).unwrap(),
                data: ProtocolBytes::new(chunk.to_vec()),
            }),
        );
        exact_sequence += 1;
    }
    submit(
        &mut exact,
        exact_sequence,
        RuntimeMessage::StateImportCommit(StateImportCommit { transfer_id }),
    );
    exact_sequence += 1;
    exact.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut exact);
    submit(
        &mut exact,
        exact_sequence,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::VmSnapshot { transfer_id },
        }),
    );
    exact.drive(RuntimeDriveBudget::default()).unwrap();
    let restored_wait = exact.operations.active_input().expect("restored wait");
    assert_eq!(exact.phase(), RuntimePhase::WaitingInput);
    assert_ne!(restored_wait.wait.wait_id, old_wait.wait_id);
    assert_ne!(
        restored_wait.wait.submission_token,
        old_wait.submission_token
    );
    assert_eq!(restored_wait.wait.submission_token.epoch, exact.epoch.0);
}

#[test]
fn empty_storage_listing_opens_a_fixed_runtime_tokenized_page() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::WaitingExternal;
    session.epoch = SessionEpoch(1);
    session.selected_locale = "en".into();
    session.storage_capabilities = StorageCapabilities {
        revisions: true,
        atomic_replace: true,
        missing_precondition: true,
        delete: true,
    };
    session
        .operations
        .insert_storage(7, PendingStorage::ListLoadSlots);
    session
        .complete_storage(
            10,
            StorageResponse {
                request_id: 7,
                result: StorageResult::Listed {
                    entries: Vec::new(),
                },
            },
        )
        .unwrap();
    assert_eq!(
        session.load_slot_paths.first().map(String::as_str),
        Some("save00.sav")
    );
    assert_eq!(
        session.load_slot_paths.last().map(String::as_str),
        Some("save99.sav")
    );
    assert_eq!(session.load_slot_paths.len(), 21);
    assert!(session.occupied_slot_paths.is_empty());
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    let wait = session.operations.active_input().expect("system slot wait");
    assert!(wait.wait.system_input);
    assert!(
        wait.choices
            .keys()
            .all(|token| token.epoch == session.epoch.0)
    );
    assert!(
        session
            .presentation
            .snapshot()
            .history
            .logical_lines
            .iter()
            .any(|line| {
                line.runs.iter().any(|run| {
                    matches!(
                        run,
                        era_runtime_protocol::DisplayRun::Text {
                            system_text: Some(reference),
                            ..
                        } if reference.key == SystemTextKey::LoadQuestion
                    )
                })
            })
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn nested_savegame_cancel_resumes_the_suspended_vm_call() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "save-menu-test".into(),
            features: vec![RuntimeFeature::Storage, RuntimeFeature::VmSnapshot],
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
                relative_path: "menu.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SHOW_SHOP\nSAVEGAME\nRESULT = 7\nWAIT\nRETURN\n".into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let artifact = session.artifact.clone().expect("compiled menu fixture");
    let entry = artifact
        .artifact()
        .functions
        .iter()
        .find(|function| function.name == "SHOW_SHOP")
        .expect("SHOW_SHOP")
        .key;
    let code = artifact
        .artifact()
        .functions
        .iter()
        .find(|function| function.key == entry)
        .unwrap()
        .code
        .clone();
    let mut vm = RuntimeVm::new(artifact, VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    session.vm = Some(vm);
    session.controller.flow = Some(SystemFlow::Normal);
    session.phase = RuntimePhase::Running;

    let mut request = None;
    let mut observed = Vec::new();
    let mut reports = Vec::new();
    for _ in 0..4 {
        reports.push(session.drive(RuntimeDriveBudget::default()).unwrap());
        let messages = drain(&mut session);
        request = messages.iter().find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) => Some(request.clone()),
            _ => None,
        });
        observed.extend(messages);
        if request.is_some() {
            break;
        }
    }
    let request = request.unwrap_or_else(|| {
            panic!(
                "SAVEGAME list request; phase={:?}, code={code:#?}, reports={reports:#?}, output={observed:#?}",
                session.phase,
            )
        });
    assert!(matches!(request.operation, StorageOperation::List { .. }));
    session
        .complete_storage(
            2,
            StorageResponse {
                request_id: request.request_id,
                result: StorageResult::Listed {
                    entries: vec![
                        StorageEntry {
                            relative_path: "save01.sav".into(),
                            byte_length: 3,
                            revision: None,
                            change_token: Some("t1".into()),
                        },
                        StorageEntry {
                            relative_path: "save25.sav".into(),
                            byte_length: 3,
                            revision: None,
                            change_token: Some("t25".into()),
                        },
                    ],
                },
            },
        )
        .unwrap();
    let scan = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) => Some(request),
            _ => None,
        })
        .expect("slot metadata read");
    assert_eq!(scan.relative_path, "save01.sav");
    assert!(matches!(
        scan.operation,
        StorageOperation::ReadRange {
            offset: 0,
            maximum_bytes: 65_536,
            ..
        }
    ));
    session
        .complete_storage(
            3,
            StorageResponse {
                request_id: scan.request_id,
                result: StorageResult::ReadChunk {
                    data: ProtocolBytes::new(b"bad".to_vec()),
                    offset: 0,
                    complete: true,
                    change_token: "t1".into(),
                },
            },
        )
        .unwrap();
    assert!(session.invalid_slot_paths.contains("save01.sav"));
    assert!(session.occupied_slot_paths.contains("save25.sav"));
    assert!(
        drain(&mut session)
            .iter()
            .all(|message| !matches!(message, RuntimeMessage::StorageRequest(_)))
    );
    let pending = session
        .operations
        .take_active_input()
        .expect("save menu wait");
    assert!(pending.host_request.is_some());
    session.operations.restore_active_input(pending.clone());
    assert!(session.operations.is_snapshot_stable());
    assert!(session.vm.as_ref().unwrap().snapshot().is_ok());
    session
        .export_state(
            99,
            StateExportRequest {
                kind: StateExportKind::VmSnapshot,
            },
        )
        .unwrap();
    assert!(drain(&mut session).into_iter().any(|message| matches!(
        message,
        RuntimeMessage::StateExportReady(StateExportReady {
            result: StateExportResult::Ready { .. },
            ..
        })
    )));
    let pending = session.operations.take_active_input().unwrap();
    assert!(
        pending
            .choices
            .values()
            .any(|value| value == &VmValue::Integer(-1_001))
    );
    session
        .finish_system_input(pending, &VmValue::Integer(-1_001))
        .unwrap();
    let stat = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) => Some(request),
            _ => None,
        })
        .expect("slot delete stat");
    assert_eq!(stat.relative_path, "save01.sav");
    assert_eq!(stat.operation, StorageOperation::Stat);
    session
        .complete_storage(
            4,
            StorageResponse {
                request_id: stat.request_id,
                result: StorageResult::Metadata(era_runtime_protocol::StorageMetadata {
                    byte_length: 3,
                    revision: Some("r1".into()),
                }),
            },
        )
        .unwrap();
    let delete = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) => Some(request),
            _ => None,
        })
        .expect("revision-bound slot delete");
    assert!(matches!(
        delete.operation,
        StorageOperation::Delete {
            precondition: StoragePrecondition::Revision(ref revision),
        } if revision == "r1"
    ));
    session
        .complete_storage(
            5,
            StorageResponse {
                request_id: delete.request_id,
                result: StorageResult::Error {
                    error: era_runtime_protocol::FrontendIoError {
                        kind: FrontendIoErrorKind::Conflict,
                        message: "changed".into(),
                        platform_code: None,
                    },
                },
            },
        )
        .unwrap();
    assert!(session.operations.active_input().is_some());
    let pending = session.operations.take_active_input().unwrap();
    session
        .finish_system_input(pending, &VmValue::Integer(100))
        .unwrap();
    for _ in 0..4 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        7
    );
}

#[test]
fn project_title_can_open_loadgame() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "title-loadgame-test".into(),
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
                relative_path: "title.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nLOADGAME\nRETURN\n".into()),
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
    for _ in 0..4 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        messages.extend(drain(&mut session));
        if messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::StorageRequest(_)))
        {
            break;
        }
    }
    assert!(
        messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::StorageRequest(StorageRequest {
                operation: StorageOperation::List { .. },
                ..
            })
        )),
        "{messages:#?}"
    );
    assert_ne!(session.phase(), RuntimePhase::Faulted);
}

#[test]
fn vm_snapshot_export_accepts_a_runtime_owned_system_wait() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "successive-root-snapshot-test".into(),
            features: vec![RuntimeFeature::VmSnapshot],
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
                relative_path: "snapshot.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nBEGIN SHOP\n@EVENTSHOP\nRETURN\n@SHOW_SHOP\nRETURN\n".into(),
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
    for _ in 0..12 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert!(
        session
            .operations
            .active_input()
            .is_some_and(|input| input.wait.system_input && input.host_request.is_none())
    );
    submit(
        &mut session,
        3,
        RuntimeMessage::StateExportRequest(StateExportRequest {
            kind: StateExportKind::VmSnapshot,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert!(
        messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::StateExportReady(StateExportReady {
                result: StateExportResult::Ready { .. },
                ..
            })
        )),
        "{messages:#?}"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn savedata_uses_atomic_frontend_storage_and_resumes_only_after_completion() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "storage-test".into(),
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
                    relative_path: "save.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nPUTFORM suffix\nRESULT = SAVENOS()\nSAVEDATA 2, \"slot\"\nWAIT\nRETURN\n"
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
    let mut request = None;
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        for message in drain(&mut session) {
            if let RuntimeMessage::StorageRequest(value) = message {
                request = Some(value);
            }
        }
        if request.is_some() {
            break;
        }
    }
    let request = request.expect("SAVEDATA storage request");
    assert_eq!(request.namespace, StorageNamespace::Save);
    assert_eq!(request.relative_path, "save02.sav");
    let StorageOperation::Write {
        data,
        atomic_replace,
        precondition,
    } = request.operation
    else {
        panic!("SAVEDATA must write")
    };
    assert!(atomic_replace);
    assert_eq!(precondition, StoragePrecondition::Any);
    let decoded = era_runtime_save::decode(
        data.as_slice(),
        era_runtime_save::SaveCodecLimits::default(),
    )
    .expect("current save bytes");
    assert_eq!(decoded.metadata.description, "slot");
    assert_eq!(session.phase(), RuntimePhase::WaitingExternal);

    submit(
        &mut session,
        3,
        RuntimeMessage::StorageResponse(StorageResponse {
            request_id: request.request_id,
            result: StorageResult::Written {
                revision: Some("r1".into()),
            },
        }),
    );
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(
        session
            .operations
            .active_input()
            .expect("WAIT after save")
            .wait
            .kind,
        WaitKind::EnterKey
    );
    let vm = session.vm.as_ref().expect("runtime VM");
    assert_eq!(read_runtime_string(vm, "SAVEDATA_TEXT").unwrap(), "suffix");
    assert_eq!(read_runtime_integer(vm, "RESULT", &[], None).unwrap(), 20);
}

#[test]
fn chkdata_returns_its_status_and_updates_the_description() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "check-save-test".into(),
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
                relative_path: "check.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nCHKDATA 99\nWAIT\nRETURN\n".into()),
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
    let request = (0..8)
        .find_map(|_| {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            drain(&mut session)
                .into_iter()
                .find_map(|message| match message {
                    RuntimeMessage::StorageRequest(request) => Some(request),
                    _ => None,
                })
        })
        .expect("CHKDATA storage request");
    assert_eq!(request.relative_path, "save99.sav");
    assert_eq!(request.operation, StorageOperation::Read);

    submit(
        &mut session,
        3,
        RuntimeMessage::StorageResponse(StorageResponse {
            request_id: request.request_id,
            result: StorageResult::Error {
                error: era_runtime_protocol::FrontendIoError {
                    kind: FrontendIoErrorKind::NotFound,
                    message: "missing slot".into(),
                    platform_code: None,
                },
            },
        }),
    );
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }

    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(read_runtime_integer(vm, "RESULT", &[], None).unwrap(), 1);
    assert_eq!(read_runtime_string(vm, "RESULTS").unwrap(), "missing slot");
}

#[test]
#[allow(clippy::too_many_lines)]
fn saveinfo_candidate_is_isolated_until_the_storage_commit() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "candidate-test".into(),
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
                    relative_path: "candidate.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nWAIT\nRETURN\n@SAVEINFO\nRESULT = 99\nRESULT:1 = GETCONFIG(\"Font size\")\nRESULTS:1 = %BARSTR(2, 4, 4)%\nPUTFORM suffix\nRETURN\n"
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
    for _ in 0..4 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
    }
    drain(&mut session);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        0
    );

    let time = LocalDateTimeResponse {
        year: 2026,
        month: 7,
        day: 17,
        hour: 12,
        minute: 34,
        second: 56,
        millisecond: 0,
        utc_offset_minutes: 480,
    };
    let mut live = session.vm.take().unwrap();
    session
        .begin_candidate_save(&mut live, 99, CandidateSaveContinuation::Autosave)
        .unwrap();
    session.vm = Some(live);
    let stat_request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) => Some(request),
            _ => None,
        })
        .unwrap();
    assert_eq!(stat_request.operation, StorageOperation::Stat);
    session
        .complete_storage(
            0,
            StorageResponse {
                request_id: stat_request.request_id,
                result: StorageResult::Metadata(era_runtime_protocol::StorageMetadata {
                    byte_length: 12,
                    revision: Some("slot-rev".into()),
                }),
            },
        )
        .unwrap();
    let clock_request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request) => Some(request),
            _ => None,
        })
        .unwrap();
    session
        .complete_service(
            0,
            ServiceResponse {
                request_id: clock_request.request_id,
                result: ServiceResult::Ready {
                    payload: ProtocolBytes::new(encode_canonical(&time).unwrap()),
                },
            },
        )
        .unwrap();
    let write_request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) => Some(request),
            _ => None,
        })
        .unwrap();
    let StorageOperation::Write {
        data: bytes,
        atomic_replace,
        precondition,
    } = write_request.operation
    else {
        panic!("candidate did not issue a write")
    };
    assert!(atomic_replace);
    assert_eq!(
        precondition,
        StoragePrecondition::Revision("slot-rev".into())
    );
    let decoded = decode_scoped_save(
        bytes.as_slice(),
        session.vm.as_ref().unwrap().vm().artifact(),
        era_runtime_save::SaveFileKind::Normal,
    )
    .unwrap();
    assert_eq!(decoded.description, "2026/07/17 12:34:56 suffix");
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        0,
        "candidate mutation leaked before commit"
    );
    session
        .complete_storage(
            0,
            StorageResponse {
                request_id: write_request.request_id,
                result: StorageResult::Written {
                    revision: Some("new-rev".into()),
                },
            },
        )
        .unwrap();
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        99
    );
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[1], None).unwrap(),
        18
    );
    let vm = session.vm.as_ref().unwrap();
    let results = runtime_variable_key(vm, "RESULTS").unwrap();
    assert_eq!(
        vm.vm().read_variable(results, &[1], None),
        Ok(VmValue::String("[**..]".into()))
    );
}

#[test]
fn sequence_gaps_are_rejected_before_execution() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let message = RuntimeMessage::ClientHello(ClientHello {
        runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
        client_name: "test".into(),
        features: Vec::new(),
        requested_limits: RuntimeOptions::default().limits,
        capabilities: capabilities(),
        preferred_locales: vec!["ja".into()],
    });
    let envelope = message
        .envelope(None, None, 2, 1, None)
        .expect("create envelope");
    let bytes = encode_envelope(&envelope, WireLimits::default()).expect("encode envelope");
    assert!(matches!(
        session.submit_envelope(&bytes),
        Err(RuntimeError::InvalidSequence {
            expected: 0,
            actual: 2
        })
    ));
}

#[test]
fn active_session_rejects_stale_epochs_and_acknowledges_journal_entries() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: vec![RuntimeFeature::StateResynchronization],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);
    assert_eq!(session.outbound_journal.len(), 1);

    let ack = RuntimeMessage::Acknowledge(era_runtime_protocol::SequenceAcknowledgement {
        through_sequence: 0,
    });
    submit(&mut session, 1, ack);
    session.drive(RuntimeDriveBudget::default()).expect("ack");
    assert!(session.outbound_journal.is_empty());

    let message = RuntimeMessage::AdvanceTime(AdvanceTime {
        monotonic_time_ns: 1,
    });
    let mut envelope = message
        .envelope(
            Some(session.options.session_id),
            Some(SessionEpoch(session.epoch.0.saturating_sub(1))),
            2,
            3,
            None,
        )
        .expect("stale envelope");
    envelope.session = Some(session.options.session_id);
    let bytes = encode_envelope(&envelope, WireLimits::default()).expect("encode envelope");
    assert!(matches!(
        session.submit_envelope(&bytes),
        Err(RuntimeError::SessionMismatch)
    ));
}

#[test]
fn configuration_is_parsed_and_resources_receive_stable_identities() {
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
                    relative_path: "emuera.config".into(),
                    category: FileCategory::Configuration,
                    payload: FilePayload::Utf8("Language=Chinese".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "resources.csv".into(),
                    category: FileCategory::ResourceManifest,
                    payload: FilePayload::Utf8("; name,path".into()),
                    content_hash: None,
                },
            ],
        },
        None,
    );
    assert!(build.report.success);
    let codes: Vec<_> = build
        .report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert!(codes.contains(&"runtime.invalid_configuration"));
    assert!(!codes.contains(&"runtime.resource_manifest_deferred"));
    let snapshot = build.snapshot.expect("normalized project snapshot");
    assert_eq!(snapshot.resources.len(), 1);
    assert_eq!(snapshot.resources[0].relative_path, "resources.csv");
    assert_eq!(
        snapshot.resources[0].payload_digest,
        *blake3::hash(b"; name,path").as_bytes()
    );
    assert_ne!(snapshot.project_identity, [0; 32]);
}

#[test]
fn frontend_calendar_values_match_dotnet_datetime_shapes() {
    let time = LocalDateTimeResponse {
        year: 2026,
        month: 7,
        day: 15,
        hour: 13,
        minute: 4,
        second: 5,
        millisecond: 6,
        utc_offset_minutes: 480,
    };
    assert_eq!(calendar_number(time), 20_260_715_130_405_006);
    assert_eq!(calendar_string(time), "2026/07/15 13:04:05");
    assert_eq!(
        milliseconds_since_year_one(LocalDateTimeResponse {
            year: 1,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            millisecond: 0,
            utc_offset_minutes: 0,
        }),
        0
    );
    assert_eq!(milliseconds_since_year_one(time) / 1_000, 63_919_717_445);
}

#[test]
fn deterministic_width_and_integer_format_tables_cover_era_usage() {
    assert_eq!(to_full_width("ABC 123 ｶﾞﾊﾟ"), "ＡＢＣ　１２３　ガパ");
    assert_eq!(to_half_width("ＡＢＣ　１２３　ガパ"), "ABC 123 ｶﾞﾊﾟ");
    assert_eq!(convert_kana_mode("あいうガ", 1), "アイウガ");
    assert_eq!(convert_kana_mode("アイウガ", 2), "あいうが");
    assert_eq!(convert_kana_mode("ｶﾞ ABC", 3), "が　ＡＢＣ");
    assert_eq!(format_era_integer(12_345, "#,##0"), Ok("12,345".into()));
    assert_eq!(format_era_integer(-7, "D3"), Ok("-007".into()));
    assert_eq!(format_era_integer(255, "X4"), Ok("00FF".into()));
}

#[test]
fn reference_bar_and_portable_named_colors_are_deterministic() {
    assert_eq!(make_bar(5, 10, 4, '*', '.'), Ok("[**..]".into()));
    assert_eq!(
        make_bar(1, 0, 4, '*', '.'),
        Err("BAR maximum must be positive")
    );
    assert_eq!(
        make_bar(1, 2, 100, '*', '.'),
        Err("BAR length must be between 1 and 99")
    );
    assert_eq!(named_color("Magenta"), Some(0x00ff_00ff));
    assert_eq!(named_color("LightSalmon"), Some(0x00ff_a07a));
    assert_eq!(named_color("transparent"), None);
}

#[test]
fn flow_input_configuration_shapes_system_waits() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let ordinary = session.system_wait(InteractionToken { epoch: 0, id: 1 });
    assert_eq!(ordinary.kind, WaitKind::IntegerButton);
    session.flow_input_enabled = true;
    session.flow_input_default = 7;
    let integer = session.system_wait(InteractionToken { epoch: 0, id: 2 });
    assert_eq!(integer.kind, WaitKind::IntegerValue);
    assert_eq!(
        integer.default_value,
        Some(era_runtime_protocol::ProtocolValue::Integer(7))
    );
    assert!(integer.mouse_input);
    session.flow_input_string = true;
    session.flow_input_default_string = "fallback".into();
    let string = session.system_wait(InteractionToken { epoch: 0, id: 3 });
    assert_eq!(string.kind, WaitKind::StringValue);
    assert_eq!(
        string.default_value,
        Some(era_runtime_protocol::ProtocolValue::String(
            "fallback".into()
        ))
    );
}

#[test]
fn frontend_monotonic_time_rebases_onto_restored_logical_time() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.logical_time_ns = 100;
    assert_eq!(session.observe_frontend_time(5), 100);
    assert_eq!(session.observe_frontend_time(15), 110);
    assert_eq!(session.observe_frontend_time(9), 110);
}

#[test]
#[allow(clippy::too_many_lines)]
fn one_input_normalization_is_scalar_default_and_activation_aware() {
    let submission = InteractionToken { epoch: 3, id: 1 };
    let long = InteractionToken { epoch: 3, id: 2 };
    let short = InteractionToken { epoch: 3, id: 3 };
    let mut pending = PendingInput {
        host_request: None,
        wait: InputWait {
            wait_id: 1,
            kind: WaitKind::StringValue,
            stability: WaitStability::StableInput,
            one_input: true,
            stop_message_skip: false,
            system_input: false,
            mouse_input: true,
            default_value: Some(era_runtime_protocol::ProtocolValue::String(
                "DEFAULT".into(),
            )),
            deadline_ns: None,
            display_time: false,
            timeout_message: None,
            submission_token: submission,
            countdown_remaining_ms: None,
        },
        result_name: Some("RESULTS".into()),
        choices: BTreeMap::from([
            (long, VmValue::String("LONG".into())),
            (short, VmValue::String("L".into())),
        ]),
        timeout_duration_ns: None,
        post_input: None,
    };
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::CommitText("βx".into()),
            false,
        ),
        Some(InputSubmission::Value(VmValue::String("β".into())))
    );
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::CommitText("😀x".into()),
            false,
        ),
        Some(InputSubmission::Value(VmValue::String("😀".into())))
    );
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::CommitText(String::new()),
            false,
        ),
        Some(InputSubmission::Value(VmValue::String("DEFAULT".into())))
    );
    assert_eq!(
        input_value(&pending, submission, InputIntent::Activate(long), false,),
        Some(InputSubmission::Value(VmValue::String("L".into())))
    );
    assert_eq!(
        input_value(&pending, submission, InputIntent::Activate(long), true,),
        Some(InputSubmission::Value(VmValue::String("LONG".into())))
    );

    pending.wait.kind = WaitKind::IntegerValue;
    pending.wait.default_value = Some(era_runtime_protocol::ProtocolValue::Integer(42));
    pending.choices = BTreeMap::from([(long, VmValue::Integer(42)), (short, VmValue::Integer(4))]);
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::CommitText("12".into()),
            false,
        ),
        Some(InputSubmission::Value(VmValue::Integer(1)))
    );
    pending.wait.deadline_ns = Some(1_000);
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::CommitText("34".into()),
            false,
        ),
        Some(InputSubmission::Value(VmValue::Integer(3)))
    );
    pending.wait.kind = WaitKind::StringValue;
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::CommitText("yz".into()),
            false,
        ),
        Some(InputSubmission::Value(VmValue::String("y".into())))
    );
    pending.wait.kind = WaitKind::IntegerValue;
    assert!(
        input_value(
            &pending,
            submission,
            InputIntent::CommitText("-1".into()),
            false,
        )
        .is_none()
    );
    assert_eq!(
        input_value(&pending, submission, InputIntent::Activate(long), false,),
        Some(InputSubmission::Value(VmValue::Integer(4)))
    );

    pending.wait.kind = WaitKind::IntegerButton;
    assert_eq!(
        input_value(&pending, submission, InputIntent::Activate(long), false,),
        Some(InputSubmission::Value(VmValue::Integer(4)))
    );
    pending.choices.remove(&short);
    assert!(input_value(&pending, submission, InputIntent::Activate(long), false,).is_none());
    assert_eq!(
        input_value(&pending, submission, InputIntent::Activate(long), true,),
        Some(InputSubmission::Value(VmValue::Integer(42)))
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn one_input_activation_uses_the_loaded_allow_long_configuration() {
    fn run(allow_long: bool) -> (i64, String) {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "one-input-test".into(),
                features: Vec::new(),
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
                files: vec![
                    SubmittedFile {
                        relative_path: "emuera.config".into(),
                        category: FileCategory::Configuration,
                        payload: FilePayload::Utf8(format!(
                            "Allow long input by mouse for ONEINPUT:{}\n",
                            if allow_long { "YES" } else { "NO" }
                        )),
                        content_hash: None,
                    },
                    SubmittedFile {
                        relative_path: "input.erb".into(),
                        category: FileCategory::Erb,
                        payload: FilePayload::Utf8(
                            "@SYSTEM_TITLE\nPRINTBUTTON \"42\", 42\nONEINPUT\nRESULT:42 = RESULT\nPRINTBUTTON \"LONG\", \"LONG\"\nONEINPUTS\nWAIT\nRETURN\n"
                                .into(),
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
                mode: StartMode::NewGame { seed: Some(1) },
            }),
        );
        for _ in 0..16 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.operations.active_input().is_some() {
                break;
            }
        }
        let (wait_id, submission_token, activation) = {
            let pending = session.operations.active_input().unwrap();
            let activation = pending
                .choices
                .iter()
                .find_map(|(token, value)| (*value == VmValue::Integer(42)).then_some(*token))
                .unwrap();
            (
                pending.wait.wait_id,
                pending.wait.submission_token,
                activation,
            )
        };
        submit(
            &mut session,
            3,
            RuntimeMessage::Input(FrontendInput {
                wait_id,
                token: submission_token,
                monotonic_time_ns: 0,
                intent: InputIntent::Activate(activation),
                message_skip: false,
            }),
        );
        for _ in 0..16 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.operations.active_input().is_some() {
                break;
            }
        }
        let (wait_id, submission_token, activation) = {
            let pending = session.operations.active_input().unwrap();
            let activation = pending
                .choices
                .iter()
                .find_map(|(token, value)| {
                    (*value == VmValue::String("LONG".into())).then_some(*token)
                })
                .unwrap();
            (
                pending.wait.wait_id,
                pending.wait.submission_token,
                activation,
            )
        };
        submit(
            &mut session,
            4,
            RuntimeMessage::Input(FrontendInput {
                wait_id,
                token: submission_token,
                monotonic_time_ns: 0,
                intent: InputIntent::Activate(activation),
                message_skip: false,
            }),
        );
        for _ in 0..16 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.operations.active_input().is_some() {
                break;
            }
        }
        let vm = session.vm.as_ref().unwrap();
        (
            read_runtime_integer(vm, "RESULT", &[42], None).unwrap(),
            read_runtime_string(vm, "RESULTS").unwrap(),
        )
    }

    assert_eq!(run(false), (4, "L".into()));
    assert_eq!(run(true), (42, "LONG".into()));
}

#[test]
fn primitive_input_uses_runtime_selection_tokens_and_rejects_timeout_spoofing() {
    let submission = InteractionToken { epoch: 7, id: 1 };
    let selection = InteractionToken { epoch: 7, id: 2 };
    let pending = PendingInput {
        host_request: None,
        wait: InputWait {
            wait_id: 9,
            kind: WaitKind::PrimitiveMouseKey,
            stability: WaitStability::Transient,
            one_input: false,
            stop_message_skip: false,
            system_input: false,
            mouse_input: true,
            default_value: None,
            deadline_ns: Some(10),
            display_time: false,
            timeout_message: None,
            submission_token: submission,
            countdown_remaining_ms: None,
        },
        result_name: Some("RESULT".into()),
        choices: BTreeMap::from([(selection, VmValue::Integer(42))]),
        timeout_duration_ns: Some(10),
        post_input: None,
    };
    let input = era_runtime_protocol::PrimitiveInput {
        input_type: 1,
        result_1: 10,
        result_2: 20,
        result_3: 1,
        result_4: 3,
        selection_token: Some(selection),
    };
    assert_eq!(
        input_value(&pending, submission, InputIntent::Primitive(input), false),
        Some(InputSubmission::Primitive(PrimitiveResult {
            fields: [1, 10, 20, 1, 3],
            selection: Some(VmValue::Integer(42)),
        }))
    );
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::Activate(selection),
            false,
        ),
        Some(InputSubmission::Value(VmValue::Integer(42)))
    );
    assert!(
        input_value(
            &pending,
            InteractionToken { epoch: 7, id: 99 },
            InputIntent::Activate(selection),
            false,
        )
        .is_none()
    );
    assert!(
        input_value(
            &pending,
            submission,
            InputIntent::Primitive(era_runtime_protocol::PrimitiveInput {
                input_type: 4,
                result_1: 0,
                result_2: 0,
                result_3: 0,
                result_4: 0,
                selection_token: None,
            }),
            false,
        )
        .is_none()
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn project_resource_metadata_is_frontend_decoded_before_load_commit() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let mut client = capabilities();
    client.graphics = true;
    client.services.push(ServiceCapability {
        kind: ServiceKind::Image,
        operation: IMAGE_METADATA_OPERATION.into(),
        versions: VersionRange::exact(IMAGE_METADATA_OPERATION_VERSION),
    });
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "resource-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client,
            preferred_locales: vec!["en".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let _ = drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "resources/sprites.csv".into(),
                    category: FileCategory::ResourceManifest,
                    payload: FilePayload::Utf8("FACE,face.png".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "resources/face.png".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::ProjectLoadReport(_)))
    );
    let request_id = messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == IMAGE_METADATA_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("image metadata request");
    submit(
        &mut session,
        2,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&ImageMetadataResponse {
                        width: 32,
                        height: 16,
                        format: "png".into(),
                        animated: false,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| {
        matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
    }));
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .and_then(|project| project.resource_graph.sprite("face"))
            .map(|sprite| (sprite.width, sprite.height)),
        Some((32, 16))
    );

    submit(
        &mut session,
        3,
        RuntimeMessage::ReloadProject(ReloadProject {
            base_revision: 1,
            target_revision: 2,
            changes: vec![FileChange::Upsert {
                file: SubmittedFile {
                    relative_path: "resources/face.png".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![4, 5, 6])),
                    content_hash: None,
                },
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let reload_messages = drain(&mut session);
    let reload_request = reload_messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == IMAGE_METADATA_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("changed image metadata request");
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .and_then(|project| project.resource_graph.sprite("face"))
            .map(|sprite| (sprite.width, sprite.height)),
        Some((32, 16)),
        "the live graph must not change before candidate metadata commits"
    );
    submit(
        &mut session,
        4,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: reload_request,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&ImageMetadataResponse {
                        width: 64,
                        height: 24,
                        format: "png".into(),
                        animated: false,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| {
            matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success && report.project_revision == 2)
        }));
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .and_then(|project| project.resource_graph.sprite("face"))
            .map(|sprite| (sprite.width, sprite.height)),
        Some((64, 24))
    );

    submit(
        &mut session,
        5,
        RuntimeMessage::ReloadProject(ReloadProject {
            base_revision: 2,
            target_revision: 3,
            changes: vec![FileChange::Upsert {
                file: SubmittedFile {
                    relative_path: "resources/face.png".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![7, 8, 9])),
                    content_hash: None,
                },
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let failed_request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == IMAGE_METADATA_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("second changed image metadata request");
    submit(
        &mut session,
        6,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: failed_request,
            result: ServiceResult::Error {
                error: era_runtime_protocol::ServiceError {
                    code: "decoder.invalid".into(),
                    message: "invalid image".into(),
                },
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let failed = drain(&mut session);
    assert!(failed.iter().any(|message| {
            matches!(message, RuntimeMessage::ProjectLoadReport(report) if !report.success && report.project_revision == 3)
        }));
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .map(|project| project.manifest.project_revision),
        Some(2),
        "failed candidate metadata must leave the previous project authoritative"
    );
}

#[test]
fn font_profile_is_session_fixed_case_insensitive_and_deterministic() {
    let mut requested = capabilities();
    requested.available_fonts = vec!["Zeta".into(), "alpha".into(), "ALPHA".into()];
    requested.font_metrics = true;
    requested.services.push(ServiceCapability {
        kind: ServiceKind::FontMetrics,
        operation: GGET_TEXT_SIZE_OPERATION.into(),
        versions: VersionRange::exact(GGET_TEXT_SIZE_OPERATION_VERSION),
    });
    let selected = selected_capabilities(&requested);
    assert_eq!(selected.available_fonts, vec!["alpha", "Zeta"]);
    assert!(selected.font_metrics);
}

#[test]
fn effect_acknowledgements_are_exact_and_failures_become_diagnostics() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.epoch = SessionEpoch(1);
    session
        .emit_effect(EffectKind::StartAnimation("flash".into()))
        .expect("emit effect");
    let _ = drain(&mut session);
    session
        .handle_message(
            10,
            RuntimeMessage::EffectAcknowledgement(EffectAcknowledgement {
                outcomes: vec![era_runtime_protocol::EffectOutcome {
                    effect_id: 1,
                    status: EffectOutcomeStatus::Failed,
                    message: Some("device unavailable".into()),
                }],
            }),
        )
        .expect("acknowledge effect");
    assert!(session.effect_journal.is_empty());
    assert!(matches!(
        drain(&mut session).as_slice(),
        [RuntimeMessage::Diagnostic(ProtocolDiagnostic { code, .. })]
            if code == "runtime.device_effect_failed"
    ));
}

#[test]
fn return_to_title_reuses_the_loaded_artifact_without_project_loading() {
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("@SYSTEM_TITLE\nWAIT\nRETURN\n".into()),
            content_hash: None,
        }],
    };
    let mut build = crate::project::build_project(&manifest, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    let expected = build
        .artifact
        .as_ref()
        .unwrap()
        .artifact()
        .manifest
        .artifact_id;
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.epoch = SessionEpoch(1);
    session.artifact = build.artifact.take();
    session.incremental = build.incremental;
    session.project_snapshot = build.snapshot;
    session.start_new_game(7).unwrap();

    assert!(std::ptr::eq(
        session.artifact.as_ref().unwrap().artifact(),
        session.vm.as_ref().unwrap().vm().artifact(),
    ));

    session.return_to_title(99).unwrap();

    assert_eq!(session.phase, RuntimePhase::Starting);
    assert_eq!(
        session
            .artifact
            .as_ref()
            .unwrap()
            .artifact()
            .manifest
            .artifact_id,
        expected
    );
    assert!(
        drain(&mut session)
            .iter()
            .all(|message| !matches!(message, RuntimeMessage::ProjectLoadReport(_)))
    );
}

#[test]
fn compiled_cache_export_prepares_the_payload_off_thread() {
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
            content_hash: None,
        }],
    };
    let identity = crate::compiled_cache::project_identity(&manifest);
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.epoch = SessionEpoch(1);

    session
        .load_project(
            99,
            &ProjectLoadRequest {
                identity,
                manifest: Some(manifest),
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();

    assert!(session.compiled_project_cache.is_none());
    assert!(session.compiled_cache_task.is_none());
    let _ = drain(&mut session);
    session
        .export_state(
            100,
            StateExportRequest {
                kind: StateExportKind::CompiledProjectCache,
            },
        )
        .unwrap();
    assert!(session.compiled_cache_task.is_some());
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected { message, .. })
            if message == "compiled project cache preparation started"
    )));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.compiled_cache_task.is_some() {
        session.poll_compiled_cache_task().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "compiled cache worker did not finish"
        );
        std::thread::yield_now();
    }
    let completion = drain(&mut session);
    assert!(
        completion.iter().any(|message| matches!(
            message,
            RuntimeMessage::Diagnostic(ProtocolDiagnostic { code, .. })
                if code == "runtime.compiled_cache_ready"
        )),
        "{completion:#?}"
    );
    let bytes = session.compiled_project_cache.as_ref().unwrap();
    assert!(crate::compiled_cache::decode(bytes, 64 * 1024 * 1024).is_ok());

    session
        .export_state(
            101,
            StateExportRequest {
                kind: StateExportKind::CompiledProjectCache,
            },
        )
        .unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::StateExportReady(StateExportReady {
            kind: StateExportKind::CompiledProjectCache,
            result: StateExportResult::Ready { .. },
        })
    )));
}

#[test]
fn compiled_cache_export_does_not_retry_a_failed_background_build() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.compiled_cache_failure = Some("synthetic encoding failure".into());

    session
        .export_state(
            100,
            StateExportRequest {
                kind: StateExportKind::CompiledProjectCache,
            },
        )
        .unwrap();

    assert!(session.compiled_cache_task.is_none());
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::ResourceLimit,
            message,
            ..
        }) if message.contains("synthetic encoding failure")
    )));
}

#[test]
fn project_load_rejects_an_uncommitted_cache_without_changing_phase() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.epoch = SessionEpoch(1);

    session
        .load_project(
            99,
            &ProjectLoadRequest {
                identity: ProjectIdentity {
                    project_revision: 1,
                    source_digest: ProtocolBytes::new(vec![0; 32]),
                },
                manifest: Some(ProjectManifest {
                    project_revision: 1,
                    files: Vec::new(),
                }),
                compiled_cache_transfer_id: Some(123),
            },
        )
        .unwrap();

    assert_eq!(session.phase, RuntimePhase::Ready);
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));
}

#[test]
fn identity_only_project_load_requests_payload_after_a_cache_miss() {
    let manifest = ProjectManifest {
        project_revision: 4,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
            content_hash: None,
        }],
    };
    let identity = crate::compiled_cache::project_identity(&manifest);
    let session = RuntimeSession::new(RuntimeOptions::default());

    let Err(report) = session.build_project_from_cache(
        &ProjectLoadRequest {
            identity,
            manifest: None,
            compiled_cache_transfer_id: None,
        },
        None,
    ) else {
        panic!("an identity without an exact cache needs source payloads");
    };

    assert!(!report.success);
    assert!(report.payload_required);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.project_payload_required")
    );
}

#[test]
fn exact_compiled_cache_load_does_not_require_a_manifest() {
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
            content_hash: None,
        }],
    };
    let mut initial = crate::project::build_project(&manifest, None);
    assert!(initial.report.success, "{:?}", initial.report.diagnostics);
    initial.incremental.compact();
    let cache = crate::compiled_cache::encode(
        &manifest,
        &[],
        initial.artifact.as_ref().unwrap(),
        &initial.incremental,
        initial.snapshot.as_ref().unwrap(),
    )
    .unwrap();
    let mut identity = crate::compiled_cache::project_identity(&manifest);
    identity.project_revision = 8;
    let session = RuntimeSession::new(RuntimeOptions::default());

    let cached = session
        .build_project_from_cache(
            &ProjectLoadRequest {
                identity,
                manifest: None,
                compiled_cache_transfer_id: None,
            },
            Some(&cache),
        )
        .expect("an exact cache loads from source identity alone");

    assert!(cached.report.success);
    assert!(!cached.report.payload_required);
    assert_eq!(cached.report.project_revision, 8);
    assert_eq!(cached.snapshot.unwrap().manifest.project_revision, 8);
}
