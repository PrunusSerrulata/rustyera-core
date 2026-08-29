use super::*;

#[test]
fn ordered_map_overwrite_keeps_insertion_position() {
    let mut map = OrderedMap::default();
    map.set("b".into(), "1".into());
    map.set("a".into(), "2".into());
    map.set("b".into(), "3".into());
    assert_eq!(
        map.entries,
        vec![("b".into(), "3".into()), ("a".into(), "2".into())]
    );
}

#[test]
fn xml_subset_preserves_mixed_content_and_selects_paths() {
    let document = parse_xml("<root a='x'>A<p><k>one</k></p>B</root>").unwrap();
    assert_eq!(document.root.inner_text(), "AoneB");
    let selection = &document.select("/root/p/k").unwrap()[0];
    assert_eq!(document.selection_value(selection, 1), "one");
    assert_eq!(parse_xml(&document.outer_xml()).unwrap(), document);
    assert_eq!(
        parse_xml("<root>\n  <item />\n</root>")
            .unwrap()
            .outer_xml(),
        "<root><item /></root>"
    );
}

#[test]
fn xpath_subset_handles_descendants_attributes_and_predicates() {
    let document =
        parse_xml("<root><p id='a'><k>one</k></p><group><p id='b'><k>two</k></p></group></root>")
            .unwrap();
    let selected = document.select("//p[@id='b']/k").unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(document.selection_value(&selected[0], 1), "two");
    let attributes = document.select("//p/@id").unwrap();
    assert_eq!(
        attributes
            .iter()
            .map(|selection| document.selection_value(selection, 0))
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(document.select("//p[contains(k, 'o')]").unwrap().len(), 2);
}

#[test]
fn xpath_subset_supports_descendant_existence_predicates() {
    let document = parse_xml(
        "<root><defname id='direct'><modifier /></defname><defname id='nested'><group><modifier /></group></defname><defname id='none'><group /></defname></root>",
    )
    .unwrap();

    let selected = document
        .select("//defname[descendant::modifier]/@id")
        .unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|selection| document.selection_value(selection, 0))
            .collect::<Vec<_>>(),
        ["direct", "nested"]
    );
    assert!(document.select("//ns:defname").is_err());
    assert!(document.select("//defname[child::modifier]").is_err());
}

#[test]
fn erafl_xpath_relative_paths_use_xpath_node_set_equality_rules() {
    // Minimal TALENT.xml and SKILL.xml shapes used by
    // CC_CALC_CHARA_STATUS.ERB and SHOW_INFO_SHOW_SKILL.ERB.
    let document = parse_xml(
        "<root><defname id='201'><flag><ignoreMugglePenalty>TRUE</ignoreMugglePenalty></flag><randomCharaTalent><appearance baseRate='4' /></randomCharaTalent></defname><defname id='208'><flag><ignoreMugglePenalty>FALSE</ignoreMugglePenalty><ignoreMugglePenalty>TRUE</ignoreMugglePenalty></flag><randomCharaTalent><appearance baseRate='' /></randomCharaTalent></defname><defname id='210'><flag><ignoreMugglePenalty>TRUE</ignoreMugglePenalty></flag></defname></root>",
    )
    .unwrap();

    let selected = document
        .select("//defname[flag/ignoreMugglePenalty='TRUE']/@id")
        .unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|selection| document.selection_value(selection, 0))
            .collect::<Vec<_>>(),
        ["201", "208", "210"]
    );
    let selected = document
        .select("//defname[flag/ignoreMugglePenalty!='TRUE']/@id")
        .unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|selection| document.selection_value(selection, 0))
            .collect::<Vec<_>>(),
        ["208"]
    );
    assert_eq!(
        document
            .select("//defname[randomCharaTalent/appearance/@baseRate!='']/@id")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn erafl_xpath_skill_filters_support_multiple_predicates_boolean_logic_and_union() {
    // Minimal SHOW_INFO_SHOW_SKILL.ERB 92-97 and 409-445 input shape.
    let document = parse_xml(
        "<root><defname id='1'><category>BATTLE</category><attributes><li>MAGIC</li></attributes><overrideTargeting>1</overrideTargeting><skillrank>UNIQUE</skillrank></defname><defname id='2'><category>BATTLE</category><attributes><li>MELEE</li></attributes><overrideTargeting>display</overrideTargeting></defname><defname id='3'><category>PASSIVE</category><attributes><li>MAGIC</li></attributes></defname></root>",
    )
    .unwrap();

    let selected = document
        .select(
            "//defname[category[text()='BATTLE']][.//attributes/li[text()='MAGIC']][.//overrideTargeting[text()='1' or text()='display']]/@id | //defname[category[text()='PASSIVE']]/@id",
        )
        .unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|selection| document.selection_value(selection, 0))
            .collect::<Vec<_>>(),
        ["1", "3"]
    );
    let selected = document
        .select("//defname[.//attributes/li[text()='MAGIC'] | .//skillrank[text()='UNIQUE']]/@id")
        .unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|selection| document.selection_value(selection, 0))
            .collect::<Vec<_>>(),
        ["1", "3"]
    );
    assert_eq!(
        document
            .select("//defname[not(category[text()='BATTLE'])]/@id")
            .unwrap()
            .iter()
            .map(|selection| document.selection_value(selection, 0))
            .collect::<Vec<_>>(),
        ["3"]
    );
}

#[test]
fn erafl_xpath_and_unicode_and_union_follow_reference_order() {
    let document = parse_xml(
        "<root><defname id='1' name='爱丽丝'><modifier category='cost' modifiedAt='BASE' /><attributes><li>MAGIC</li></attributes></defname><defname id='2'><category>PASSIVE</category><attributes><li>MAGIC</li></attributes></defname><defname id='3'><category>PASSIVE</category></defname></root>",
    )
    .unwrap();
    let values = |query| {
        document
            .select(query)
            .unwrap()
            .iter()
            .map(|selection| document.selection_value(selection, 0))
            .collect::<Vec<_>>()
    };

    assert_eq!(
        values("//defname[modifier[@category='cost' and @modifiedAt='BASE']]/@id"),
        ["1"]
    );
    assert_eq!(values("//defname[@name='爱丽丝']/@id"), ["1"]);
    assert_eq!(
        values(
            "//defname[category[text()='PASSIVE']]/@id | //defname[.//attributes/li[text()='MAGIC']]/@id"
        ),
        ["1", "2", "3"]
    );
}

#[test]
fn erafl_xpath_kojo_contains_concat_and_numeric_relations_match_reference() {
    let document = parse_xml(
        "<root><command id='1' actionName='ATTACK,ANY' skillDefName='FIRE' /><command id='2' actionName='REST' skillDefName='ANY' /><reqlist><level upto='2' /><level upto='5' /></reqlist><portrait id='42' name='Alice' /></root>",
    )
    .unwrap();
    let values = |query| {
        document
            .select(query)
            .unwrap()
            .iter()
            .map(|selection| document.selection_value(selection, 0))
            .collect::<Vec<_>>()
    };

    assert_eq!(
        values(
            "//command[contains(concat(',',@actionName,','), ',ATTACK,') or contains(concat(',',@actionName,','), ',GROUP_A,') or contains(concat(',',@actionName,','), ',GROUP_B,') or contains(concat(',',@actionName,','), ',ANY,')][contains(concat(',',@skillDefName,','), ',FIRE,') or contains(concat(',',@skillDefName,','), ',ANY,')]/@id"
        ),
        ["1"]
    );
    assert_eq!(values("//reqlist/level[3<=@upto]/@upto"), ["5"]);
    assert_eq!(values("//portrait[@id=42]/@name"), ["Alice"]);
}

#[test]
fn xml_attribute_append_replaces_an_existing_name_at_the_end() {
    let mut element = XmlElement {
        name: "item".into(),
        attributes: vec![
            ("id".into(), "a".into()),
            ("kind".into(), "old".into()),
            ("tail".into(), "kept".into()),
        ],
        children: Vec::new(),
    };

    element.append_attribute("kind".into(), "new".into());

    assert_eq!(
        element.attributes,
        vec![
            ("id".into(), "a".into()),
            ("tail".into(), "kept".into()),
            ("kind".into(), "new".into()),
        ]
    );
    assert_eq!(
        element.outer_xml(),
        "<item id=\"a\" tail=\"kept\" kind=\"new\" />"
    );
}

#[test]
fn deterministic_table_ids_are_monotonic() {
    let table = DataTable::new();
    assert_eq!(table.next_id, 1);
    assert_eq!(table.columns[0].name, "id");
}

#[test]
fn rejected_data_table_rows_do_not_consume_ids() {
    let mut state = StructuredState::default();
    state
        .install_fresh_table("table".into(), DataTable::new())
        .unwrap();
    let request = NativeCallRequest {
        service_key: SymbolKey([0; 16]),
        omitted_arguments: Vec::new(),
        import: erabasic_bytecode::RuntimeImport {
            key: SymbolKey([0; 16]),
            namespace: "test".into(),
            name: "dt_row_add".into(),
            abi_version: 1,
            parameters: Vec::new(),
            result: None,
        },
        arguments: vec![
            VmValue::String("table".into()),
            VmValue::String("missing".into()),
            VmValue::Integer(1),
        ],
        places: Vec::new(),
        implicit_places: BTreeMap::new(),
    };

    assert!(state.call_data_table("dt_row_add", &request).is_err());
    let table = &state.data_tables["table"];
    assert_eq!(table.next_id, 1);
    assert!(table.rows.is_empty());
}

#[test]
fn data_table_xml_matches_reference_dataset_shape_and_round_trips() {
    let mut table = DataTable::new();
    table.columns.extend([
        Column {
            identity: 0,
            default_value: Cell::Null,
            name: "name".into(),
            value_type: DataType::String,
            nullable: true,
        },
        Column {
            identity: 0,
            default_value: Cell::Null,
            name: "score".into(),
            value_type: DataType::Int32,
            nullable: false,
        },
    ]);
    table.rows.push(DataRow {
        id: 1,
        cells: vec![Cell::Null, Cell::String("A&B".into()), Cell::Integer(7)],
    });
    let schema = data_table_schema_xml("table", &table);
    assert_eq!(
        schema,
        concat!(
            "<?xml version=\"1.0\" encoding=\"utf-16\"?>\r\n",
            "<xs:schema id=\"NewDataSet\" xmlns=\"\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:msdata=\"urn:schemas-microsoft-com:xml-msdata\">\r\n",
            "  <xs:element name=\"NewDataSet\" msdata:IsDataSet=\"true\" msdata:MainDataTable=\"table\" msdata:CaseSensitive=\"true\" msdata:UseCurrentLocale=\"true\">\r\n",
            "    <xs:complexType>\r\n",
            "      <xs:choice minOccurs=\"0\" maxOccurs=\"unbounded\">\r\n",
            "        <xs:element name=\"table\" msdata:CaseSensitive=\"True\">\r\n",
            "          <xs:complexType>\r\n",
            "            <xs:sequence>\r\n",
            "              <xs:element name=\"id\" type=\"xs:long\" />\r\n",
            "              <xs:element name=\"name\" type=\"xs:string\" minOccurs=\"0\" />\r\n",
            "              <xs:element name=\"score\" type=\"xs:int\" />\r\n",
            "            </xs:sequence>\r\n",
            "          </xs:complexType>\r\n",
            "        </xs:element>\r\n",
            "      </xs:choice>\r\n",
            "    </xs:complexType>\r\n",
            "    <xs:unique name=\"Constraint1\" msdata:PrimaryKey=\"true\">\r\n",
            "      <xs:selector xpath=\".//table\" />\r\n",
            "      <xs:field xpath=\"id\" />\r\n",
            "    </xs:unique>\r\n",
            "  </xs:element>\r\n",
            "</xs:schema>"
        )
    );
    let data = data_table_data_xml("table", &table);
    assert_eq!(
        data,
        "<DocumentElement>\r\n  <table>\r\n    <id>1</id>\r\n    <name>A&amp;B</name>\r\n    <score>7</score>\r\n  </table>\r\n</DocumentElement>"
    );
    let parsed_schema = parse_data_table_schema("table", &schema).unwrap();
    let parsed = parse_data_table_xml("table", &parsed_schema, &data).unwrap();
    assert_eq!(parsed.columns, table.columns);
    assert_eq!(parsed.rows, table.rows);
}

#[test]
fn data_table_xml_rejects_partial_or_mismatched_input_before_commit() {
    let table = DataTable::new();
    let schema = data_table_schema_xml("table", &table);
    let parsed_schema = parse_data_table_schema("table", &schema).unwrap();
    assert!(parse_data_table_schema("other", &schema).is_err());
    assert!(
        parse_data_table_xml(
            "table",
            &parsed_schema,
            "<DocumentElement><table><id>bad</id></table></DocumentElement>"
        )
        .is_err()
    );
}

#[test]
fn extension_scopes_clear_and_import_without_touching_other_scopes() {
    let declarations = ExtensionData {
        save_maps: BTreeSet::from(["save".into()]),
        global_maps: BTreeSet::from(["global".into()]),
        static_maps: BTreeSet::from(["static".into()]),
        ..ExtensionData::default()
    };
    let mut state = StructuredState::default();
    for key in ["save", "global", "static"] {
        let mut map = OrderedMap::default();
        map.set("key".into(), key.into());
        state.maps.insert(key.into(), map);
    }

    state.clear_for_transaction(
        &declarations,
        &crate::VmRuntimeStateTransaction::ResetNewGame,
    );
    assert!(state.maps["save"].entries.is_empty());
    assert!(!state.maps["global"].entries.is_empty());
    assert!(!state.maps["static"].entries.is_empty());

    state
        .maps
        .get_mut("save")
        .unwrap()
        .set("key".into(), "save".into());
    state.clear_for_transaction(
        &declarations,
        &crate::VmRuntimeStateTransaction::ResetGameData,
    );
    assert!(state.maps["save"].entries.is_empty());

    state.clear_for_transaction(
        &declarations,
        &crate::VmRuntimeStateTransaction::ResetGlobalData,
    );
    assert!(state.maps["global"].entries.is_empty());
    assert!(state.maps["static"].entries.is_empty());

    let imported = state
        .import_extensions(
            &declarations,
            StructuredScope::Ordinary,
            &[
                StructuredExtension::Map {
                    key: "save".into(),
                    entries: vec![("a".into(), "1".into())],
                },
                StructuredExtension::Map {
                    key: "undeclared".into(),
                    entries: vec![("b".into(), "2".into())],
                },
            ],
        )
        .unwrap();
    assert_eq!(imported, BTreeSet::from([(0x20, "save".into())]));
    assert_eq!(state.maps["save"].entries, vec![("a".into(), "1".into())]);
    assert!(!state.maps.contains_key("undeclared"));
}

#[test]
fn data_table_save_extensions_write_reference_xml_and_read_legacy_json() {
    let declarations = ExtensionData {
        save_data_tables: BTreeSet::from(["table".into()]),
        ..ExtensionData::default()
    };
    let mut table = DataTable::new();
    table.columns.push(Column {
        identity: 0,
        default_value: Cell::Null,
        name: "name".into(),
        value_type: DataType::String,
        nullable: true,
    });
    table.rows.push(DataRow {
        id: 4,
        cells: vec![Cell::Null, Cell::String("saved".into())],
    });
    let mut state = StructuredState::default();
    table.next_id = 5;
    state.install_fresh_table("table".into(), table).unwrap();
    let table = state.data_tables["table"].clone();

    let exported = state.export_extensions(&declarations, StructuredScope::Ordinary);
    let StructuredExtension::DataTable { key, schema, data } = &exported[0] else {
        panic!("expected a DataTable extension");
    };
    assert_eq!(key, "table");
    assert!(schema.starts_with("<?xml"));
    assert!(data.starts_with("<DocumentElement>"));

    let mut xml_import = StructuredState::default();
    assert_eq!(
        xml_import
            .import_extensions(&declarations, StructuredScope::Ordinary, &exported)
            .unwrap(),
        BTreeSet::from([(0x22, "table".into())])
    );
    assert_eq!(xml_import.data_tables["table"].columns, table.columns);
    assert_eq!(xml_import.data_tables["table"].rows, table.rows);
    assert_eq!(xml_import.data_tables["table"].next_id, 5);

    let legacy = StructuredExtension::DataTable {
        key: "table".into(),
        schema: r#"[{"name":"id","value_type":"Int64","nullable":false},{"name":"name","value_type":"String","nullable":true}]"#.into(),
        data: r#"{"case_sensitive":true,"next_id":5,"columns":[{"name":"id","value_type":"Int64","nullable":false},{"name":"name","value_type":"String","nullable":true}],"rows":[{"id":4,"cells":["Null",{"String":"saved"}]}]}"#.into(),
    };
    let mut legacy_import = StructuredState::default();
    legacy_import
        .import_extensions(&declarations, StructuredScope::Ordinary, &[legacy])
        .unwrap();
    assert_eq!(legacy_import.data_tables["table"].columns, table.columns);
    assert_eq!(legacy_import.data_tables["table"].rows, table.rows);
    assert_eq!(legacy_import.data_tables["table"].next_id, 5);
}

fn column_request(name: &str, arguments: Vec<VmValue>) -> NativeCallRequest {
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
    state.clear_for_transaction(
        &declarations,
        &crate::VmRuntimeStateTransaction::ResetGlobalData,
    );
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
