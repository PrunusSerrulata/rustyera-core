use super::ports::CaptureHost;
use super::*;
use erabasic_bytecode::RuntimeImport;
fn host_request(id: u64) -> HostCallRequest {
    HostCallRequest {
        id: HostRequestId(id),
        fiber: FiberId(id.saturating_add(100)),
        import: RuntimeImport {
            key: SymbolKey([u8::try_from(id).unwrap_or(u8::MAX); 16]),
            namespace: "test".into(),
            name: format!("HOST_{id}"),
            abi_version: 1,
            parameters: Vec::new(),
            result: None,
        },
        arguments: vec![VmValue::Integer(i64::try_from(id).unwrap_or(i64::MAX))],
        omitted_arguments: Vec::new(),
        origin: crate::VmExecutionOrigin {
            generation: GenerationId(1),
            function: SymbolKey([0; 16]),
            function_name: "TEST".into(),
            instruction: u32::try_from(id).unwrap_or(u32::MAX),
            command: format!("HOST_{id}"),
            source: None,
        },
    }
}

#[test]
fn capture_host_keeps_the_single_request_inline() {
    let mut host = CaptureHost::default();
    let request = host_request(1);
    assert_eq!(host.call(request.clone()), HostCallResult::Deferred);
    assert!(host.overflow.is_empty());
    assert_eq!(host.take(request.id), Some(request));
    assert!(host.is_empty());
}

#[test]
fn capture_host_preserves_multiple_fibers_without_fifo_assumptions() {
    let mut host = CaptureHost::default();
    let requests = [host_request(1), host_request(2), host_request(3)];
    for request in &requests {
        assert_eq!(host.call(request.clone()), HostCallResult::Deferred);
    }
    assert_eq!(host.take(requests[1].id), Some(requests[1].clone()));
    assert_eq!(host.take(requests[0].id), Some(requests[0].clone()));
    assert_eq!(host.take(requests[2].id), Some(requests[2].clone()));
    assert!(host.is_empty());
}
