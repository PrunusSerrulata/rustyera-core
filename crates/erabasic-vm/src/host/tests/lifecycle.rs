use super::*;
#[test]
fn native_registry_preserves_classification_and_never_promotes_legacy_messages() {
    struct FailingNative(ExecutionFailure);
    impl NativeService for FailingNative {
        fn call(&mut self, _: NativeCallRequest) -> Result<NativeReady, ExecutionFailure> {
            Err(self.0.clone())
        }
    }
    let key = SymbolKey([29; 16]);
    for expected in [
        native_script_failure(ScriptFaultKind::Argument, "bad script value"),
        ExecutionFailure::from("Script(Arithmetic): catch this external-looking message"),
    ] {
        let mut registry = NativeServiceRegistry::default();
        registry.register(key, FailingNative(expected.clone()));
        let mut request = classified_native_request("test", Vec::new());
        request.service_key = key;
        let failure = registry.call(key, request).unwrap_err();
        assert_eq!(failure, expected);
    }
    let mut request = classified_native_request("missing", Vec::new());
    request.service_key = key;
    let failure = NativeServiceRegistry::default()
        .call(key, request)
        .unwrap_err();
    assert_eq!(failure.category, FaultCategory::HostContract);
}

#[test]
fn format_width_rejects_script_domain_without_promoting_bad_argument_types() {
    let failure = apply_width("a", Some(&VmValue::Integer(-1)), None).unwrap_err();
    assert_eq!(
        failure.category,
        FaultCategory::Script(ScriptFaultKind::Argument)
    );
    let failure = apply_width("a", Some(&VmValue::String("3".into())), None).unwrap_err();
    assert_eq!(failure.category, FaultCategory::HostContract);
}

#[test]
fn randdata_execution_distinguishes_invalid_state_from_missing_service() {
    let mut registry = NativeServiceRegistry {
        random: Some(Arc::new(Mutex::new(Sfmt19937::new(1234)))),
        ..NativeServiceRegistry::default()
    };
    let before = registry.random_values().unwrap();
    let mut invalid = before.clone();
    invalid[624] = 625;
    let failure = registry.set_random_values_execution(&invalid).unwrap_err();
    assert_eq!(
        failure.category,
        FaultCategory::Script(ScriptFaultKind::Argument)
    );
    assert_eq!(failure.code, VmFaultCode::Native);
    assert_eq!(failure.message, "RANDDATA index exceeds 624");
    assert_eq!(registry.random_values().unwrap(), before);
    let failure = NativeServiceRegistry::default()
        .set_random_values_execution(&before)
        .unwrap_err();
    assert_eq!(failure.category, FaultCategory::InternalInvariant);
    assert_eq!(failure.code, VmFaultCode::Native);
    assert_eq!(failure.message, "random native service is not registered");
}

#[test]
fn native_registry_keeps_family_service_key_distinct_from_physical_import() {
    struct Keys;
    impl NativeService for Keys {
        fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, ExecutionFailure> {
            assert_ne!(request.service_key, request.import.key);
            Ok(NativeReady::value(VmValue::Integer(7)))
        }
    }
    let family_key = SymbolKey::derive("test", b"family");
    let mut registry = NativeServiceRegistry::default();
    registry.register(family_key, Keys);
    let mut request = classified_native_request("abs", vec![VmValue::Integer(-7)]);
    request.service_key = family_key;
    assert_eq!(
        registry.call(family_key, request.clone()).unwrap().value,
        Some(VmValue::Integer(7))
    );
    request.service_key = request.import.key;
    assert_eq!(
        registry.call(family_key, request).unwrap_err().category,
        FaultCategory::HostContract
    );
}

#[test]
fn map_candidate_protection_preserves_parent_roots_and_rejects_stale_guard() {
    use crate::structured::{MapLeaseOrigin, MapLeaseOwner};
    let mut state = StructuredState::default();
    let request = NativeCallRequest {
        service_key: SymbolKey::default(),
        omitted_arguments: Vec::new(),
        import: RuntimeImport {
            key: SymbolKey::default(),
            namespace: "rustyera.vm".into(),
            name: "map_create".into(),
            abi_version: erabasic_bytecode::NATIVE_ABI_VERSION,
            parameters: vec![erabasic_bytecode::BytecodeType::String],
            result: Some(erabasic_bytecode::BytecodeType::Integer),
        },
        arguments: vec![VmValue::String("m".into())],
        places: Vec::new(),
        implicit_places: BTreeMap::new(),
    };
    state.call("map_create", &request).unwrap();
    let owner = MapLeaseOwner {
        fiber: crate::FiberId(1),
        frame: crate::FrameId(1),
        generation: crate::GenerationId(1),
        function: SymbolKey::derive("test.map", b"candidate"),
        origin: MapLeaseOrigin::Bytecode { begin: 1 },
    };
    let parent_lease = state.capture_map("m", owner).unwrap().unwrap();
    let roots = [parent_lease].into_iter().collect::<BTreeSet<_>>();
    let parent = NativeServiceRegistry {
        structured: Some(Arc::new(Mutex::new(state.clone()))),
        ..NativeServiceRegistry::default()
    };
    let base = parent.map_lease_stamp().unwrap();
    let mut candidate = NativeServiceRegistry {
        structured: Some(Arc::new(Mutex::new(state))),
        ..NativeServiceRegistry::default()
    };
    candidate.protect_map_roots(roots.clone()).unwrap();
    candidate.retain_map_leases(&BTreeSet::new()).unwrap();
    candidate.validate_map_roots(&roots).unwrap();
    assert!(
        candidate
            .finish_map_candidate(&BTreeSet::new(), BTreeSet::new())
            .is_err()
    );
    parent.validate_map_lease_stamp(base).unwrap();
    candidate
        .finish_map_candidate(&roots, BTreeSet::new())
        .unwrap();
    parent.release_map(parent_lease).unwrap();
    assert!(parent.validate_map_lease_stamp(base).is_err());
}
