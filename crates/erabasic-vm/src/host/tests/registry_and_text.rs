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
fn non_u_strfind_returns_offsets_accepted_by_non_u_substring() {
    let input = "【HPH小腹HPH】";
    let first = strfind_legacy_bytes(input, "HPH", 0, LegacyEncoding::ChineseHans);
    assert_eq!(first, 2);
    assert_eq!(
        substring_legacy_bytes(input, first, Some(3), LegacyEncoding::ChineseHans),
        "HPH"
    );

    let remainder = substring_legacy_bytes(input, first + 3, None, LegacyEncoding::ChineseHans);
    let second = strfind_legacy_bytes(&remainder, "HPH", 0, LegacyEncoding::ChineseHans);
    assert_eq!(second, 4);
    assert_eq!(
        substring_legacy_bytes(&remainder, second, Some(3), LegacyEncoding::ChineseHans),
        "HPH"
    );

    for start in [1, 2] {
        assert_eq!(
            strfind_legacy_bytes("界HPH", "HPH", start, LegacyEncoding::ChineseHans),
            2,
            "a start within or at the end of a multibyte character advances past it"
        );
    }
    assert_eq!(
        strfind_legacy_bytes("界HPH", "HPH", 3, LegacyEncoding::ChineseHans),
        -1,
        "a start after the first marker character must not search backwards"
    );

    for encoding in [
        LegacyEncoding::ChineseHans,
        LegacyEncoding::ChineseHant,
        LegacyEncoding::Japanese,
        LegacyEncoding::Korean,
    ] {
        let input = "A界B";
        let total = i64::try_from(encoding.encoded_len(input)).expect("short fixture length");
        assert_eq!(strfind_legacy_bytes(input, "", -1, encoding), -1);
        assert_eq!(substring_legacy_bytes(input, -1, Some(1), encoding), "A");
        assert_eq!(strfind_legacy_bytes(input, "B", 1, encoding), 3);
        assert_eq!(substring_legacy_bytes(input, 3, Some(1), encoding), "B");
        assert_eq!(strfind_legacy_bytes(input, "", total, encoding), -1);
        assert_eq!(substring_legacy_bytes(input, total, None, encoding), "");
    }
}

#[test]
fn legacy_text_shortcuts_preserve_boundaries_and_empty_results() {
    fn boundary(value: &str, index: usize, encoding: LegacyEncoding) -> usize {
        if index == 0 {
            return 0;
        }
        let mut consumed = 0;
        value
            .char_indices()
            .find_map(|(offset, character)| {
                consumed += encoding.encoded_char_len(character);
                (consumed >= index).then_some(offset + character.len_utf8())
            })
            .unwrap_or(value.len())
    }

    for encoding in [
        LegacyEncoding::Japanese,
        LegacyEncoding::Korean,
        LegacyEncoding::ChineseHans,
        LegacyEncoding::ChineseHant,
    ] {
        for value in ["", "abc", "界", "A界B", "😀界ｱé", "TYPE:道具"] {
            let total = encoding.encoded_len(value);
            for start in [-1, 0, 1, 2, 3, 4, 5, 6, 9, i64::MAX] {
                let index = usize::try_from(start.max(0)).unwrap_or(usize::MAX);
                let byte_start = boundary(value, index, encoding);
                for length in [
                    None,
                    Some(-1),
                    Some(0),
                    Some(1),
                    Some(2),
                    Some(5),
                    Some(i64::MAX),
                ] {
                    let expected = if index >= total || length == Some(0) {
                        String::new()
                    } else {
                        let requested = length
                            .and_then(|n| usize::try_from(n).ok())
                            .filter(|n| *n <= total)
                            .unwrap_or(total);
                        let end = boundary(&value[byte_start..], requested, encoding);
                        value[byte_start..byte_start + end].to_owned()
                    };
                    assert_eq!(
                        substring_legacy_bytes(value, start, length, encoding),
                        expected,
                        "substring {encoding:?} {value:?} {start} {length:?}"
                    );
                }
                for needle in ["", "a", "界", "B", "missing", "TYPE:", "😀"] {
                    let expected = if start < 0 || index >= total {
                        -1
                    } else {
                        value[byte_start..].find(needle).map_or(-1, |offset| {
                            i64::try_from(encoding.encoded_len(&value[..byte_start + offset]))
                                .unwrap()
                        })
                    };
                    assert_eq!(
                        strfind_legacy_bytes(value, needle, start, encoding),
                        expected,
                        "find {encoding:?} {value:?} {needle:?} {start}"
                    );
                }
            }
        }
    }
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
