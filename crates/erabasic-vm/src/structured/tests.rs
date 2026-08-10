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
    assert!(document.select("//p[contains(k, 'o')]").is_err());
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
    state.data_tables.insert("table".into(), DataTable::new());
    let request = NativeCallRequest {
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
            name: "name".into(),
            value_type: DataType::String,
            nullable: true,
        },
        Column {
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
