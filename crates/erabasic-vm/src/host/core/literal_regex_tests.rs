use super::*;

fn request(name: &str, arguments: Vec<VmValue>) -> NativeCallRequest {
    NativeCallRequest {
        service_key: SymbolKey::default(),
        omitted_arguments: Vec::new(),
        import: RuntimeImport {
            key: SymbolKey::default(),
            namespace: "test".into(),
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

fn strings(values: &[&str]) -> Vec<VmValue> {
    values
        .iter()
        .map(|value| VmValue::String((*value).into()))
        .collect()
}

#[test]
fn short_literal_guard_is_conservative_at_syntax_and_size_boundaries() {
    for byte in 0u8..=127 {
        let pattern = char::from(byte).to_string();
        let expected = !b"\\.^$|?*+()[]{}".contains(&byte);
        assert_eq!(is_short_literal_pattern(&pattern), expected, "{pattern:?}");
        if expected {
            assert!(crate::regex_compat::build(&pattern).is_ok(), "{pattern:?}");
        }
    }
    assert!(!is_short_literal_pattern(""));
    assert!(is_short_literal_pattern(&"x".repeat(256)));
    assert!(!is_short_literal_pattern(&"x".repeat(257)));
    assert!(is_short_literal_pattern(&"界".repeat(85)));
    assert!(!is_short_literal_pattern(&"界".repeat(86)));
}

#[test]
fn literal_count_matches_regex_without_populating_compiled_cache() {
    let mut native = CoreNative::new("strcount".into(), LegacyEncoding::default());
    let mut patterns: Vec<String> = (0u8..=127)
        .map(|byte| char::from(byte).to_string())
        .filter(|pattern| is_short_literal_pattern(pattern))
        .collect();
    patterns.extend(["aa", "aba", "界", "😀", "e\u{301}", "\r\n", "标签/"].map(str::to_owned));
    patterns.push("x".repeat(256));
    for pattern in patterns {
        let regex = crate::regex_compat::build(&pattern).unwrap();
        for input in [
            String::new(),
            pattern.clone(),
            pattern.repeat(3),
            format!("前{pattern}后{pattern}尾"),
            "aaaaababa界😀e\u{301}\r\n".into(),
        ] {
            let expected = regex.find_iter(&input).map(Result::unwrap).count();
            let actual = native
                .call(request("strcount", strings(&[&input, &pattern])))
                .unwrap();
            assert_eq!(
                actual.value,
                Some(VmValue::Integer(i64::try_from(expected).unwrap())),
                "pattern={pattern:?}, input={input:?}"
            );
            assert!(actual.writes.is_empty());
        }
    }
    assert!(native.regex_cache.entries.is_empty());
}

#[test]
fn literal_replacement_matches_regex_and_retains_dollar_expansion_fallback() {
    for pattern in ["a", "aa", "aba", "界", "😀", "\r\n", "e\u{301}"] {
        let regex = crate::regex_compat::build(pattern).unwrap();
        for replacement in ["", "X", "界😀", "\\", "$0", "$$", "$1", "${missing}"] {
            let mut native = CoreNative::new("replace".into(), LegacyEncoding::default());
            for input in [
                String::new(),
                pattern.repeat(3),
                format!("前{pattern}后{pattern}"),
                "aaaaababa界😀\r\n".into(),
            ] {
                let expected = regex
                    .try_replacen(&input, 0, replacement)
                    .unwrap()
                    .into_owned();
                for mode in [None, Some(0), Some(-1), Some(3)] {
                    let mut args = strings(&[&input, pattern, replacement]);
                    args.extend(mode.map(VmValue::Integer));
                    let actual = native.call(request("replace", args)).unwrap();
                    assert_eq!(
                        actual.value,
                        Some(VmValue::String(expected.clone())),
                        "pattern={pattern:?}, input={input:?}, replacement={replacement:?}"
                    );
                    assert!(actual.writes.is_empty());
                }
            }
            assert_eq!(
                native.regex_cache.entries.is_empty(),
                !replacement.contains('$')
            );
        }
    }
}

#[test]
fn count_regex_fallback_and_argument_error_priority_remain_unchanged() {
    for pattern in ["", "a+", "[ab]", "(?i)a", "(?<=x)a", r"a\."] {
        let mut count = CoreNative::new("strcount".into(), LegacyEncoding::default());
        let regex = crate::regex_compat::build(pattern).unwrap();
        let input = "xa. aAa ab 😀";
        assert_eq!(
            count
                .call(request("strcount", strings(&[input, pattern])))
                .unwrap()
                .value,
            Some(VmValue::Integer(
                i64::try_from(regex.find_iter(input).map(Result::unwrap).count()).unwrap()
            ))
        );
        assert_eq!(count.regex_cache.entries.len(), 1);
    }
    let mut count = CoreNative::new("strcount".into(), LegacyEncoding::default());
    for pattern in ["[", r"(a)\1", "a{1000000000}"] {
        let expected = regex_compile_failure(
            "STRCOUNT",
            &crate::regex_compat::build(pattern).unwrap_err(),
        );
        let args = vec![VmValue::Integer(0), VmValue::String(pattern.into())];
        assert_eq!(count.call(request("strcount", args)).unwrap_err(), expected);
    }
    assert_eq!(
        count
            .call(request(
                "strcount",
                vec![VmValue::Integer(0), VmValue::String("a".into())]
            ))
            .unwrap_err(),
        native_contract_failure("strcount argument 1 must be string")
    );
}

#[test]
fn replacement_regex_fallback_and_argument_error_priority_remain_unchanged() {
    for pattern in ["", "a+", "[ab]", "(?i)a", "(?<=x)a", r"a\."] {
        let mut replace = CoreNative::new("replace".into(), LegacyEncoding::default());
        let regex = crate::regex_compat::build(pattern).unwrap();
        let input = "xa. aAa ab 😀";
        assert_eq!(
            replace
                .call(request("replace", strings(&[input, pattern, "X"])))
                .unwrap()
                .value,
            Some(VmValue::String(
                regex.try_replacen(input, 0, "X").unwrap().into_owned()
            ))
        );
        assert_eq!(replace.regex_cache.entries.len(), 1);
    }
    let mut replace = CoreNative::new("replace".into(), LegacyEncoding::default());
    for args in [
        vec![VmValue::Integer(0)],
        vec![VmValue::Integer(0), VmValue::String("[".into())],
    ] {
        assert_eq!(
            replace.call(request("replace", args)).unwrap_err(),
            native_contract_failure("REPLACE argument 1 must be string")
        );
    }
    let args = vec![
        VmValue::String("a".into()),
        VmValue::String("[".into()),
        VmValue::Integer(0),
    ];
    assert_eq!(
        replace.call(request("replace", args)).unwrap_err(),
        native_contract_failure("REPLACE argument 3 must be string")
    );
    let mut args = strings(&["a", "a", "X"]);
    args.push(VmValue::String("invalid".into()));
    assert_eq!(
        replace.call(request("replace", args)).unwrap_err(),
        native_contract_failure("REPLACE argument 4 must be integer")
    );
}

#[test]
fn count_literal_length_boundaries_match_regex_and_select_the_bounded_path() {
    for pattern in [
        "x".repeat(256),
        "x".repeat(257),
        "😀".repeat(64),
        "😀".repeat(64) + "a",
    ] {
        let mut native = CoreNative::new("strcount".into(), LegacyEncoding::default());
        let regex = crate::regex_compat::build(&pattern).unwrap();
        let input = format!("前{pattern}{pattern}后");
        let expected = regex.find_iter(&input).map(Result::unwrap).count();
        let actual = native
            .call(request("strcount", strings(&[&input, &pattern])))
            .unwrap();
        assert_eq!(
            actual.value,
            Some(VmValue::Integer(i64::try_from(expected).unwrap()))
        );
        assert!(actual.writes.is_empty());
        assert_eq!(native.regex_cache.entries.is_empty(), pattern.len() <= 256);
    }
}

#[test]
fn replacement_literal_length_boundaries_match_regex_and_select_the_bounded_path() {
    for pattern in [
        "x".repeat(256),
        "x".repeat(257),
        "😀".repeat(64),
        "😀".repeat(64) + "a",
    ] {
        let mut native = CoreNative::new("replace".into(), LegacyEncoding::default());
        let regex = crate::regex_compat::build(&pattern).unwrap();
        let input = format!("前{pattern}{pattern}后");
        let expected = regex.try_replacen(&input, 0, "替換").unwrap().into_owned();
        let actual = native
            .call(request("replace", strings(&[&input, &pattern, "替換"])))
            .unwrap();
        assert_eq!(actual.value, Some(VmValue::String(expected)));
        assert!(actual.writes.is_empty());
        assert_eq!(native.regex_cache.entries.is_empty(), pattern.len() <= 256);
    }
}
