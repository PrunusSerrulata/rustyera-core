#[test]
fn drive_hint_tracks_blocked_service_inbound_and_resumed_work() {
    let (mut harness, open) =
        SqlHarness::start("@SYSTEM_TITLE\nSQL_CONNECT \"db\"\nFORCEWAIT\nRETURN\n");
    assert!(!harness.session.has_immediate_work());
    let idle = harness
        .session
        .drive(RuntimeDriveBudget::default())
        .unwrap();
    assert!(!idle.immediate_work);
    let response = open_response(&open, revision(1));
    submit(
        &mut harness.session,
        harness.next_sequence,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: open.wire.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(era_protocol::encode_canonical(&response).unwrap()),
            },
        }),
    );
    assert!(harness.session.has_immediate_work());
    let resumed = harness
        .session
        .drive(RuntimeDriveBudget {
            maximum_vm_instructions: 100,
            maximum_runtime_transitions: 1,
        })
        .unwrap();
    assert!(resumed.immediate_work);
    let waiting = harness
        .session
        .drive(RuntimeDriveBudget::default())
        .unwrap();
    assert!(!waiting.immediate_work);
}

#[test]
fn drive_hint_checks_runnable_fibers_not_the_presence_of_a_blocked_fiber() {
    let (mut harness, _) = SqlHarness::start(
        "@SYSTEM_TITLE\nCALL WORKER\nSQL_CONNECT \"db\"\nFORCEWAIT\nRETURN\n@WORKER\nIF FLAG:0 == 0\nFLAG:0 = 1\nRETURN\nENDIF\nWHILE 1\nFLAG:0 += 1\nWEND\n",
    );
    // A Running actor may contain both a blocked fiber and independent runnable work.
    // SQL itself uses the stronger WaitingExternal actor barrier; preserve that barrier too.
    let vm = harness.session.vm.as_mut().unwrap();
    let worker = vm
        .vm()
        .artifact()
        .functions
        .iter()
        .find(|function| function.name == "WORKER")
        .unwrap()
        .key;
    vm.spawn_entry(worker, Vec::new()).unwrap();
    assert!(!harness.session.has_immediate_work());
    harness.session.phase = RuntimePhase::Running;
    assert!(harness.session.has_immediate_work());
    let report = harness
        .session
        .drive(RuntimeDriveBudget {
            maximum_vm_instructions: 100,
            maximum_runtime_transitions: 16,
        })
        .unwrap();
    assert!(report.immediate_work);
    assert!(report.vm_instructions > 0);
}
