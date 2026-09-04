use super::*;
pub(super) fn column_request(name: &str, arguments: Vec<VmValue>) -> NativeCallRequest {
    NativeCallRequest {
        service_key: SymbolKey([0; 16]),
        omitted_arguments: Vec::new(),
        import: erabasic_bytecode::RuntimeImport {
            key: SymbolKey([0; 16]),
            namespace: "rustyera.vm".into(),
            name: name.into(),
            abi_version: erabasic_bytecode::NATIVE_ABI_VERSION,
            parameters: arguments.iter().map(VmValue::value_type).collect(),
            result: (name == "dt__column_resolve")
                .then_some(erabasic_bytecode::BytecodeType::String),
        },
        arguments,
        places: Vec::new(),
        implicit_places: BTreeMap::new(),
    }
}

fn table_with_default(value_type: DataType, default_value: Cell) -> StructuredState {
    let mut state = StructuredState::default();
    let mut table = DataTable::new();
    table.columns.push(Column {
        identity: 0,
        name: "value".into(),
        value_type,
        nullable: true,
        default_value,
    });
    state.install_fresh_table("table".into(), table).unwrap();
    state
}

fn resolve_default_ticket(state: &mut StructuredState) -> VmValue {
    state
        .call(
            "dt__column_resolve",
            &column_request(
                "dt__column_resolve",
                vec![
                    VmValue::String("value".into()),
                    VmValue::String("table".into()),
                ],
            ),
        )
        .unwrap()
        .value
        .unwrap()
}

#[test]
fn column_defaults_saturate_numeric_types_without_treating_minimum_as_omitted() {
    for (value_type, minimum, maximum) in [
        (DataType::Int8, i64::from(i8::MIN), i64::from(i8::MAX)),
        (DataType::Int16, i64::from(i16::MIN), i64::from(i16::MAX)),
        (DataType::Int32, i64::from(i32::MIN), i64::from(i32::MAX)),
        (DataType::Int64, i64::MIN, i64::MAX),
    ] {
        let mut state = table_with_default(value_type, Cell::Null);
        let ticket = resolve_default_ticket(&mut state);
        for (input, expected) in [(i64::MIN, minimum), (i64::MAX, maximum)] {
            state
                .call(
                    "dt__column_apply_int",
                    &column_request(
                        "dt__column_apply_int",
                        vec![ticket.clone(), VmValue::Integer(input)],
                    ),
                )
                .unwrap();
            assert_eq!(
                state.data_tables["table"].columns[1].default_value,
                Cell::Integer(expected)
            );
        }
    }
}

#[test]
fn column_option_private_signatures_and_tickets_fail_before_state_changes() {
    let mut state = table_with_default(DataType::Int32, Cell::Integer(7));
    let ticket = resolve_default_ticket(&mut state);
    let original = state.clone();
    let valid = column_request(
        "dt__column_apply_int",
        vec![ticket.clone(), VmValue::Integer(8)],
    );
    let mut invalid = Vec::new();
    let mut request = valid.clone();
    request.import.result = Some(erabasic_bytecode::BytecodeType::Integer);
    invalid.push(request);
    let mut request = valid.clone();
    request.import.namespace = "script".into();
    invalid.push(request);
    let mut request = valid.clone();
    request.import.abi_version = 0;
    invalid.push(request);
    let mut request = valid.clone();
    request.import.parameters.clear();
    invalid.push(request);
    let mut request = valid.clone();
    request.arguments[1] = VmValue::String("8".into());
    invalid.push(request);
    for ticket in [
        "dtc1:0000000000000000:3",
        "dtc1:ffffffffffffffff:3",
        "dtc1:0000000000000002:5",
        "dtc1:000000000000000A:3",
        "bad",
    ] {
        let mut request = valid.clone();
        request.arguments[0] = VmValue::String(ticket.into());
        invalid.push(request);
    }
    for request in invalid {
        assert!(state.call("dt__column_apply_int", &request).is_err());
        assert_eq!(state, original);
    }
    assert!(
        state
            .call(
                "dt__column_check_str",
                &column_request("dt__column_check_str", vec![ticket])
            )
            .is_err()
    );
    assert_eq!(state, original);
}

#[test]
fn column_option_missing_targets_return_the_required_result_write_only() {
    let mut state = table_with_default(DataType::Int32, Cell::Null);
    for (table, column, expected) in [("missing", "value", -1), ("table", "missing", 0)] {
        let mut request = column_request(
            "dt__column_resolve",
            vec![
                VmValue::String(column.into()),
                VmValue::String(table.into()),
            ],
        );
        assert!(
            state.call("dt__column_resolve", &request).is_err(),
            "missing RESULT is not silently ignored"
        );
        request.implicit_places.insert(
            "RESULT".into(),
            NativePlaceView {
                argument_index: usize::MAX,
                target: PlaceDescriptor::default(),
                values: vec![VmValue::Integer(77)],
            },
        );
        let result = state.call("dt__column_resolve", &request).unwrap();
        assert_eq!(result.value, Some(VmValue::String(String::new())));
        assert_eq!(result.writes.len(), 1);
        assert_eq!(result.writes[0].value, VmValue::Integer(expected));
        assert_eq!(result.writes[0].target.indices, vec![0]);
    }
}

#[test]
fn column_ticket_retains_original_type_after_removal_and_replacement() {
    let mut state = table_with_default(DataType::Int8, Cell::Integer(3));
    let ticket = resolve_default_ticket(&mut state);
    let old_identity = state.data_tables["table"].columns[1].identity;
    state.remove_column("table", 1).unwrap();
    state
        .append_fresh_column(
            "table",
            Column {
                identity: 0,
                name: "value".into(),
                value_type: DataType::String,
                nullable: true,
                default_value: Cell::String("replacement".into()),
            },
        )
        .unwrap();
    assert!(state.data_tables["table"].columns[1].identity > old_identity);
    state
        .call(
            "dt__column_check_int",
            &column_request("dt__column_check_int", vec![ticket.clone()]),
        )
        .unwrap();
    state
        .call(
            "dt__column_apply_int",
            &column_request(
                "dt__column_apply_int",
                vec![ticket.clone(), VmValue::Integer(999)],
            ),
        )
        .unwrap();
    assert_eq!(
        state.data_tables["table"].columns[1].default_value,
        Cell::String("replacement".into())
    );
    assert!(
        state
            .call(
                "dt__column_check_str",
                &column_request("dt__column_check_str", vec![ticket])
            )
            .is_err()
    );
}

#[test]
fn structured_bundle_rejects_old_or_corrupt_column_state_and_preserves_valid_identity() {
    let state = table_with_default(DataType::Int8, Cell::Integer(8));
    let bytes = state.encode().unwrap();
    assert_eq!(StructuredState::decode(&bytes).unwrap(), state);
    let mut old = bytes.clone();
    old[..4].copy_from_slice(&2_u32.to_le_bytes());
    assert!(StructuredState::decode(&old).is_err());
    let mut invalid_states = Vec::new();
    let mut invalid = state.clone();
    invalid.data_tables.get_mut("table").unwrap().columns[1].identity = 1;
    invalid_states.push(invalid);
    let mut invalid = state.clone();
    invalid.next_column_identity = 2;
    invalid_states.push(invalid);
    let mut invalid = state.clone();
    invalid.column_identity_revision = 0;
    invalid_states.push(invalid);
    let mut invalid = state.clone();
    invalid.data_tables.get_mut("table").unwrap().columns[1].default_value = Cell::Integer(128);
    invalid_states.push(invalid);
    for invalid in invalid_states {
        let mut bytes = STRUCTURED_BUNDLE_VERSION.to_le_bytes().to_vec();
        bytes.extend(serde_json::to_vec(&invalid).unwrap());
        assert!(StructuredState::decode(&bytes).is_err());
    }
    let mut payload = serde_json::to_value(&state).unwrap();
    payload["data_tables"]["table"]["columns"][1]
        .as_object_mut()
        .unwrap()
        .remove("default_value");
    let mut missing_field = STRUCTURED_BUNDLE_VERSION.to_le_bytes().to_vec();
    missing_field.extend(serde_json::to_vec(&payload).unwrap());
    assert!(StructuredState::decode(&missing_field).is_err());
}

#[test]
fn column_identity_allocator_overflow_does_not_replace_existing_data() {
    let mut state = table_with_default(DataType::String, Cell::Null);
    state.next_column_identity = u64::MAX - 1;
    let before = state.clone();
    assert!(
        state
            .install_fresh_table("table".into(), DataTable::new())
            .is_err()
    );
    assert_eq!(state, before);
    state.column_identity_revision = u64::MAX - 1;
    let before = state.clone();
    assert!(state.remove_table("table").is_err());
    assert_eq!(state, before);
}

#[test]
fn column_default_xml_keeps_unicode_attribute_whitespace_and_explicit_null() {
    // This checks Rust persistence, not a .NET XML-string oracle golden.
    let mut state = table_with_default(DataType::String, Cell::String("爱<&\"\r\n\t".into()));
    let table = state.data_tables.get_mut("table").unwrap();
    table.rows = vec![
        DataRow {
            id: 1,
            cells: vec![Cell::Null, Cell::Null],
        },
        DataRow {
            id: 2,
            cells: vec![Cell::Null, Cell::String(String::new())],
        },
    ];
    table.next_id = 3;
    let schema = data_table_schema_xml("table", table);
    let parsed_schema = parse_data_table_schema("table", &schema).unwrap();
    assert_eq!(
        parsed_schema.columns[1].default_value,
        table.columns[1].default_value
    );
    let data = data_table_data_xml("table", table);
    let parsed = parse_data_table_xml("table", &parsed_schema, &data).unwrap();
    assert_eq!(parsed.rows, table.rows);
    let inherited = parse_data_table_xml(
        "table",
        &parsed_schema,
        "<DocumentElement><table><id>3</id></table></DocumentElement>",
    )
    .unwrap();
    assert_eq!(inherited.rows[0].cells[1], table.columns[1].default_value);
    for xml in [
        "<DocumentElement xmlns:n='http://www.w3.org/2001/XMLSchema-instance'><table><id>3</id><value n:nil='true'/></table></DocumentElement>",
        "<DocumentElement><table xmlns:n='http://www.w3.org/2001/XMLSchema-instance'><id>3</id><value n:nil='1'/></table></DocumentElement>",
    ] {
        assert_eq!(
            parse_data_table_xml("table", &parsed_schema, xml)
                .unwrap()
                .rows[0]
                .cells[1],
            Cell::Null
        );
    }
    let rebound = "<DocumentElement xmlns:n='http://www.w3.org/2001/XMLSchema-instance'><table><id>3</id><value xmlns:n='unrelated' n:nil='true'/></table></DocumentElement>";
    assert_eq!(
        parse_data_table_xml("table", &parsed_schema, rebound)
            .unwrap()
            .rows[0]
            .cells[1],
        Cell::String(String::new())
    );
    assert!(parse_data_table_xml("table", &parsed_schema, "<DocumentElement><table><id>3</id><value>a</value><value>b</value></table></DocumentElement>").is_err());
}

#[test]
fn column_default_xml_rejects_wrong_types_but_accepts_empty_and_minimum_defaults() {
    for (value_type, default) in [
        (DataType::String, Cell::String(String::new())),
        (DataType::Int64, Cell::Integer(i64::MIN)),
    ] {
        let state = table_with_default(value_type, default.clone());
        let table = &state.data_tables["table"];
        let schema = data_table_schema_xml("table", table);
        assert_eq!(
            parse_data_table_schema("table", &schema).unwrap().columns[1].default_value,
            default
        );
    }
    let state = table_with_default(DataType::Int8, Cell::Integer(12));
    let schema = data_table_schema_xml("table", &state.data_tables["table"]);
    for invalid in ["128", "text"] {
        assert!(
            parse_data_table_schema(
                "table",
                &schema.replace("default=\"12\"", &format!("default=\"{invalid}\""))
            )
            .is_err()
        );
    }
}

#[test]
fn imported_data_tables_receive_fresh_column_identities_and_preserve_defaults() {
    let declarations = ExtensionData {
        global_data_tables: BTreeSet::from(["table".into()]),
        ..ExtensionData::default()
    };
    let mut state = table_with_default(DataType::Int32, Cell::Integer(8));
    let ticket = resolve_default_ticket(&mut state);
    let initial_stamp = state.column_identity_stamp();
    let previous_identity = state.data_tables["table"].columns[1].identity;
    let exported = state.export_extensions(&declarations, StructuredScope::Global);
    state
        .import_extensions(&declarations, StructuredScope::Global, &exported)
        .unwrap();
    assert_ne!(state.column_identity_stamp(), initial_stamp);
    assert!(state.data_tables["table"].columns[1].identity > previous_identity);
    state
        .call(
            "dt__column_apply_int",
            &column_request("dt__column_apply_int", vec![ticket, VmValue::Integer(19)]),
        )
        .unwrap();
    assert_eq!(
        state.data_tables["table"].columns[1].default_value,
        Cell::Integer(8)
    );
    let stamp = state.column_identity_stamp();
    state
        .clear_for_transaction(
            &declarations,
            &crate::VmRuntimeStateTransaction::ResetGlobalData,
        )
        .unwrap();
    assert_eq!(state.column_identity_stamp(), stamp);
    assert_eq!(
        state.data_tables["table"].columns[1].default_value,
        Cell::Integer(8)
    );
}

#[test]
fn structured_native_input_parse_is_catchable_but_contract_failure_is_not() {
    let state = Arc::new(Mutex::new(StructuredState::default()));
    let mut native = StructuredNative::new("xml_document", Arc::clone(&state));
    let bad_xml = column_request(
        "xml_document",
        vec![
            VmValue::String("doc".into()),
            VmValue::String("<root>".into()),
        ],
    );
    let failure = crate::NativeService::call(&mut native, bad_xml).unwrap_err();
    assert_eq!(
        failure.category,
        FaultCategory::Script(ScriptFaultKind::Parse)
    );
    assert_eq!(failure.code, VmFaultCode::Native);
    assert!(state.lock().unwrap().xml_documents.is_empty());
    let bad_argument = column_request(
        "xml_document",
        vec![VmValue::String("doc".into()), VmValue::Integer(1)],
    );
    let failure = crate::NativeService::call(&mut native, bad_argument).unwrap_err();
    assert_eq!(failure.category, FaultCategory::HostContract);
    assert_eq!(failure.code, VmFaultCode::Native);
}

#[test]
fn xpath_input_errors_do_not_promote_internal_selection_failures() {
    let document = parse_xml("<root>text<item /></root>").unwrap();
    for expression in ["//root[", "//ns:item", "//root[contains('a')]", "//root |"] {
        let failure = document.select(expression).unwrap_err();
        assert_eq!(
            failure.category,
            FaultCategory::Script(ScriptFaultKind::Parse)
        );
    }
    let failure = document.select("//root/text()").unwrap_err();
    assert_eq!(
        failure.category,
        FaultCategory::Script(ScriptFaultKind::Argument)
    );
    let failure = document.element(&[usize::MAX]).unwrap_err();
    assert_eq!(failure.category, FaultCategory::HostContract);
}

#[test]
fn data_table_script_row_domains_preserve_state_and_shared_validation_is_contract_only() {
    let mut state = table_with_default(DataType::Int8, Cell::Null);
    state.data_tables.get_mut("table").unwrap().columns[1].nullable = false;
    let original = state.clone();
    for arguments in [
        vec![VmValue::String("table".into())],
        vec![
            VmValue::String("table".into()),
            VmValue::String("value".into()),
            VmValue::Integer(256),
        ],
        vec![
            VmValue::String("table".into()),
            VmValue::String("missing".into()),
            VmValue::Integer(1),
        ],
    ] {
        let failure = state
            .call("dt_row_add", &column_request("dt_row_add", arguments))
            .unwrap_err();
        assert_eq!(
            failure.category,
            FaultCategory::Script(ScriptFaultKind::Argument)
        );
        assert_eq!(state, original);
    }
    let failure = super::column_identity::validate_row_cells(
        &state.data_tables["table"],
        &[Cell::Null, Cell::Null],
    )
    .unwrap_err();
    assert_eq!(failure.category, FaultCategory::HostContract);
}

#[test]
fn data_table_fromxml_preserves_local_parse_fallback_without_swallowing_resource_limits() {
    let mut state = StructuredState::default();
    state
        .install_fresh_table("table".into(), DataTable::new())
        .unwrap();
    let original = state.clone();
    let schema = data_table_schema_xml("table", &DataTable::new());
    for (schema_input, data) in [
        ("<bad>".to_owned(), "<DocumentElement />"),
        (
            schema.clone(),
            "<DocumentElement><table><id>bad</id></table></DocumentElement>",
        ),
    ] {
        let result = state
            .call(
                "dt_fromxml",
                &column_request(
                    "dt_fromxml",
                    vec![
                        VmValue::String("table".into()),
                        VmValue::String(schema_input),
                        VmValue::String(data.into()),
                    ],
                ),
            )
            .unwrap();
        assert_eq!(result.value, Some(VmValue::Integer(0)));
        assert_eq!(state, original);
    }
    state.next_column_identity = u64::MAX - 1;
    let exhausted = state.clone();
    let failure = state
        .call(
            "dt_fromxml",
            &column_request(
                "dt_fromxml",
                vec![
                    VmValue::String("table".into()),
                    VmValue::String(schema),
                    VmValue::String("<DocumentElement />".into()),
                ],
            ),
        )
        .unwrap_err();
    assert_eq!(failure.category, FaultCategory::ResourceLimit);
    assert_eq!(state, exhausted);
}

#[test]
fn data_table_select_keeps_filter_false_but_sort_and_result_contract_are_distinct() {
    let mut state = StructuredState::default();
    let mut table = DataTable::new();
    table.rows.push(DataRow {
        id: 1,
        cells: vec![Cell::Null],
    });
    table.next_id = 2;
    state.install_fresh_table("table".into(), table).unwrap();
    let mut request = column_request(
        "dt_select",
        vec![
            VmValue::String("table".into()),
            VmValue::String("id=invalid".into()),
            VmValue::String(String::new()),
        ],
    );
    request.implicit_places.insert(
        "RESULT".into(),
        NativePlaceView {
            argument_index: usize::MAX,
            target: PlaceDescriptor::default(),
            values: vec![VmValue::Integer(77), VmValue::Integer(88)],
        },
    );
    let result = state.call("dt_select", &request).unwrap();
    assert_eq!(result.value, Some(VmValue::Integer(0)));
    assert_eq!(result.writes[0].value, VmValue::Integer(0));
    request.arguments[2] = VmValue::String("missing".into());
    let failure = state.call("dt_select", &request).unwrap_err();
    assert_eq!(
        failure.category,
        FaultCategory::Script(ScriptFaultKind::Argument)
    );
    request.arguments[2] = VmValue::String(String::new());
    request.implicit_places.clear();
    let failure = state.call("dt_select", &request).unwrap_err();
    assert_eq!(failure.category, FaultCategory::HostContract);
}

#[test]
fn data_table_cell_set_only_converts_script_domain_failures_to_minus_two() {
    let mut state = table_with_default(DataType::Int8, Cell::Integer(1));
    state
        .call(
            "dt_row_add",
            &column_request("dt_row_add", vec![VmValue::String("table".into())]),
        )
        .unwrap();
    let mut request = column_request(
        "dt_cell_set",
        vec![
            VmValue::String("table".into()),
            VmValue::Integer(0),
            VmValue::String("value".into()),
            VmValue::Integer(256),
        ],
    );
    let original = state.clone();
    assert_eq!(
        state.call("dt_cell_set", &request).unwrap().value,
        Some(VmValue::Integer(-2))
    );
    request.arguments[3] = VmValue::IntegerPlace(Box::default());
    let failure = state.call("dt_cell_set", &request).unwrap_err();
    assert_eq!(failure.category, FaultCategory::HostContract);
    assert_eq!(state, original);
}
