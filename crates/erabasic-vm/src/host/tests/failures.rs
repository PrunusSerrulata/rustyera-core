use super::*;
#[test]
fn core_native_domains_are_script_failures_but_malformed_arguments_are_contract_failures() {
    for (name, arguments, kind) in [
        (
            "abs",
            vec![VmValue::Integer(i64::MIN)],
            ScriptFaultKind::Arithmetic,
        ),
        (
            "sqrt",
            vec![VmValue::Integer(-1)],
            ScriptFaultKind::Argument,
        ),
        (
            "log",
            vec![VmValue::Integer(0)],
            ScriptFaultKind::Arithmetic,
        ),
        (
            "toint",
            vec![VmValue::String("0b2".into())],
            ScriptFaultKind::Parse,
        ),
        (
            "strcount",
            vec![VmValue::String("text".into()), VmValue::String("[".into())],
            ScriptFaultKind::Parse,
        ),
        (
            "encodetouni",
            vec![VmValue::String("a".into()), VmValue::Integer(2)],
            ScriptFaultKind::Bounds,
        ),
        (
            "unicodetostr",
            vec![VmValue::Integer(0xD800)],
            ScriptFaultKind::Argument,
        ),
    ] {
        let failure = CoreNative::new(name.into(), LegacyEncoding::default())
            .call(classified_native_request(name, arguments))
            .unwrap_err();
        assert_eq!(failure.category, FaultCategory::Script(kind), "{name}");
        assert_eq!(failure.code, VmFaultCode::Native, "{name}");
    }
    for name in ["unchecked_add", "toint", "not_a_native"] {
        let failure = CoreNative::new(name.into(), LegacyEncoding::default())
            .call(classified_native_request(
                name,
                vec![VmValue::Integer(1), VmValue::String("wrong".into())],
            ))
            .unwrap_err();
        assert_eq!(failure.category, FaultCategory::HostContract, "{name}");
        assert!(!failure.is_script());
    }
}

#[test]
fn snake_numeric_read_fallback_does_not_hide_native_contract_failures() {
    let compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let mut native = CoreNative::new("toint".into(), LegacyEncoding::default())
        .with_compatibility(&compatibility);
    assert_eq!(
        native
            .call(classified_native_request(
                "toint",
                vec![VmValue::String("0b2".into())]
            ))
            .unwrap()
            .value,
        Some(VmValue::Integer(0))
    );
    let failure = native
        .call(classified_native_request(
            "toint",
            vec![VmValue::Integer(1)],
        ))
        .unwrap_err();
    assert_eq!(failure.category, FaultCategory::HostContract);
    let pure = evaluate_pure_native("sqrt", vec![VmValue::Integer(-1)]).unwrap_err();
    assert_eq!(pure, "SQRT argument 1 is negative");
}

#[test]
fn regex_runtime_capacity_is_uncatchable_even_with_native_legacy_code() {
    let error = fancy_regex::Error::RuntimeError(fancy_regex::RuntimeError::BacktrackLimitExceeded);
    let failure = super::core::regex_runtime_failure("STRCOUNT", &error);
    assert_eq!(failure.category, FaultCategory::ResourceLimit);
    assert_eq!(failure.code, VmFaultCode::Native);
    assert!(
        failure
            .message
            .starts_with("STRCOUNT regex execution failed:")
    );
    assert!(!failure.is_script());
}

#[test]
fn regex_compile_errors_keep_the_core_native_boundary_contract() {
    let error = crate::regex_compat::build("[").unwrap_err();
    let failure = super::core::regex_compile_failure("REPLACE", &error);
    assert_eq!(
        failure.category,
        FaultCategory::Script(ScriptFaultKind::Parse)
    );
    assert_eq!(failure.code, VmFaultCode::Native);
    assert!(
        failure
            .message
            .starts_with("REPLACE argument 2 is not a regex:")
    );
}

#[test]
fn replace_mode_domain_is_separate_from_missing_native_place_views() {
    let mut native = CoreNative::new("replace".into(), LegacyEncoding::default());
    let request = |third| {
        classified_native_request(
            "replace",
            vec![
                VmValue::String("a".into()),
                VmValue::String("a".into()),
                third,
                VmValue::Integer(1),
            ],
        )
    };
    let domain = native
        .call(request(VmValue::String("replacement".into())))
        .unwrap_err();
    assert_eq!(
        domain.category,
        FaultCategory::Script(ScriptFaultKind::Argument)
    );
    let malformed = native
        .call(request(VmValue::StringPlace(Box::default())))
        .unwrap_err();
    assert_eq!(malformed.category, FaultCategory::HostContract);
}
