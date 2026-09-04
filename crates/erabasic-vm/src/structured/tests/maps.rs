use super::*;
pub(super) fn map_request(name: &str, arguments: Vec<VmValue>) -> NativeCallRequest {
    let mut request = column_request(name, arguments);
    request.import.result = Some(
        if matches!(name, "map_values" | "map_findkey" | "map_tostring") {
            erabasic_bytecode::BytecodeType::String
        } else {
            erabasic_bytecode::BytecodeType::Integer
        },
    );
    for (name, values) in [
        ("RESULT", vec![VmValue::Integer(77)]),
        (
            "RESULTS",
            vec![
                VmValue::String("old-first".into()),
                VmValue::String("old-tail".into()),
            ],
        ),
    ] {
        request.implicit_places.insert(
            name.into(),
            NativePlaceView {
                argument_index: usize::MAX,
                target: PlaceDescriptor {
                    variable: SymbolKey::derive("test.map", name.as_bytes()),
                    indices: vec![0],
                    ..PlaceDescriptor::default()
                },
                values,
            },
        );
    }
    request
}

pub(super) fn map_strings(values: &[&str]) -> Vec<VmValue> {
    values
        .iter()
        .map(|value| VmValue::String((*value).into()))
        .collect()
}

pub(super) fn map_with_entries(entries: &[(&str, &str)]) -> StructuredState {
    let mut state = StructuredState::default();
    let mut map = OrderedMap::default();
    for (key, value) in entries {
        map.set((*key).into(), (*value).into());
    }
    state.maps.insert("m".into(), map);
    state
}

#[test]
fn map_merge_snapshots_self_and_preserves_existing_key_positions() {
    let mut state = map_with_entries(&[("b", "old"), ("a", "keep")]);
    let mut source = OrderedMap::default();
    source.set("b".into(), "new".into());
    source.set("c".into(), "added".into());
    state.maps.insert("source".into(), source);
    let ready = map_test_call(
        &mut state,
        "map_merge",
        &map_request("map_merge", map_strings(&["m", "source"])),
    )
    .unwrap();
    assert_eq!(ready.value, Some(VmValue::Integer(1)));
    let expected = vec![
        ("b".into(), "new".into()),
        ("a".into(), "keep".into()),
        ("c".into(), "added".into()),
    ];
    assert_eq!(state.maps["m"].entries, expected);
    map_test_call(
        &mut state,
        "map_merge",
        &map_request("map_merge", map_strings(&["m", "m"])),
    )
    .unwrap();
    assert_eq!(state.maps["m"].entries, expected);
    for names in [["missing", "m"], ["m", "missing"]] {
        let ready = map_test_call(
            &mut state,
            "map_merge",
            &map_request("map_merge", map_strings(&names)),
        )
        .unwrap();
        assert_eq!(ready.value, Some(VmValue::Integer(0)));
        assert_eq!(state.maps["m"].entries, expected);
    }
}

#[test]
fn map_filters_keep_exact_modes_and_findkey_serialized_count() {
    for (mode, needle, expected) in [
        ("KEY_CONTAINS", "a", 2),
        ("KEY_PREFIX", "a", 1),
        ("KEY_SUFFIX", "a", 1),
        ("VAL_CONTAINS", "red", 2),
        ("VAL_EQ", "red", 1),
        ("VAL_NE", "red", 2),
    ] {
        let mut state = map_with_entries(&[("ab", "red"), ("ba", "redder"), ("c", "blue")]);
        let ready = map_test_call(
            &mut state,
            "map_removeif",
            &map_request("map_removeif", map_strings(&["m", needle, mode])),
        )
        .unwrap();
        assert_eq!(ready.value, Some(VmValue::Integer(expected)), "{mode}");
    }
    let mut state = map_with_entries(&[("", "yes"), ("a,b", "yes"), ("tail", "no")]);
    let ready = map_test_call(
        &mut state,
        "map_findkey",
        &map_request("map_findkey", map_strings(&["m", "yes", "VAL_EQ"])),
    )
    .unwrap();
    assert_eq!(ready.value, Some(VmValue::String(",a,b".into())));
    assert_eq!(ready.writes[0].value, VmValue::Integer(3));
    let before = state.clone();
    for mode in ["val_eq", "UNKNOWN"] {
        let ready = map_test_call(
            &mut state,
            "map_removeif",
            &map_request("map_removeif", map_strings(&["m", "yes", mode])),
        )
        .unwrap();
        assert_eq!(ready.value, Some(VmValue::Integer(-1)));
        assert_eq!(state.maps, before.maps);
    }
    for mode in ["VAL_NE", "val_eq", "UNKNOWN"] {
        let ready = map_test_call(
            &mut state,
            "map_findkey",
            &map_request("map_findkey", map_strings(&["m", "yes", mode])),
        )
        .unwrap();
        assert_eq!(ready.value, Some(VmValue::String(String::new())));
        assert_eq!(ready.writes[0].value, VmValue::Integer(0));
    }
    let mut empty_key = map_with_entries(&[("", "yes")]);
    let ready = map_test_call(
        &mut empty_key,
        "map_findkey",
        &map_request("map_findkey", map_strings(&["m", "yes", "VAL_EQ"])),
    )
    .unwrap();
    assert_eq!(ready.value, Some(VmValue::String(String::new())));
    assert_eq!(ready.writes[0].value, VmValue::Integer(0));
}

#[test]
fn map_values_preserves_implicit_first_value_and_truncates_array_writes() {
    let mut state = map_with_entries(&[("b", "one"), ("a", "two"), ("c", "three")]);
    let ready = map_test_call(
        &mut state,
        "map_values",
        &map_request("map_values", map_strings(&["m"])),
    )
    .unwrap();
    assert_eq!(ready.value, Some(VmValue::String("one,two,three".into())));
    assert!(ready.writes.is_empty());
    let mut request = map_request(
        "map_values",
        vec![VmValue::String("m".into()), VmValue::Integer(1)],
    );
    let target = request.implicit_places["RESULTS"].target.variable;
    let ready = map_test_call(&mut state, "map_values", &request).unwrap();
    assert_eq!(ready.value, Some(VmValue::String("one".into())));
    let values = ready
        .writes
        .iter()
        .filter(|write| write.target.variable == target)
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].target.indices, [0]);
    assert_eq!(values[0].value, VmValue::String("one".into()));
    assert_eq!(values[1].target.indices, [1]);
    assert_eq!(values[1].value, VmValue::String("two".into()));
    assert_eq!(ready.writes.last().unwrap().value, VmValue::Integer(3));
    state.maps.get_mut("m").unwrap().entries.clear();
    let ready = map_test_call(&mut state, "map_values", &request).unwrap();
    assert_eq!(ready.value, Some(VmValue::String("old-first".into())));
    assert_eq!(ready.writes.len(), 1);
    assert_eq!(ready.writes[0].value, VmValue::Integer(0));
    request.arguments[1] = VmValue::Integer(0);
    request.implicit_places.clear();
    let ready = map_test_call(&mut state, "map_values", &request).unwrap();
    assert_eq!(ready.value, Some(VmValue::String(String::new())));
    assert!(ready.writes.is_empty());

    let mut state = map_with_entries(&[("b", "one")]);
    let target = PlaceDescriptor {
        variable: SymbolKey::derive("test.map", b"output"),
        indices: vec![0],
        ..PlaceDescriptor::default()
    };
    let mut request = map_request(
        "map_values",
        vec![
            VmValue::String("m".into()),
            VmValue::StringPlace(Box::new(target.clone())),
            VmValue::Integer(1),
        ],
    );
    request.places.push(NativePlaceView {
        argument_index: 1,
        target: target.clone(),
        values: vec![VmValue::String("old".into()); 3],
    });
    let ready = map_test_call(&mut state, "map_values", &request).unwrap();
    assert_eq!(ready.value, Some(VmValue::String(String::new())));
    assert_eq!(
        ready.writes.len(),
        2,
        "one output value and RESULT, with no tail clearing"
    );
    assert_eq!(ready.writes[0].target, target);
    assert_eq!(ready.writes[0].value, VmValue::String("one".into()));
    request.arguments[2] = VmValue::Integer(0);
    request.places.clear();
    request.implicit_places.clear();
    assert!(
        map_test_call(&mut state, "map_values", &request)
            .unwrap()
            .writes
            .is_empty()
    );
}

#[test]
fn map_missing_targets_return_sentinels_without_native_places() {
    let mut state = StructuredState::default();
    for operation in [
        "map_values",
        "map_removeif",
        "map_findkey",
        "map_tostring",
        "map_fromstring",
    ] {
        // This only checks the core's early return; it does not prove script-level lazy evaluation.
        let mut request = map_request(operation, map_strings(&["missing"]));
        request.implicit_places.clear();
        let ready = map_test_call(&mut state, operation, &request).unwrap();
        assert!(ready.writes.is_empty());
        assert_eq!(
            ready.value,
            Some(if matches!(operation, "map_removeif" | "map_fromstring") {
                VmValue::Integer(0)
            } else {
                VmValue::String(String::new())
            })
        );
    }
}

#[test]
fn map_string_conversion_merges_without_escaping_and_counts_duplicate_entries() {
    let mut state = map_with_entries(&[("keep", "old"), ("a", "first")]);
    let ready = map_test_call(
        &mut state,
        "map_fromstring",
        &map_request(
            "map_fromstring",
            map_strings(&["m", "a=1,skip,a=2,=empty,b=x=y,,"]),
        ),
    )
    .unwrap();
    assert_eq!(ready.value, Some(VmValue::Integer(4)));
    assert!(ready.writes.is_empty());
    let ready = map_test_call(
        &mut state,
        "map_tostring",
        &map_request("map_tostring", map_strings(&["m"])),
    )
    .unwrap();
    assert_eq!(
        ready.value,
        Some(VmValue::String("keep=old,a=2,=empty,b=x=y".into()))
    );
    let ready = map_test_call(
        &mut state,
        "map_fromstring",
        &map_request(
            "map_fromstring",
            map_strings(&["m", "a=>new||z=>x=>y", "||", "=>"]),
        ),
    )
    .unwrap();
    assert_eq!(ready.value, Some(VmValue::Integer(2)));
    let ready = map_test_call(
        &mut state,
        "map_tostring",
        &map_request("map_tostring", map_strings(&["m", "||", "=>"])),
    )
    .unwrap();
    assert_eq!(
        ready.value,
        Some(VmValue::String(
            "keep=>old||a=>new||=>empty||b=>x=y||z=>x=>y".into()
        ))
    );
    let before = state.clone();
    let ready = map_test_call(
        &mut state,
        "map_fromstring",
        &map_request("map_fromstring", map_strings(&["m", ""])),
    )
    .unwrap();
    assert_eq!(ready.value, Some(VmValue::Integer(0)));
    assert_eq!(state.maps, before.maps);
    let invalid = map_request(
        "map_fromstring",
        vec![
            VmValue::String("m".into()),
            VmValue::String(String::new()),
            VmValue::Integer(0),
        ],
    );
    assert!(
        map_test_call(&mut state, "map_fromstring", &invalid).is_err(),
        "explicit separators are read before the empty-data return"
    );
    assert_eq!(state.maps, before.maps);
}

#[test]
fn map_extensions_reuse_ordered_bundle_and_global_scope_storage() {
    let mut state = map_with_entries(&[("b", "old"), ("a", "kept")]);
    map_test_call(
        &mut state,
        "map_fromstring",
        &map_request("map_fromstring", map_strings(&["m", "b=new,c=added"])),
    )
    .unwrap();
    let encoded = state.encode().unwrap();
    let decoded = StructuredState::decode(&encoded).unwrap();
    assert_eq!(decoded, state);
    let declarations = ExtensionData {
        global_maps: ["m".to_owned()].into_iter().collect(),
        ..ExtensionData::default()
    };
    let exported = state.export_extensions(&declarations, StructuredScope::Global);
    let mut imported = StructuredState::default();
    imported
        .import_extensions(&declarations, StructuredScope::Global, &exported)
        .unwrap();
    assert_eq!(imported.maps["m"].entries, state.maps["m"].entries);
    assert!(
        state
            .export_extensions(&declarations, StructuredScope::Ordinary)
            .is_empty()
    );
}
