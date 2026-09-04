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
fn sql_map_xml_rows_preserve_inner_xml_order_and_duplicate_keys() {
    let rows = parse_map_xml_rows(
        r#"<?xml version="1.0"?>
        <map>
          <p><k>plain</k><v>first</v></p>
          <p><k>markup</k><v><b lang="zh">bold</b>&amp;text</v></p>
          <p><k>empty</k><v /></p>
          <p><k>duplicate</k><v>old</v></p>
          <p><k>duplicate</k><v><i>new</i></v></p>
          <p><k>missing-value</k></p>
          <p><v>missing-key</v></p>
        </map>"#,
    )
    .unwrap();

    assert_eq!(
        rows,
        vec![
            ("plain".into(), "first".into()),
            ("markup".into(), "<b lang=\"zh\">bold</b>&amp;text".into()),
            ("empty".into(), String::new()),
            ("duplicate".into(), "old".into()),
            ("duplicate".into(), "<i>new</i>".into()),
        ]
    );
}

#[test]
fn sql_map_xml_rows_require_the_exact_root_and_direct_children() {
    assert!(parse_map_xml_rows("<root><map /></root>").is_err());
    assert!(parse_map_xml_rows("<Map />").is_err());

    let rows = parse_map_xml_rows(
        "<map><group><p><k>nested-row</k><v>x</v></p></group>\
         <p><group><k>nested-key</k></group><v>x</v></p>\
         <p><k>first</k><k>second</k><v>one</v><v>two</v></p></map>",
    )
    .unwrap();
    assert_eq!(rows, vec![("first".into(), "one".into())]);
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
