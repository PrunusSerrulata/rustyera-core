use super::*;
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
fn strcount_supports_snake_name_predicate_lookahead() {
    let request = |input: &str, pattern: &str| NativeCallRequest {
        service_key: SymbolKey([0; 16]),
        omitted_arguments: Vec::new(),
        import: RuntimeImport {
            key: SymbolKey([0; 16]),
            namespace: "test".into(),
            name: "strcount".into(),
            abi_version: 1,
            parameters: vec![],
            result: None,
        },
        arguments: vec![
            VmValue::String(input.into()),
            VmValue::String(pattern.into()),
        ],
        places: Vec::new(),
        implicit_places: BTreeMap::new(),
    };
    let snake_names = concat!(
        r"(?i)(?=.*\b浊酒\b).*$|",
        r"(?=.*\bNULL\b).*$|",
        r"(?=.*\b灵梦\b).*$"
    );
    let mut count = CoreNative::new("strcount".into(), LegacyEncoding::default());
    for (input, pattern, expected) in [
        ("喝 浊酒", snake_names, 1),
        ("reimu", r"(?i)(?=.*\bREIMU\b).*$", 1),
        ("ordinary text", snake_names, 0),
    ] {
        assert_eq!(
            count.call(request(input, pattern)).unwrap().value,
            Some(VmValue::Integer(expected))
        );
    }
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

pub(super) fn classified_native_request(name: &str, arguments: Vec<VmValue>) -> NativeCallRequest {
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
