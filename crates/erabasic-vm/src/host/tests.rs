use super::*;

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
fn strform_preserves_plain_runtime_text_and_rejects_unimplemented_expansion() {
    assert_eq!(
        evaluate_pure_native("STRFORM", vec![VmValue::String("plain text".into())]),
        Ok(VmValue::String("plain text".into()))
    );
    assert_eq!(
        evaluate_pure_native("STRFORM", vec![VmValue::String("%RESULTS%".into())]),
        Err("STRFORM runtime expansion is not yet supported for FORM metacharacters".into())
    );
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
fn random_native_implements_one_and_two_argument_ranges() {
    let mut native = RandomNative {
        name: "rand".into(),
        state: Arc::new(Mutex::new(Sfmt19937::new(1))),
    };
    let request = |arguments| NativeCallRequest {
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
    let mut count = CoreNative {
        name: "strcount".into(),
        legacy_encoding: LegacyEncoding::default(),
    };
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
    let mut escape = CoreNative {
        name: "escape".into(),
        legacy_encoding: LegacyEncoding::default(),
    };
    assert_eq!(
        escape
            .call(request("escape", vec![VmValue::String("a+b".into())]))
            .unwrap()
            .value,
        Some(VmValue::String("a\\+b".into()))
    );
}
