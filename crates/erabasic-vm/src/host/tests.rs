use super::*;

#[test]
fn registered_override_is_not_path_memo_safe() {
    let key = SymbolKey([7; 16]);
    let mut registry = NativeServiceRegistry::default();
    registry.path_memo_safe_keys.insert(key);

    assert!(registry.path_memo_safe(key));
    assert!(registry.register(
        key,
        CoreNative::new("abs".into(), LegacyEncoding::default()),
    ));
    assert!(!registry.path_memo_safe(key));
}

#[test]
fn compiler_formatters_are_path_memo_safe_until_overridden() {
    assert!(compiler_native_path_memo_safe("format_integer"));
    assert!(compiler_native_path_memo_safe("format_string"));
    assert!(!compiler_native_path_memo_safe("times"));

    let formatter = SymbolKey([9; 16]);
    let mut registry = NativeServiceRegistry::default();
    registry.path_memo_safe_keys.insert(formatter);
    assert!(registry.register(
        formatter,
        CompilerNative {
            name: "format_integer".into(),
            character_width_mode: CharacterWidthModeHandle::default(),
        },
    ));
    assert!(!registry.path_memo_safe(formatter));
}

#[test]
fn form_width_honors_alignment_and_unicode_display_columns() {
    assert_eq!(
        apply_width("7", Some(&VmValue::Integer(3)), Some(&VmValue::Integer(0)),).unwrap(),
        "  7"
    );
    assert_eq!(
        apply_width("7", Some(&VmValue::Integer(3)), Some(&VmValue::Integer(1)),).unwrap(),
        "7  "
    );
    assert_eq!(
        apply_width(
            "你",
            Some(&VmValue::Integer(20)),
            Some(&VmValue::Integer(1)),
        )
        .unwrap(),
        format!("你{}", " ".repeat(18))
    );
    assert_eq!(
        apply_width(
            "霊夢",
            Some(&VmValue::Integer(20)),
            Some(&VmValue::Integer(1)),
        )
        .unwrap(),
        format!("霊夢{}", " ".repeat(16))
    );
    assert_eq!(
        apply_width(
            "■……■",
            Some(&VmValue::Integer(12)),
            Some(&VmValue::Integer(1)),
        )
        .unwrap(),
        format!("■……■{}", " ".repeat(4))
    );
    assert_eq!(
        apply_width(
            "■……■",
            Some(&VmValue::Integer(12)),
            Some(&VmValue::Integer(0)),
        )
        .unwrap(),
        format!("{}■……■", " ".repeat(4))
    );
    assert_eq!(
        apply_width(
            "■……■",
            Some(&VmValue::Integer(4)),
            Some(&VmValue::Integer(1)),
        )
        .unwrap(),
        "■……■"
    );
    assert!(apply_width("x", Some(&VmValue::Integer(-1)), None).is_err());
}

#[test]
fn owned_form_width_reuses_unpadded_and_left_padded_storage() {
    let mut value = String::with_capacity(16);
    value.push('7');
    let allocation = value.as_ptr();
    let value =
        apply_owned_width_with_mode(value, None, None, crate::CharacterWidthMode::Automatic)
            .unwrap();
    assert_eq!(value.as_ptr(), allocation);

    let value = apply_owned_width_with_mode(
        value,
        Some(&VmValue::Integer(3)),
        Some(&VmValue::Integer(1)),
        crate::CharacterWidthMode::Automatic,
    )
    .unwrap();
    assert_eq!(value, "7  ");
    assert_eq!(value.as_ptr(), allocation);
}

#[test]
fn form_width_uses_the_selected_project_column_policy() {
    let width = Some(&VmValue::Integer(4));
    let left = Some(&VmValue::Integer(1));
    assert_eq!(
        apply_width_with_mode("☀", width, left, crate::CharacterWidthMode::Automatic).unwrap(),
        "☀  "
    );
    assert_eq!(
        apply_width_with_mode("☀", width, left, crate::CharacterWidthMode::AmbiguousNarrow,)
            .unwrap(),
        "☀   "
    );
    assert_eq!(
        apply_width_with_mode("…", width, left, crate::CharacterWidthMode::AmbiguousWide,).unwrap(),
        "…  "
    );
}

#[test]
fn getline_repeats_ambiguous_cjk_glyphs_by_their_full_width() {
    assert_eq!(crate::logical_line_string("■", 8).unwrap(), "■■■■");
    assert_eq!(crate::logical_line_string("…", 6).unwrap(), "………");
    assert_eq!(crate::logical_line_string("A■", 7).unwrap(), "A■A■A");
    assert_eq!(
        crate::logical_line_string("\u{200b}■", 5).unwrap(),
        "\u{200b}■\u{200b}■"
    );
    assert_eq!(crate::logical_line_string("■", 1).unwrap(), "");
    assert!(crate::logical_line_string("\u{200b}", 8).is_err());
}

#[test]
fn non_u_substring_uses_legacy_bytes_and_advances_to_boundaries() {
    assert_eq!(
        substring_legacy_bytes("A界B", 1, Some(1), LegacyEncoding::ChineseHans),
        "界"
    );
    assert_eq!(
        substring_legacy_bytes("A界B", 2, Some(1), LegacyEncoding::ChineseHans),
        "B"
    );
    assert_eq!(substring_scalars("A界B", 1, Some(1)), "界");
    assert_eq!(
        substring_legacy_bytes("abcdef", 2, Some(-1), LegacyEncoding::ChineseHans),
        "cdef"
    );
    assert_eq!(substring_scalars("abcdef", 2, Some(-1)), "cdef");
    assert_eq!(
        substring_legacy_bytes("abcdef", -1, Some(2), LegacyEncoding::ChineseHans),
        "ab"
    );
    assert_eq!(substring_scalars("abcdef", -1, Some(2)), "ab");
}

#[test]
fn context_free_strform_requires_the_vm_for_runtime_expansion() {
    assert_eq!(
        evaluate_pure_native("STRFORM", vec![VmValue::String("plain text".into())]),
        Ok(VmValue::String("plain text".into()))
    );
    assert_eq!(
        evaluate_pure_native("STRFORM", vec![VmValue::String("%RESULTS%".into())]),
        Err("STRFORM template requires VM execution context".into())
    );
    for template in [
        "%", "{RESULT}", "}", r"\s", "***", "+++", "===", "///", "$$$",
    ] {
        assert_eq!(
            evaluate_pure_native("STRFORM", vec![VmValue::String(template.into())]),
            Err("STRFORM template requires VM execution context".into()),
            "{template:?}",
        );
    }
}

#[test]
fn era_numeric_parser_keeps_reference_prefix_fraction_and_whitespace_rules() {
    assert_eq!(parse_era_numeric("12.99", false), Ok(Some(12)));
    assert_eq!(parse_era_numeric("0x10", false), Ok(Some(16)));
    assert_eq!(parse_era_numeric("0b101", true), Ok(Some(5)));
    assert_eq!(parse_era_numeric("2e3", false), Ok(Some(2_000)));
    assert_eq!(parse_era_numeric(" 12", false), Ok(None));
    assert_eq!(parse_era_numeric("１２", true), Ok(None));
    assert_eq!(parse_era_numeric("12x", true), Ok(None));
}

#[test]
fn snake_toint_catches_integer_reader_errors_without_changing_isnumeric() {
    let reference = erabasic_compat::CompatibilityIdentity::reference();
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    for value in [
        "9223372036854775808",
        "-9223372036854775809",
        "0x10000000000000000",
        "0b102",
        "2e",
        "2e2147483648",
        "2e999",
    ] {
        let arguments = vec![VmValue::String(value.into())];
        assert!(
            evaluate_pure_native_with_compatibility("TOINT", arguments.clone(), &reference)
                .is_err(),
            "{value}",
        );
        assert_eq!(
            evaluate_pure_native_with_compatibility("TOINT", arguments.clone(), &snake),
            Ok(VmValue::Integer(0)),
            "{value}",
        );
        assert_eq!(
            evaluate_pure_native_with_compatibility("ISNUMERIC", arguments.clone(), &snake),
            evaluate_pure_native_with_compatibility("ISNUMERIC", arguments, &reference),
            "{value}",
        );
    }
    for (value, expected) in [
        ("12.99", 12),
        ("0x10", 16),
        ("0b101", 5),
        ("2e3", 2000),
        (" 12", 0),
        ("12x", 0),
        ("", 0),
    ] {
        assert_eq!(
            evaluate_pure_native_with_compatibility(
                "TOINT",
                vec![VmValue::String(value.into())],
                &snake,
            ),
            Ok(VmValue::Integer(expected)),
        );
    }
    assert!(
        evaluate_pure_native_with_compatibility("TOINT", vec![VmValue::Integer(1)], &snake)
            .is_err(),
    );
}

#[test]
fn unchecked_natives_wrap_even_under_the_snake_arithmetic_policy() {
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    for (name, values, expected) in [
        ("UNCHECKED_ADD", vec![i64::MAX, 1], i64::MIN),
        ("UNCHECKED_SUB", vec![i64::MIN, 1], i64::MAX),
        ("UNCHECKED_MUL", vec![i64::MAX, 2], -2),
        ("UNCHECKED_NEG", vec![i64::MIN], i64::MIN),
    ] {
        assert_eq!(
            evaluate_pure_native_with_compatibility(
                name,
                values.into_iter().map(VmValue::Integer).collect(),
                &snake,
            ),
            Ok(VmValue::Integer(expected)),
            "{name}",
        );
    }
}

#[test]
fn unchecked_natives_reject_missing_extra_and_noninteger_arguments_in_both_profiles() {
    for profile in [
        erabasic_compat::CompatibilityProfileId::EmueraEm,
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let compatibility = erabasic_compat::CompatibilityIdentity::for_profile(profile);
        for (name, arity) in [
            ("UNCHECKED_ADD", 2),
            ("UNCHECKED_SUB", 2),
            ("UNCHECKED_MUL", 2),
            ("UNCHECKED_NEG", 1),
        ] {
            let valid = vec![VmValue::Integer(1); arity];
            assert!(
                evaluate_pure_native_with_compatibility(name, valid.clone(), &compatibility)
                    .is_ok()
            );
            for count in (0..arity).chain(std::iter::once(arity + 1)) {
                assert!(
                    evaluate_pure_native_with_compatibility(
                        name,
                        vec![VmValue::Integer(1); count],
                        &compatibility,
                    )
                    .is_err(),
                    "{profile}: {name} accepted {count} arguments",
                );
            }
            for index in 0..arity {
                let mut invalid = valid.clone();
                invalid[index] = VmValue::String("1".into());
                assert!(
                    evaluate_pure_native_with_compatibility(name, invalid, &compatibility).is_err(),
                    "{profile}: {name} accepted string at {index}",
                );
            }
        }
    }
}

#[test]
fn invalid_randdata_does_not_replace_native_rng_state() {
    let mut registry = NativeServiceRegistry {
        random: Some(Arc::new(Mutex::new(Sfmt19937::new(1234)))),
        ..NativeServiceRegistry::default()
    };
    let before = registry.random_values().unwrap();
    let replacement = Sfmt19937::new(4321).era_values();
    for index in [-1, 625, i64::MAX] {
        let mut invalid = replacement.clone();
        invalid[624] = index;
        assert!(registry.set_random_values(&invalid).is_err());
        assert_eq!(registry.random_values().unwrap(), before);
    }
    assert!(registry.set_random_values(&replacement[..624]).is_err());
    assert_eq!(registry.random_values().unwrap(), before);
    registry.set_random_values(&replacement).unwrap();
    assert_eq!(registry.random_values().unwrap(), replacement);
}

#[test]
fn random_native_implements_one_and_two_argument_ranges() {
    let mut native = RandomNative {
        name: "rand".into(),
        state: Arc::new(Mutex::new(Sfmt19937::new(1))),
    };
    let request = |arguments| NativeCallRequest {
        service_key: SymbolKey([0; 16]),
        omitted_arguments: Vec::new(),
        import: RuntimeImport {
            key: SymbolKey([0; 16]),
            namespace: "test".into(),
            name: "rand".into(),
            abi_version: 1,
            parameters: vec![],
            result: None,
        },
        arguments,
        places: Vec::new(),
        implicit_places: BTreeMap::new(),
    };
    let value = native
        .call(request(vec![VmValue::Integer(8)]))
        .unwrap()
        .value
        .unwrap();
    assert!(matches!(value, VmValue::Integer(0..=7)));
    let value = native
        .call(request(vec![VmValue::Integer(27), VmValue::Integer(31)]))
        .unwrap()
        .value
        .unwrap();
    assert!(matches!(value, VmValue::Integer(27..=30)));
    let value = native
        .call(request(vec![
            VmValue::Integer(i64::MIN),
            VmValue::Integer(3),
        ]))
        .unwrap()
        .value
        .unwrap();
    assert!(matches!(value, VmValue::Integer(0..=2)));
    assert!(native.call(request(vec![VmValue::Integer(0)])).is_err());
    assert!(
        native
            .call(request(vec![VmValue::Integer(5), VmValue::Integer(5)]))
            .is_err()
    );
}

#[test]
fn times_native_multiplies_rationally_and_truncates_toward_zero() {
    let target = PlaceDescriptor::default();
    let mut native = CompilerNative {
        name: "times".into(),
        character_width_mode: CharacterWidthModeHandle::default(),
    };
    let ready = native
        .call(NativeCallRequest {
            service_key: SymbolKey([0; 16]),
            omitted_arguments: Vec::new(),
            import: RuntimeImport {
                key: SymbolKey([0; 16]),
                namespace: "test".into(),
                name: "times".into(),
                abi_version: 1,
                parameters: vec![],
                result: None,
            },
            arguments: vec![
                VmValue::IntegerPlace(Box::new(target.clone())),
                VmValue::Integer(3),
                VmValue::Integer(2),
            ],
            places: vec![NativePlaceView {
                argument_index: 0,
                target: target.clone(),
                values: vec![VmValue::Integer(-7)],
            }],
            implicit_places: BTreeMap::new(),
        })
        .expect("valid TIMES call");
    assert_eq!(
        ready.writes,
        vec![HostWrite {
            target,
            value: VmValue::Integer(-10),
        }]
    );
}

#[test]
fn regex_string_natives_match_non_overlapping_reference_semantics() {
    let request = |name: &str, arguments: Vec<VmValue>| NativeCallRequest {
        service_key: SymbolKey([0; 16]),
        omitted_arguments: Vec::new(),
        import: RuntimeImport {
            key: SymbolKey([0; 16]),
            namespace: "test".into(),
            name: name.into(),
            abi_version: 1,
            parameters: vec![],
            result: None,
        },
        arguments,
        places: Vec::new(),
        implicit_places: BTreeMap::new(),
    };
    let mut count = CoreNative::new("strcount".into(), LegacyEncoding::default());
    assert_eq!(
        count
            .call(request(
                "strcount",
                vec![
                    VmValue::String("ababa".into()),
                    VmValue::String("aba".into())
                ],
            ))
            .unwrap()
            .value,
        Some(VmValue::Integer(1))
    );
    assert_eq!(
        count
            .call(request(
                "strcount",
                vec![VmValue::String("aba".into()), VmValue::String("aba".into())],
            ))
            .unwrap()
            .value,
        Some(VmValue::Integer(1))
    );
    assert!(
        count
            .call(request(
                "strcount",
                vec![VmValue::String("text".into()), VmValue::String("[".into())],
            ))
            .is_err()
    );
    let error = count
        .call(request(
            "strcount",
            vec![VmValue::Integer(1), VmValue::String("[".into())],
        ))
        .unwrap_err();
    assert!(
        error
            .message
            .starts_with("STRCOUNT argument 2 is not a regex:")
    );

    let mut escape = CoreNative::new("escape".into(), LegacyEncoding::default());
    assert_eq!(
        escape
            .call(request("escape", vec![VmValue::String("a+b".into())]))
            .unwrap()
            .value,
        Some(VmValue::String("a\\+b".into()))
    );
}

#[test]
fn replace_native_uses_reference_regex_literal_and_array_modes() {
    let request = |arguments: Vec<VmValue>, places: Vec<NativePlaceView>| NativeCallRequest {
        service_key: SymbolKey([0; 16]),
        omitted_arguments: Vec::new(),
        import: RuntimeImport {
            key: SymbolKey([0; 16]),
            namespace: "test".into(),
            name: "replace".into(),
            abi_version: 1,
            parameters: vec![],
            result: None,
        },
        arguments,
        places,
        implicit_places: BTreeMap::new(),
    };
    let mut native = CoreNative::new("replace".into(), LegacyEncoding::default());
    let replace = |native: &mut CoreNative, request| native.call(request).unwrap().value.unwrap();

    assert_eq!(
        replace(
            &mut native,
            request(
                vec![
                    VmValue::String("属性:暴击率加成+[$VALUE:CRITICAL_BONUS]％".into()),
                    VmValue::String(r"\[\$VALUE:CRITICAL_BONUS\]".into()),
                    VmValue::String("4".into()),
                ],
                vec![],
            ),
        ),
        VmValue::String("属性:暴击率加成+4％".into())
    );
    assert_eq!(
        replace(
            &mut native,
            request(
                vec![
                    VmValue::String("<img src='portrait'>".into()),
                    VmValue::String("(<img.+?>)".into()),
                    VmValue::String("[$1]".into()),
                ],
                vec![],
            ),
        ),
        VmValue::String("[<img src='portrait'>]".into())
    );
    assert_eq!(
        replace(
            &mut native,
            request(
                vec![
                    VmValue::String("a+b".into()),
                    VmValue::String("+".into()),
                    VmValue::String("-".into()),
                    VmValue::Integer(2),
                ],
                vec![],
            ),
        ),
        VmValue::String("a-b".into())
    );

    let replacements = PlaceDescriptor::default();
    assert_eq!(
        replace(
            &mut native,
            request(
                vec![
                    VmValue::String("A1 B2 C3".into()),
                    VmValue::String(r"\d".into()),
                    VmValue::StringPlace(Box::new(replacements.clone())),
                    VmValue::Integer(1),
                ],
                vec![NativePlaceView {
                    argument_index: 2,
                    target: replacements,
                    values: vec![VmValue::String("x".into()), VmValue::String("y".into()),],
                }],
            ),
        ),
        VmValue::String("Ax By C".into())
    );
    assert!(
        native
            .call(request(
                vec![
                    VmValue::String("text".into()),
                    VmValue::String("[".into()),
                    VmValue::String("x".into()),
                ],
                vec![],
            ))
            .unwrap_err()
            .message
            .starts_with("REPLACE argument 2 is not a regex:")
    );
}

fn classified_native_request(name: &str, arguments: Vec<VmValue>) -> NativeCallRequest {
    NativeCallRequest {
        service_key: SymbolKey::default(),
        omitted_arguments: Vec::new(),
        import: RuntimeImport {
            key: SymbolKey::default(),
            namespace: "typed-test".into(),
            name: name.into(),
            abi_version: 1,
            parameters: Vec::new(),
            result: None,
        },
        arguments,
        places: Vec::new(),
        implicit_places: BTreeMap::new(),
    }
}

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
fn regex_compilation_capacity_is_uncatchable_even_with_native_legacy_code() {
    let error = regex::RegexBuilder::new(r"\w+")
        .size_limit(0)
        .build()
        .unwrap_err();
    assert!(matches!(error, regex::Error::CompiledTooBig(_)));
    let failure = super::core::regex_failure("STRCOUNT", &error);
    assert_eq!(failure.category, FaultCategory::ResourceLimit);
    assert_eq!(failure.code, VmFaultCode::Native);
    assert!(!failure.is_script());
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
