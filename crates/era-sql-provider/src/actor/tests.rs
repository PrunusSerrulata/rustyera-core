use super::*;
use era_runtime_protocol::{
    SqlConnectionHandleV1, SqlErrorCodeV1, SqlOperationV1, SqlResultV1, StorageNamespace,
    StorageOperation, StorageResult,
};

fn handle(id: u64) -> SqlProviderHandleV1 {
    SqlProviderHandleV1 {
        service_epoch: 1,
        id,
    }
}

fn request() -> SqlRequestV1 {
    SqlRequestV1 {
        provider: handle(1),
        operation: SqlOperationV1::Disconnect {
            connection: SqlConnectionHandleV1 {
                service_epoch: 1,
                id: 1,
            },
        },
    }
}

fn storage_request() -> StorageRequest {
    StorageRequest {
        request_id: 7,
        namespace: StorageNamespace::Data,
        relative_path: "sql/test".into(),
        operation: StorageOperation::Read,
        idempotency_key: "test".into(),
        deadline_ns: None,
    }
}

fn storage_error(request_id: u64) -> StorageResponse {
    StorageResponse {
        request_id,
        result: StorageResult::Error {
            error: era_runtime_protocol::FrontendIoError {
                kind: era_runtime_protocol::FrontendIoErrorKind::Other,
                message: "fixture".into(),
                platform_code: None,
            },
        },
    }
}

#[test]
fn cancellation_is_cloneable_terminal_and_shutdown_confirms_thread_exit() {
    let mut owner = NativeSqlProvider::new().unwrap();
    let cancellation = owner.cancellation_handle();
    thread::spawn(move || cancellation.clone().cancel())
        .join()
        .unwrap();
    let mut callbacks = 0;
    assert!(
        owner
            .handle(request(), 2, &mut |request: StorageRequest| {
                callbacks += 1;
                storage_error(request.request_id)
            })
            .is_err()
    );
    assert_eq!(callbacks, 0);
    assert!(owner.reset().is_err());
    assert!(
        owner
            .register(handle(1), crate::ProviderRole::Live)
            .is_err()
    );
    owner.shutdown().unwrap();
    assert!(owner.completed);
    assert!(owner.owner.is_none());
    owner.shutdown().unwrap();
}

#[test]
fn lifecycle_ack_preserves_reset_registration_high_water_mark() {
    let mut owner = NativeSqlProvider::new().unwrap();
    owner
        .register(handle(1), crate::ProviderRole::Live)
        .unwrap();
    owner
        .register(handle(2), crate::ProviderRole::Candidate)
        .unwrap();
    owner.promote_candidate(handle(2)).unwrap();
    owner.reset().unwrap();
    assert_eq!(
        owner
            .register(handle(1), crate::ProviderRole::Live)
            .unwrap_err()
            .code,
        SqlErrorCodeV1::StaleEpoch
    );
    let response = owner
        .handle(request(), 2, &mut |_: StorageRequest| {
            panic!("stale handle must not reach storage")
        })
        .unwrap();
    assert!(matches!(response.result, SqlResultV1::Error { error }
        if error.code == SqlErrorCodeV1::StaleEpoch));
    // A structured lifecycle rejection must not poison a healthy transport.
    owner
        .register(handle(3), crate::ProviderRole::Live)
        .unwrap();
    owner.retire(handle(3)).unwrap();
    owner.shutdown().unwrap();
}

#[test]
fn expired_request_cannot_start_a_storage_continuation_or_receive_a_queued_reply() {
    let control = Control::default();
    assert!(control.begin_request(Instant::now()).is_err());
    let (sender, receiver) = mpsc::sync_channel(1);
    let mut storage = ContinuationStorage {
        replies: sender,
        control: control.clone(),
    };
    assert!(matches!(
        storage.handle(storage_request()).result,
        StorageResult::Error { .. }
    ));
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    let (sender, receiver) = mpsc::sync_channel(1);
    sender.try_send(42).unwrap();
    assert!(receive(&receiver, &control).is_err());
    assert!(
        control
            .begin_request(Instant::now() + TRANSPORT_BUDGET)
            .is_err()
    );
}

#[test]
fn callback_cancellation_discards_returned_value_and_prevents_further_storage() {
    let (commands, receiver) = mpsc::sync_channel(1);
    let (done, completion) = mpsc::sync_channel(1);
    let control = Control::default();
    let worker_control = control.clone();
    let worker = thread::spawn(move || {
        let _completion = Completion(done);
        let Command::Execute(_, _, replies) = receive(&receiver, &worker_control).unwrap() else {
            panic!("expected execute");
        };
        let (sender, response) = mpsc::sync_channel(1);
        replies
            .try_send(Reply::Storage(storage_request(), sender))
            .ok()
            .unwrap();
        assert!(receive(&response, &worker_control).is_err());
        let mut continuation = ContinuationStorage {
            replies,
            control: worker_control,
        };
        assert!(matches!(
            continuation.handle(storage_request()).result,
            StorageResult::Error { .. }
        ));
    });
    let mut owner = NativeSqlProvider {
        commands,
        control,
        completion,
        owner: Some(worker),
        completed: false,
    };
    let cancellation = owner.cancellation_handle();
    let mut callbacks = 0;
    assert!(
        owner
            .handle(request(), 2, &mut |request: StorageRequest| {
                callbacks += 1;
                cancellation.cancel();
                storage_error(request.request_id)
            })
            .is_err()
    );
    assert_eq!(callbacks, 1);
    owner.shutdown().unwrap();
}
