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

    state
        .clear_for_transaction(
            &declarations,
            &crate::VmRuntimeStateTransaction::ResetNewGame,
        )
        .unwrap();
    assert!(state.maps["save"].entries.is_empty());
    assert!(!state.maps["global"].entries.is_empty());
    assert!(!state.maps["static"].entries.is_empty());

    state
        .maps
        .get_mut("save")
        .unwrap()
        .set("key".into(), "save".into());
    state
        .clear_for_transaction(
            &declarations,
            &crate::VmRuntimeStateTransaction::ResetGameData,
        )
        .unwrap();
    assert!(state.maps["save"].entries.is_empty());

    state
        .clear_for_transaction(
            &declarations,
            &crate::VmRuntimeStateTransaction::ResetGlobalData,
        )
        .unwrap();
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

fn map_request(name: &str, arguments: Vec<VmValue>) -> NativeCallRequest {
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

fn map_strings(values: &[&str]) -> Vec<VmValue> {
    values
        .iter()
        .map(|value| VmValue::String((*value).into()))
        .collect()
}

fn map_with_entries(entries: &[(&str, &str)]) -> StructuredState {
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

fn map_owner(slot: usize) -> MapLeaseOwner {
    MapLeaseOwner {
        fiber: crate::FiberId(1),
        frame: crate::FrameId(1),
        generation: crate::GenerationId(1),
        function: SymbolKey::derive("test.map", b"owner"),
        origin: MapLeaseOrigin::Bytecode { begin: slot },
    }
}
fn map_test_call(
    state: &mut StructuredState,
    name: &str,
    request: &NativeCallRequest,
) -> Result<NativeReady, ExecutionFailure> {
    let Some(operation) = MapOperation::from_name(name) else {
        return state.call(name, request);
    };
    let Some(lease) = state.capture_map(string_argument(request, 0)?, map_owner(1))? else {
        return Ok(NativeReady::value(VmValue::default_for(
            operation.result_type(),
        )));
    };
    let result = state.call_leased_map(
        operation,
        lease,
        request,
        &mut crate::compat_text::TextBudget::new(1_000_000, 1_000_000_000),
    );
    state.release_map_lease(lease)?;
    result
}

#[test]
fn map_capture_survives_release_recreate_and_snapshot_without_aliasing() {
    let mut state = map_with_entries(&[("a", "old")]);
    let first = state.capture_map("m", map_owner(1)).unwrap().unwrap();
    state.retire_map_binding("m");
    state.maps.insert("m".into(), OrderedMap::default());
    let second = state.capture_map("m", map_owner(2)).unwrap().unwrap();
    state
        .leased_map_mut(second)
        .unwrap()
        .set("a".into(), "new".into());
    state
        .leased_map_mut(first)
        .unwrap()
        .set("b".into(), "detached".into());
    assert_eq!(state.maps["m"].entries, vec![("a".into(), "new".into())]);
    let decoded = StructuredState::decode(&state.encode().unwrap()).unwrap();
    assert_eq!(
        decoded.leased_map(first).unwrap().entries,
        vec![("a".into(), "old".into()), ("b".into(), "detached".into())]
    );
    decoded
        .validate_map_lease_owners(&[first, second].into_iter().collect())
        .unwrap();
    assert!(
        decoded
            .validate_map_lease_owners(&[second].into_iter().collect())
            .is_err()
    );
    state.release_map_lease(first).unwrap();
    assert!(state.leased_map(first).is_err());
    state.release_map_lease(second).unwrap();
    assert!(state.all_map_leases().is_empty());
    assert_eq!(state.maps["m"].entries, vec![("a".into(), "new".into())]);
}

#[test]
fn map_reachability_releases_only_abandoned_captures() {
    let mut state = map_with_entries(&[("a", "old")]);
    let outer = state.capture_map("m", map_owner(1)).unwrap().unwrap();
    let inner = state.capture_map("m", map_owner(2)).unwrap().unwrap();
    state.retire_map_binding("m");
    state
        .retain_map_leases(&[outer].into_iter().collect())
        .unwrap();
    assert!(state.leased_map(inner).is_err());
    assert_eq!(state.leased_map(outer).unwrap().entries.len(), 1);
    state.retain_map_leases(&BTreeSet::new()).unwrap();
    assert!(state.all_map_leases().is_empty());
}

#[test]
fn map_reset_and_import_retire_captured_identity_without_aliasing() {
    let mut state = map_with_entries(&[("a", "old")]);
    let lease = state.capture_map("m", map_owner(1)).unwrap().unwrap();
    let declarations = ExtensionData {
        save_maps: ["m".into()].into_iter().collect(),
        ..ExtensionData::default()
    };
    state
        .clear_for_transaction(
            &declarations,
            &crate::VmRuntimeStateTransaction::ResetNewGame,
        )
        .unwrap();
    assert!(state.maps["m"].entries.is_empty());
    assert_eq!(
        state.leased_map(lease).unwrap().entries,
        vec![("a".into(), "old".into())]
    );
    state
        .import_extensions(
            &declarations,
            StructuredScope::Ordinary,
            &[StructuredExtension::Map {
                key: "m".into(),
                entries: vec![("b".into(), "new".into())],
            }],
        )
        .unwrap();
    assert_eq!(state.maps["m"].entries, vec![("b".into(), "new".into())]);
    assert_eq!(
        state.leased_map(lease).unwrap().entries,
        vec![("a".into(), "old".into())]
    );
}

#[test]
fn map_revision_exhaustion_rejects_stamp_and_batch_reclaim_atomically() {
    let mut state = map_with_entries(&[("a", "old")]);
    let first = state.capture_map("m", map_owner(1)).unwrap().unwrap();
    let second = state.capture_map("m", map_owner(2)).unwrap().unwrap();
    state.map_leases.revision = u64::MAX;
    assert!(state.map_lease_stamp().is_err());
    assert!(state.retain_map_leases(&BTreeSet::new()).is_err());
    assert_eq!(
        state.all_map_leases(),
        [first, second].into_iter().collect()
    );
    assert!(state.leased_map(first).is_ok());
    assert!(state.leased_map(second).is_ok());
}

mod icu72_raw_ce_candidates {
    use crate::compat_collation::{
        FixedIcu72Root,
        ce::{CeError, CeLimits},
        raw_off::{RawRootData, raw_off_elements},
    };
    use zerovec::{ZeroSlice, ZeroVec};

    #[derive(Clone, Copy)]
    enum Mapping {
        Plain,
        Expansion,
        Contraction,
        Discontiguous,
    }
    struct Data {
        mapping: Mapping,
        contexts: ZeroVec<'static, u16>,
    }
    impl Data {
        fn new(mapping: Mapping) -> Self {
            // UCharsTrie one-unit linear match ('b'), final 32-bit CE32.
            // Layout matches ICU72 root_standard_data context examples:
            // default high/low, 0x30, char, 0xffff, result high/low.
            let suffix = if matches!(mapping, Mapping::Discontiguous) {
                0x308
            } else {
                0x62
            };
            Self {
                mapping,
                contexts: ZeroVec::alloc_from_slice(&[
                    0x2a00, 0x0505, 0x30, suffix, 0xffff, 0x2c00, 0x0505,
                ]),
            }
        }
    }
    impl RawRootData for Data {
        fn ce32(&self, cp: u32) -> Result<u32, CeError> {
            Ok(match cp {
                0 => 0,
                0x61 if matches!(self.mapping, Mapping::Expansion) => 0x02c6,
                0x61 if matches!(self.mapping, Mapping::Discontiguous) => 0x06c9,
                0x61 if matches!(self.mapping, Mapping::Contraction) => 0x00c9,
                0x61 => 0x2a00_0505,
                0x62 => 0x2c00_0505,
                0x6f => 0x4600_0505,
                0x308 => 0x0000_9605,
                0x301 => 0x0000_8805,
                0x316 => 0x0000_8a05,
                _ => 0xffff_ffff,
            })
        }
        fn ce32_at(&self, _: usize) -> Result<u32, CeError> {
            Err(CeError::MalformedProvider)
        }
        fn ce_at(&self, index: usize) -> Result<u64, CeError> {
            [0x2a00_0000_0500_0500, 0x2c00_0000_0500_0500]
                .get(index)
                .copied()
                .ok_or(CeError::MalformedProvider)
        }
        fn contexts(&self) -> &ZeroSlice<u16> {
            &self.contexts
        }
        fn jamo_ce32_at(&self, _: usize) -> Result<u32, CeError> {
            Err(CeError::MalformedProvider)
        }
        fn fcd16(&self, cp: u32) -> Result<u16, CeError> {
            Ok(match cp {
                0x308 | 0x301 => 0xe6e6,
                0x316 => 0xdcdc,
                _ => 0,
            })
        }
    }
    fn limits() -> CeLimits {
        CeLimits {
            utf16_units: 64,
            ce64: 128,
            context_depth: 64,
        }
    }
    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn raw_ce_expansion_and_legacy_continuation_keep_native_forward_offsets() {
        let expansion =
            raw_off_elements(&Data::new(Mapping::Expansion), &units("a"), limits()).unwrap();
        assert_eq!(
            expansion
                .elements
                .iter()
                .map(|e| (e.forward_low, e.forward_high))
                .collect::<Vec<_>>(),
            [(0, 1), (1, 1)]
        );
        let implicit =
            raw_off_elements(&Data::new(Mapping::Plain), &units("😀"), limits()).unwrap();
        let legacy = implicit.legacy_elements().unwrap();
        assert_eq!(legacy.len(), 2);
        assert_eq!((legacy[0].forward_low, legacy[0].forward_high), (0, 2));
        assert_eq!((legacy[1].forward_low, legacy[1].forward_high), (2, 2));
        assert!(legacy[1].continuation_half);
    }

    #[test]
    fn raw_ce_lookahead_consumes_work_even_before_any_output() {
        use crate::compat_collation::{ce::TextBudget, raw_off::raw_off_elements_bounded};
        let data = Data::new(Mapping::Contraction);
        let mut budget = TextBudget::new(3, 100_000);
        let failure =
            raw_off_elements_bounded(&data, &units("ab"), limits(), &mut budget).unwrap_err();
        assert_eq!(failure, CeError::WorkLimit);
        assert_eq!(budget.remaining_work(), 0);
    }

    #[test]
    fn raw_ce_contraction_commits_only_longest_matching_suffix() {
        let data = Data::new(Mapping::Contraction);
        let matched = raw_off_elements(&data, &units("abx"), limits()).unwrap();
        assert_eq!(matched.elements[0].value, 0x2c00_0000_0500_0500);
        assert_eq!(
            (
                matched.elements[0].forward_low,
                matched.elements[0].forward_high
            ),
            (0, 2)
        );
        assert_eq!(matched.elements[1].forward_low, 2);
        let failed = raw_off_elements(&data, &units("ax"), limits()).unwrap();
        assert_eq!(failed.elements[0].value, 0x2a00_0000_0500_0500);
        assert_eq!(
            (
                failed.elements[0].forward_low,
                failed.elements[0].forward_high
            ),
            (0, 1)
        );
    }

    #[test]
    fn simple_affix_keeps_zero_ce_and_prefix_only_combining_guard() {
        let root = FixedIcu72Root::from_validated_data(Data::new(Mapping::Plain));
        assert!(
            !root
                .starts_with_utf16(&units("o\u{308}"), &units("o"), limits())
                .unwrap()
        );
        assert!(
            root.starts_with_utf16(&units("o\0\u{308}"), &units("o"), limits())
                .unwrap()
        );
        assert!(
            root.ends_with_utf16(&units("o\u{308}"), &units("\u{308}"), limits())
                .unwrap()
        );
        assert!(
            !root
                .starts_with_utf16(&units("\u{308}"), &[0], limits())
                .unwrap()
        );
        assert!(
            root.starts_with_utf16(&units("\u{308}"), &[], limits())
                .unwrap()
        );
    }

    #[test]
    fn raw_ce_lone_surrogates_do_not_become_replacement_character() {
        let data = Data::new(Mapping::Plain);
        let lead = raw_off_elements(&data, &[0xd800], limits()).unwrap();
        let trail = raw_off_elements(&data, &[0xdc00], limits()).unwrap();
        let replacement = raw_off_elements(&data, &[0xfffd], limits()).unwrap();
        assert_ne!(lead.elements[0].value, trail.elements[0].value);
        assert_ne!(lead.elements[0].value, replacement.elements[0].value);
    }
    #[test]
    fn raw_ce_discontiguous_match_buffers_skipped_marks_and_respects_blocking() {
        let data = Data::new(Mapping::Discontiguous);
        let allowed = raw_off_elements(&data, &units("a\u{316}\u{308}"), limits()).unwrap();
        assert_eq!(
            allowed.elements.iter().map(|e| e.value).collect::<Vec<_>>(),
            [0x2c00_0000_0500_0500, 0x0000_0000_8a00_0500]
        );
        assert_eq!(
            allowed
                .elements
                .iter()
                .map(|e| (e.forward_low, e.forward_high))
                .collect::<Vec<_>>(),
            [(0, 3), (3, 3)]
        );
        let blocked = raw_off_elements(&data, &units("a\u{301}\u{308}"), limits()).unwrap();
        assert_eq!(blocked.elements[0].value, 0x2a00_0000_0500_0500);
        assert_eq!(blocked.elements[0].forward_high, 1);
        assert_eq!(blocked.elements.len(), 3);
    }
}

// These assertions are fixed-source-derived candidates, not captured oracle goldens.
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
