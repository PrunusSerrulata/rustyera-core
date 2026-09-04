#[test]
fn script_sequence_precedes_but_preserves_previously_expanded_macro_tails() {
    let mut session = start_snake_input_project(
        "@SYSTEM_TITLE\n#DIM FIRST\n#DIM SECOND\nINPUT\nRESULT = SEQUENCEINPUT(\"21\")\nINPUT\nFIRST = RESULT\nINPUT\nSECOND = RESULT\nFORCEWAIT\nRETURN\n",
        capabilities(),
    );
    submit_text(&mut session, 3, "1\\n90");
    drive_input_set(&mut session);
    assert_eq!(runtime_integer(&session, "FIRST"), 21);
    assert_eq!(runtime_integer(&session, "SECOND"), 90);
    assert!(session.input_controller.pending_sequence.is_none());
    let records = input_replay_records(&session);
    assert_eq!(records[0]["step_count"], 3);
    assert_eq!(records[2]["source"]["raw"], "21");
    assert_eq!(records[3]["source"]["fragment"], 1);
}

#[test]
fn script_sequence_last_write_wins_including_the_empty_string() {
    let mut session = start_snake_input_project(
        "@SYSTEM_TITLE\n#DIMS SEEN\nINPUT\nRESULT = SEQUENCEINPUT(\"discard\")\nRESULT = SEQUENCEINPUT(\"\")\nINPUTS\nSEEN '= RESULTS\nFORCEWAIT\nRETURN\n",
        capabilities(),
    );
    submit_text(&mut session, 3, "1");
    drive_input_set(&mut session);
    assert_eq!(runtime_string(&session, "SEEN"), "");
    assert!(session.input_controller.pending_sequence.is_none());
    assert_eq!(input_replay_records(&session)[2]["source"]["raw"], "");
}

#[test]
fn disabling_macro_preserves_admitted_tails_and_admits_new_text_as_one_literal() {
    let mut session = start_snake_input_project(
        "@SYSTEM_TITLE\n#DIM FIRST\n#DIMS SEEN\nINPUT\nRESULT = DISABLE_INPUT_MACRO()\nINPUTS\nFIRST = TOINT(RESULTS)\nINPUTS\nSEEN '= RESULTS\nFORCEWAIT\nRETURN\n",
        capabilities(),
    );
    submit_text(&mut session, 3, "1\\n2");
    drive_input_set(&mut session);
    assert_eq!(runtime_integer(&session, "FIRST"), 2);
    let literal = "@CONFIG(1\\n2)*2\\e";
    submit_text(&mut session, 4, literal);
    drive_input_set(&mut session);
    assert_eq!(runtime_string(&session, "SEEN"), literal);
    assert!(session.queued_input.is_empty());
    assert!(!session.message_skip);
}

#[test]
fn inactive_compiled_and_form_key_queries_skip_argument_and_preserve_latch() {
    let mut session = start_snake_input_project(
        "@SYSTEM_TITLE\n#DIM SEEN\nINPUT\nRESULT = GETKEY(BUMP())\nRESULT = STRFORMCHECK(\"{GETKEYTRIGGERED(BUMP())}\")\nINPUT\nSEEN = TOINT(STRFORM(\"{GETKEYTRIGGERED(65)}\"))\nFORCEWAIT\nRETURN\n@BUMP\n#FUNCTION\nFLAG:0 += 1\nRETURNF 65\n",
        snake_input_capabilities(),
    );
    send_key(&mut session, 3, 1, true);
    send_key(&mut session, 4, 2, false);
    send_focus(&mut session, 5, false);
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let inactive_devices = session.device_input.clone();
    submit_text(&mut session, 6, "0");
    drive_input_set(&mut session);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "FLAG", &[0], None).unwrap(),
        0
    );
    assert_eq!(session.device_input, inactive_devices);
    send_focus(&mut session, 7, true);
    submit_text(&mut session, 8, "1");
    drive_input_set(&mut session);
    assert_eq!(runtime_integer(&session, "SEEN"), 1);
    assert_eq!(session.device_input.snake_query(65, true), 0);
}

#[test]
fn unavailable_latch_skips_key_arguments_and_warns_once_per_execution_site() {
    let mut session = start_snake_input_project(
        "@SYSTEM_TITLE\nFOR LOCAL, 0, 2\nRESULT = GETKEY(BUMP())\nNEXT\nWAIT\nRETURN\n@BUMP\n#FUNCTION\nFLAG:0 += 1\nRETURNF 65\n",
        capabilities(),
    );
    assert_eq!(runtime_integer(&session, "RESULT"), 0);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "FLAG", &[0], None).unwrap(),
        0
    );
    let notices = drain(&mut session)
        .into_iter()
        .filter(|message| {
            matches!(message,
            RuntimeMessage::Diagnostic(value)
                if value.code == "compat.input.device_latch_unavailable" && value.source.is_some())
        })
        .count();
    assert_eq!(notices, 1);
}

#[test]
fn await_zero_requires_ack_and_only_new_ordered_events_survive_the_pump() {
    let mut session = start_snake_input_project(
        "@SYSTEM_TITLE\n#DIM SEEN\nINPUT\nAWAIT 0\nSEEN = GETKEYTRIGGERED(65)\nFORCEWAIT\nRETURN\n",
        snake_input_capabilities(),
    );
    send_focus(&mut session, 3, true);
    send_key(&mut session, 4, 1, true);
    send_key(&mut session, 5, 2, false);
    submit_text(&mut session, 6, "0");
    let mut pump = None;
    for _ in 0..32 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        for message in drain(&mut session) {
            if let RuntimeMessage::ServiceRequest(request) = message
                && request.operation == DEVICE_PUMP_OPERATION
            {
                pump = Some(request);
            }
        }
        if pump.is_some() {
            break;
        }
    }
    let pump = pump.expect("AWAIT 0 must emit a device pump request");
    let request: DevicePumpRequest =
        era_protocol::decode_canonical(pump.payload.as_slice()).unwrap();
    assert_eq!(request.after_event_sequence, 2);
    session
        .negotiated_features
        .insert(RuntimeFeature::VmSnapshot);
    for purpose in [
        SnapshotExportPurpose::Normal,
        SnapshotExportPurpose::Debug,
        SnapshotExportPurpose::Diagnosis,
    ] {
        session
            .export_state(
                0,
                StateExportRequest {
                    kind: StateExportKind::VmSnapshot,
                    snapshot_purpose: purpose,
                },
            )
            .unwrap();
        assert!(drain(&mut session).iter().any(|message| matches!(message,
            RuntimeMessage::StateExportReady(StateExportReady { result: StateExportResult::Ineligible { reasons }, .. })
            if reasons.contains(&SnapshotIneligibleReason::SnapshotStateUnavailable))));
    }
    assert_eq!(runtime_integer(&session, "SEEN"), 0);
    assert_eq!(session.device_input.snake_query(65, true), 0);
    send_key(&mut session, 7, 3, true);
    send_key(&mut session, 8, 4, false);
    submit(
        &mut session,
        9,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: pump.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&DevicePumpResponse {
                        epoch: request.epoch,
                        through_event_sequence: 4,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    drive_input_set(&mut session);
    assert_eq!(runtime_integer(&session, "SEEN"), 1);
    send_key(&mut session, 10, 3, true);
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_rejection(&mut session, CommandErrorCode::StaleRequest);
    assert_eq!(session.device_input.snake_query(65, true), 0);
}

#[test]
fn admission_queued_before_epoch_change_is_rejected_when_drained() {
    let mut session = start_input_project("@SYSTEM_TITLE\nINPUT\nRETURN\n");
    send_key(&mut session, 3, 1, true);
    session.advance_epoch();
    session.drive(single_transition_budget()).unwrap();
    assert_rejection(&mut session, CommandErrorCode::StaleRequest);
    assert_eq!(session.device_input.event_sequence, 0);
}

#[test]
fn environment_negotiation_controls_platform_value_without_claiming_a_host_os() {
    for (client, platform, known) in [(capabilities(), 5, 0), (snake_input_capabilities(), 0, 1)] {
        let mut session = start_snake_input_project(
            "@SYSTEM_TITLE\n#DIM PLATFORM\n#DIM KNOWN\n#DIM ZERO\n#DIM NEGATIVE\nFOR LOCAL, 0, 2\nPLATFORM = GETPLATFORM()\nNEXT\nKNOWN = ENV_HAS_CAPABILITY(\"input.timed_viewport\")\nZERO = ENV_HAS_CAPABILITY(\"input.timed_viewport\", 0)\nNEGATIVE = ENV_HAS_CAPABILITY(\"input.timed_viewport\", -1)\nRESULT = ENV_HAS_CAPABILITY(\"unknown.capability\")\nWAIT\nRETURN\n",
            client,
        );
        assert_eq!(runtime_integer(&session, "PLATFORM"), platform);
        assert_eq!(runtime_integer(&session, "KNOWN"), known);
        assert_eq!(runtime_integer(&session, "ZERO"), 0);
        assert_eq!(runtime_integer(&session, "NEGATIVE"), 0);
        assert_eq!(runtime_integer(&session, "RESULT"), 0);
        let messages = drain(&mut session);
        let notices: Vec<_> = messages
            .iter()
            .filter_map(|message| match message {
                RuntimeMessage::Diagnostic(value)
                    if value.code == "compat.portability.platform_mapping" =>
                {
                    Some(value)
                }
                _ => None,
            })
            .collect();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].source.is_some());
    }
}

#[test]
fn sequence_waiting_for_admission_rejects_all_snapshot_purposes() {
    let mut session = start_snake_input_project("@SYSTEM_TITLE\nINPUT\nRETURN\n", capabilities());
    let vm = session.vm.as_ref().unwrap();
    session.input_controller.pending_sequence = Some(PendingSequence {
        text: String::new(),
        site: SequenceSite {
            artifact: vm.artifact_id(),
            function: vm.vm().artifact().functions[0].key,
            instruction: 0,
        },
    });
    for purpose in [
        SnapshotExportPurpose::Normal,
        SnapshotExportPurpose::Debug,
        SnapshotExportPurpose::Diagnosis,
    ] {
        session
            .export_state(
                0,
                StateExportRequest {
                    kind: StateExportKind::VmSnapshot,
                    snapshot_purpose: purpose,
                },
            )
            .unwrap();
        assert!(drain(&mut session).iter().any(|message| matches!(message,
            RuntimeMessage::StateExportReady(StateExportReady { result: StateExportResult::Ineligible { reasons }, .. })
            if reasons.contains(&SnapshotIneligibleReason::SnapshotStateUnavailable))));
    }
    assert!(session.input_controller.pending_sequence.is_some());
}

#[test]
fn undo_regenerates_script_sequence_without_injecting_a_second_copy_and_restores_macro_switch() {
    let source = concat!(
        "@SYSTEM_TITLE\nCALL SHARED_INPUT\nRETURN\n@SHOW_SHOP\nCALL SHARED_INPUT\nRETURN\n",
        "@SHARED_INPUT\nINPUTS\nSAVESTR:0 '= RESULTS\nRESULT = SEQUENCEINPUT(\"script\")\nINPUTS\nSAVESTR:1 '= RESULTS\nINPUTS\nWAIT\nRETURN\n"
    );
    let mut session = start_input_project_with(
        source,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        capabilities(),
        true,
    );
    let random = session.vm.as_ref().unwrap().export_random_state().unwrap();
    let baseline = {
        let vm = session.vm.as_ref().unwrap();
        encode_scoped_save(
            &vm.export_era_state(),
            vm.vm().artifact(),
            era_runtime_save::SaveFileKind::Normal,
            "input checkpoint".into(),
            Vec::new(),
            session.traditional_save_format(),
        )
        .unwrap()
    };
    session
        .establish_input_undo_checkpoint(3, baseline, random)
        .unwrap();
    submit_text(&mut session, 3, "external");
    drive_input_set(&mut session);
    assert_eq!(session.undo_checkpoint.as_ref().unwrap().inputs.len(), 2);
    submit_text(&mut session, 4, "remove");
    drive_input_set(&mut session);
    session.input_controller.macro_enabled = false;
    let token = session.input_undo_state().token.expect("undo token");
    submit(
        &mut session,
        5,
        RuntimeMessage::InputUndoRequest(InputUndoRequest { token }),
    );
    for _ in 0..64 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.undo_replay.is_none() && session.operations.active_input().is_some() {
            break;
        }
    }
    assert_eq!(session.phase, RuntimePhase::WaitingInput);
    assert_wait(&session, WaitKind::StringValue);
    assert!(session.input_controller.macro_enabled);
    assert!(session.input_controller.pending_sequence.is_none());
    assert!(session.queued_input.is_empty());
    let checkpoint = session.undo_checkpoint.as_ref().unwrap();
    assert_eq!(
        checkpoint
            .inputs
            .iter()
            .map(|input| input.value.as_str())
            .collect::<Vec<_>>(),
        ["external", "script"]
    );
    assert!(matches!(
        checkpoint.inputs[1].source.as_ref().unwrap().root,
        InputRoot::Sequence(_)
    ));
    assert_eq!(
        input_replay_records(&session)[0]["step_count"],
        0,
        "automatic revalidation is not a new frontend admission"
    );
}

#[test]
fn await_positive_duration_starts_after_ack_not_at_request_creation() {
    let mut session = start_snake_input_project(
        "@SYSTEM_TITLE\n#DIM SEEN\nINPUT\nAWAIT 10\nSEEN = 1\nFORCEWAIT\nRETURN\n",
        snake_input_capabilities(),
    );
    submit_text(&mut session, 3, "0");
    let mut pump = None;
    for _ in 0..32 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        for message in drain(&mut session) {
            if let RuntimeMessage::ServiceRequest(request) = message
                && request.operation == DEVICE_PUMP_OPERATION
            {
                pump = Some(request);
            }
        }
        if pump.is_some() {
            break;
        }
    }
    let pump = pump.expect("device pump");
    let request: DevicePumpRequest =
        era_protocol::decode_canonical(pump.payload.as_slice()).unwrap();
    session.observe_frontend_time(0);
    submit(
        &mut session,
        4,
        RuntimeMessage::AdvanceTime(AdvanceTime {
            monotonic_time_ns: 100_000_000,
        }),
    );
    submit(
        &mut session,
        5,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: pump.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&DevicePumpResponse {
                        epoch: request.epoch,
                        through_event_sequence: request.after_event_sequence,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_eq!(runtime_integer(&session, "SEEN"), 0);
    submit(
        &mut session,
        6,
        RuntimeMessage::AdvanceTime(AdvanceTime {
            monotonic_time_ns: 109_999_999,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_eq!(runtime_integer(&session, "SEEN"), 0);
    submit(
        &mut session,
        7,
        RuntimeMessage::AdvanceTime(AdvanceTime {
            monotonic_time_ns: 110_000_000,
        }),
    );
    drive_input_set(&mut session);
    assert_eq!(runtime_integer(&session, "SEEN"), 1);
}
