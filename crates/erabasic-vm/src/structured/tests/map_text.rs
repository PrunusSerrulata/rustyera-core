use super::*;
#[test]
fn map_fixed_culture_facade_retains_combining_zero_and_utf16_offsets() {
    use crate::compat_text::{SearchMatch, TextBudget, map_first_match, map_prefix, map_suffix};
    let mut budget = TextBudget::new(100_000, 1_000_000);
    assert!(!map_prefix("o\u{308}", "o", &mut budget).unwrap());
    assert!(map_prefix("o\0\u{308}", "o", &mut budget).unwrap());
    assert!(map_suffix("o\u{308}", "\u{308}", &mut budget).unwrap());
    assert_eq!(
        map_first_match("😀=x", "=", &mut budget).unwrap(),
        Some(SearchMatch {
            start_utf16: 2,
            limit_utf16: 3
        })
    );
    assert_eq!(
        map_first_match("éX", "e\u{301}", &mut budget).unwrap(),
        Some(SearchMatch {
            start_utf16: 0,
            limit_utf16: 1
        })
    );
    assert_eq!(
        map_first_match("abc", "\0", &mut budget).unwrap(),
        Some(SearchMatch {
            start_utf16: 0,
            limit_utf16: 0
        })
    );
    assert_eq!(map_first_match("a\u{301}", "a", &mut budget).unwrap(), None);
}

#[test]
fn map_utf16_substring_rejects_unrepresentable_result_without_lossy_decode() {
    use crate::compat_text::{TextBudget, TextError, map_entry_at_utf16_index};
    let mut budget = TextBudget::new(100_000, 1_000_000);
    assert_eq!(
        map_entry_at_utf16_index("😀=x", "=", 2, &mut budget),
        Ok(("😀".into(), "x".into()))
    );
    assert_eq!(
        map_entry_at_utf16_index("éX", "e\u{301}", 0, &mut budget),
        Ok((String::new(), String::new()))
    );
    assert_eq!(
        map_entry_at_utf16_index("é", "e\u{301}", 0, &mut budget),
        Err(TextError::SubstringOutOfRange)
    );
    assert_eq!(
        map_entry_at_utf16_index("a😀", "==", 0, &mut budget),
        Err(TextError::UnsupportedUtf16Substring)
    );
}

#[test]
fn map_fromstring_keeps_prior_entries_on_linguistic_length_and_surrogate_error() {
    for (input, separator) in [("éX,é", "e\u{301}"), ("a,😀", "\0")] {
        let mut state = map_with_entries(&[("keep", "old")]);
        let request = map_request("map_fromstring", map_strings(&["m", input, ",", separator]));
        let failure = map_test_call(&mut state, "map_fromstring", &request).unwrap_err();
        assert_eq!(
            failure.category,
            crate::FaultCategory::Script(crate::ScriptFaultKind::Argument)
        );
        assert_eq!(
            state.maps["m"].entries,
            [
                ("keep".into(), "old".into()),
                (String::new(), String::new())
            ]
        );
        assert!(state.all_map_leases().is_empty());
    }
}

#[test]
fn map_fromstring_empty_separators_preserve_split_and_duplicate_rules() {
    let mut state = map_with_entries(&[("keep", "old")]);
    let ready = map_test_call(
        &mut state,
        "map_fromstring",
        &map_request("map_fromstring", map_strings(&["m", "a=x,b=y", ""])),
    )
    .unwrap();
    assert_eq!(ready.value, Some(VmValue::Integer(1)));
    assert_eq!(state.maps["m"].entries[1], ("a".into(), "x,b=y".into()));
    let ready = map_test_call(
        &mut state,
        "map_fromstring",
        &map_request(
            "map_fromstring",
            map_strings(&["m", "first,second", ",", ""]),
        ),
    )
    .unwrap();
    assert_eq!(ready.value, Some(VmValue::Integer(2)));
    assert_eq!(state.maps["m"].entries[2], (String::new(), "second".into()));
}

fn map_test_budget_call(
    state: &mut StructuredState,
    request: &NativeCallRequest,
    budget: &mut crate::compat_text::TextBudget,
) -> Result<NativeReady, ExecutionFailure> {
    let kind = MapOperation::from_name(&request.import.name).unwrap();
    let lease = state.capture_map("m", map_owner(1)).unwrap().unwrap();
    let result = state.call_leased_map(kind, lease, request, budget);
    state.release_map_lease(lease).unwrap();
    result
}

#[test]
fn map_comparison_budget_is_cumulative_and_keeps_fromstring_partial_commit() {
    use crate::compat_text::TextBudget;
    let mut state = map_with_entries(&[("keep", "old")]);
    let request = map_request("map_fromstring", map_strings(&["m", "a,b", ",", ""]));
    // Empty kvSep avoids CE work. Ten units cover split + first commit, but
    // not the complete second lookup; resetting per-entry would wrongly pass.
    let failure = map_test_budget_call(&mut state, &request, &mut TextBudget::new(10, 1_000_000))
        .unwrap_err();
    assert_eq!(failure.category, crate::FaultCategory::ResourceLimit);
    assert_eq!(
        state.maps["m"].entries,
        [("keep".into(), "old".into()), (String::new(), "a".into())]
    );
    let mut state = map_with_entries(&[("a", "1"), ("b", "2")]);
    let request = map_request("map_removeif", map_strings(&["m", "", "KEY_PREFIX"]));
    let failure =
        map_test_budget_call(&mut state, &request, &mut TextBudget::new(1, 1_000_000)).unwrap_err();
    assert_eq!(failure.category, crate::FaultCategory::ResourceLimit);
    assert_eq!(
        state.maps["m"].entries,
        [("a".into(), "1".into()), ("b".into(), "2".into())]
    );
    let request = map_request("map_removeif", map_strings(&["m", "a", "KEY_CONTAINS"]));
    assert_eq!(
        map_test_budget_call(&mut state, &request, &mut TextBudget::new(0, 0))
            .unwrap()
            .value,
        Some(VmValue::Integer(1)),
        "ordinary ordinal mode has no new budget semantics"
    );
}

#[test]
fn fixed_comparison_failures_do_not_become_script_false() {
    use crate::compat_collation::ce::CeError;
    use crate::compat_text::{TextBudget, TextError, map_prefix};
    assert_eq!(
        map_prefix("a", "a", &mut TextBudget::new(0, 1_000_000)),
        Err(TextError::Collation(CeError::WorkLimit))
    );
    assert_eq!(
        map_prefix("a", "a", &mut TextBudget::new(100, 0)),
        Err(TextError::Collation(CeError::ByteLimit))
    );
    for error in [
        CeError::WorkLimit,
        CeError::ByteLimit,
        CeError::Allocation,
        CeError::InputLimit,
        CeError::ElementLimit,
        CeError::ContextLimit,
    ] {
        assert_eq!(
            TextError::from(error).failure().category,
            crate::FaultCategory::ResourceLimit
        );
    }
    assert_eq!(
        TextError::from(CeError::MalformedProvider)
            .failure()
            .category,
        crate::FaultCategory::InternalInvariant
    );
    assert_eq!(
        TextError::InvalidElementOffsets.failure().category,
        crate::FaultCategory::InternalInvariant
    );
}
