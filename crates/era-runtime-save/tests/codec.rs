use era_runtime_save::{
    SaveCodecLimits, SaveDocument, SaveEntry, SaveExtension, SaveFileKind, SaveFormat,
    SaveMetadata, SaveValue, Text1808Layout, Text1808ValueType, Text1808Variable, decode,
    decode_save_extension, decode_text_with_layout, encode, encode_save_extension,
    encode_text_with_layout,
};

fn binary_document(format: SaveFormat) -> SaveDocument {
    SaveDocument {
        format,
        kind: SaveFileKind::Normal,
        metadata: SaveMetadata {
            unique_code: 42,
            version: 7,
            description: "save 🙂".into(),
        },
        characters: vec![vec![SaveEntry {
            name: "NO".into(),
            value: SaveValue::Integer(3),
        }]],
        character_user_defined_starts: vec![None],
        variables: vec![
            SaveEntry {
                name: "MONEY".into(),
                value: SaveValue::Integer(-50),
            },
            SaveEntry {
                name: "NAME".into(),
                value: SaveValue::String("主人公".into()),
            },
            SaveEntry {
                name: "A".into(),
                value: SaveValue::Integers {
                    dimensions: vec![2, 2],
                    values: vec![0, 1, 300, -2],
                },
            },
            SaveEntry {
                name: "S".into(),
                value: SaveValue::Strings {
                    dimensions: vec![3],
                    values: vec![String::new(), "x".into(), String::new()],
                },
            },
        ],
        opaque_extensions: Vec::new(),
        text_payload: None,
    }
}

fn text_variable(
    name: &str,
    value_type: Text1808ValueType,
    dimensions: &[u32],
) -> Text1808Variable {
    Text1808Variable {
        name: name.into(),
        value_type,
        dimensions: dimensions.to_vec(),
    }
}

#[test]
fn binary_and_gzip_round_trip() {
    for format in [SaveFormat::Binary1808, SaveFormat::Binary1808Gzip] {
        let document = binary_document(format);
        let bytes = encode(&document, format, SaveCodecLimits::default()).unwrap();
        assert_eq!(
            encode(&document, format, SaveCodecLimits::default()).unwrap(),
            bytes,
            "save encoding must be deterministic"
        );
        assert_eq!(
            decode(&bytes, SaveCodecLimits::default()).unwrap(),
            document
        );
    }
}

#[test]
fn binary_rejects_float_unknown_and_trailing_records() {
    let bytes = encode(
        &binary_document(SaveFormat::Binary1808),
        SaveFormat::Binary1808,
        SaveCodecLimits::default(),
    )
    .unwrap();
    let money_utf16 = "MONEY"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let name = bytes
        .windows(money_utf16.len())
        .position(|window| window == money_utf16)
        .expect("MONEY key in binary fixture");
    assert_eq!(bytes[name - 1], u8::try_from(money_utf16.len()).unwrap());
    assert_eq!(bytes[name - 2], 0x00);

    for (tag, expected) in [(0x04, "unsupported Float"), (0x08, "unknown variable type")] {
        let mut invalid = bytes.clone();
        invalid[name - 2] = tag;
        let error = decode(&invalid, SaveCodecLimits::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{error}");
    }

    let mut trailing = bytes;
    trailing.push(0);
    assert!(decode(&trailing, SaveCodecLimits::default()).is_err());

    let mut gzip = encode(
        &binary_document(SaveFormat::Binary1808Gzip),
        SaveFormat::Binary1808Gzip,
        SaveCodecLimits::default(),
    )
    .unwrap();
    gzip.push(0);
    assert!(decode(&gzip, SaveCodecLimits::default()).is_err());
}

#[test]
fn gzip_decode_caps_the_expanded_body() {
    let mut document = binary_document(SaveFormat::Binary1808Gzip);
    document.metadata.description = "compressible".repeat(4096);
    let bytes = encode(
        &document,
        SaveFormat::Binary1808Gzip,
        SaveCodecLimits::default(),
    )
    .unwrap();
    let limits = SaveCodecLimits {
        maximum_bytes: bytes.len(),
        ..SaveCodecLimits::default()
    };
    let error = decode(&bytes, limits).unwrap_err().to_string();
    assert!(error.contains("decompressed bytes"), "{error}");
}

#[test]
fn all_current_binary_file_kinds_round_trip() {
    for kind in [
        SaveFileKind::Normal,
        SaveFileKind::Global,
        SaveFileKind::Variable,
        SaveFileKind::Character,
    ] {
        let mut document = binary_document(SaveFormat::Binary1808);
        document.kind = kind;
        match kind {
            SaveFileKind::Global | SaveFileKind::Variable => {
                document.characters.clear();
                document.character_user_defined_starts.clear();
            }
            SaveFileKind::Normal | SaveFileKind::Character => {}
        }
        if kind == SaveFileKind::Character {
            document.variables.clear();
        }
        let bytes = encode(
            &document,
            SaveFormat::Binary1808,
            SaveCodecLimits::default(),
        )
        .unwrap();
        assert_eq!(
            decode(&bytes, SaveCodecLimits::default()).unwrap(),
            document
        );
    }
}

#[test]
fn every_binary_file_kind_requires_its_primary_terminator() {
    for kind in [
        SaveFileKind::Normal,
        SaveFileKind::Global,
        SaveFileKind::Variable,
        SaveFileKind::Character,
    ] {
        let mut document = binary_document(SaveFormat::Binary1808);
        document.kind = kind;
        document.variables.clear();
        document.characters.clear();
        document.character_user_defined_starts.clear();
        let mut bytes = encode(
            &document,
            SaveFormat::Binary1808,
            SaveCodecLimits::default(),
        )
        .unwrap();
        if matches!(kind, SaveFileKind::Normal | SaveFileKind::Global) {
            assert_eq!(bytes.pop(), Some(0xff));
            assert_eq!(bytes.pop(), Some(0xff));
        } else {
            assert_eq!(bytes.pop(), Some(0xff));
        }
        assert!(
            decode(&bytes, SaveCodecLimits::default()).is_err(),
            "{kind:?}"
        );
    }
}

#[test]
fn typed_binary_extensions_preserve_map_order_and_xml_payloads() {
    let extensions = vec![
        SaveExtension::Map {
            key: "inventory".into(),
            entries: vec![("b".into(), "2".into()), ("a".into(), "1".into())],
        },
        SaveExtension::Xml {
            key: "tree".into(),
            document: "<root><value>🙂</value></root>".into(),
        },
        SaveExtension::DataTable {
            key: "rows".into(),
            schema: "<schema />".into(),
            data: "<data />".into(),
        },
    ];
    let mut document = binary_document(SaveFormat::Binary1808);
    document.opaque_extensions = extensions
        .iter()
        .map(|extension| encode_save_extension(extension, SaveCodecLimits::default()).unwrap())
        .collect();
    let bytes = encode(
        &document,
        SaveFormat::Binary1808,
        SaveCodecLimits::default(),
    )
    .unwrap();
    let decoded = decode(&bytes, SaveCodecLimits::default()).unwrap();
    let typed: Vec<_> = decoded
        .opaque_extensions
        .iter()
        .map(|extension| decode_save_extension(extension, SaveCodecLimits::default()).unwrap())
        .collect();
    assert_eq!(typed, extensions);
}

#[test]
fn current_text_payload_is_preserved_exactly() {
    let bytes = b"42\n7\nslot\n0\n__EMUERA_1808_STRAT__\n";
    let document = decode(bytes, SaveCodecLimits::default()).unwrap();
    assert_eq!(document.metadata.unique_code, 42);
    assert_eq!(
        encode(&document, SaveFormat::Text1808, SaveCodecLimits::default()).unwrap(),
        bytes
    );
}

#[test]
fn text_rejects_non_utf8_and_accepts_historical_metadata_envelopes() {
    assert!(decode(&[0xff], SaveCodecLimits::default()).is_err());
    let historical = decode(b"1\n2\nx\n", SaveCodecLimits::default()).unwrap();
    assert_eq!(historical.metadata.description, "x");
}

#[test]
fn schema_aware_text_round_trip_uses_reference_bom_crlf_and_groups() {
    let layout = Text1808Layout {
        kind: SaveFileKind::Normal,
        base_variables: vec![text_variable("DAY", Text1808ValueType::Integer, &[3])],
        base_character_variables: vec![text_variable("NAME", Text1808ValueType::String, &[])],
        extended_groups: vec![vec![text_variable(
            "SAVED",
            Text1808ValueType::Integer,
            &[2],
        )]],
        extended_character_groups: vec![vec![text_variable(
            "NICKNAME",
            Text1808ValueType::String,
            &[],
        )]],
        unsupported_extended_groups: Vec::new(),
        unsupported_extended_character_groups: Vec::new(),
    };
    let document = SaveDocument {
        format: SaveFormat::Text1808,
        kind: SaveFileKind::Normal,
        metadata: SaveMetadata {
            unique_code: 42,
            version: 7,
            description: "slot".into(),
        },
        characters: vec![vec![
            SaveEntry {
                name: "NAME".into(),
                value: SaveValue::String("Alice".into()),
            },
            SaveEntry {
                name: "NICKNAME".into(),
                value: SaveValue::String("A".into()),
            },
        ]],
        character_user_defined_starts: vec![None],
        variables: vec![
            SaveEntry {
                name: "DAY".into(),
                value: SaveValue::Integers {
                    dimensions: vec![3],
                    values: vec![1, 0, 0],
                },
            },
            SaveEntry {
                name: "SAVED".into(),
                value: SaveValue::Integers {
                    dimensions: vec![2],
                    values: vec![0, 9],
                },
            },
        ],
        opaque_extensions: Vec::new(),
        text_payload: None,
    };
    let bytes = encode_text_with_layout(&document, &layout, SaveCodecLimits::default()).unwrap();
    assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
    assert!(bytes.windows(2).any(|window| window == b"\r\n"));
    let decoded = decode_text_with_layout(&bytes, &layout, SaveCodecLimits::default()).unwrap();
    assert_eq!(decoded.metadata, document.metadata);
    assert_eq!(decoded.characters, document.characters);
    assert_eq!(decoded.variables, document.variables);
}

#[test]
fn schema_aware_text_rejects_float_groups_and_trailing_groups() {
    let layout = Text1808Layout {
        kind: SaveFileKind::Normal,
        base_variables: Vec::new(),
        base_character_variables: Vec::new(),
        extended_groups: vec![Vec::new(), Vec::new(), Vec::new()],
        extended_character_groups: Vec::new(),
        unsupported_extended_groups: vec![false, false, true],
        unsupported_extended_character_groups: Vec::new(),
    };
    let float = b"1\r\n2\r\nslot\r\n0\r\n__EMUERA_1808_STRAT__\r\n__EMU_SEPARATOR__\r\n__EMU_SEPARATOR__\r\nFLOAT:1\r\n__EMU_SEPARATOR__\r\n";
    let error = decode_text_with_layout(float, &layout, SaveCodecLimits::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("unsupported Float"), "{error}");

    let trailing = b"1\r\n2\r\nslot\r\n0\r\n__EMUERA_1808_STRAT__\r\n__EMU_SEPARATOR__\r\n__EMU_SEPARATOR__\r\n__EMU_SEPARATOR__\r\n__EMU_SEPARATOR__\r\n";
    assert!(decode_text_with_layout(trailing, &layout, SaveCodecLimits::default()).is_err());

    let global_layout = Text1808Layout {
        kind: SaveFileKind::Global,
        base_variables: Vec::new(),
        base_character_variables: Vec::new(),
        extended_groups: vec![Vec::new(), Vec::new(), Vec::new()],
        extended_character_groups: Vec::new(),
        unsupported_extended_groups: vec![false, false, true],
        unsupported_extended_character_groups: Vec::new(),
    };
    let global_float = b"1\r\n2\r\n__EMUERA_1808_STRAT__\r\n__EMU_SEPARATOR__\r\n__EMU_SEPARATOR__\r\nFLOAT:1\r\n__EMU_SEPARATOR__\r\n";
    assert!(
        decode_text_with_layout(global_float, &global_layout, SaveCodecLimits::default()).is_err()
    );

    let character_layout = Text1808Layout {
        kind: SaveFileKind::Normal,
        base_variables: Vec::new(),
        base_character_variables: Vec::new(),
        extended_groups: Vec::new(),
        extended_character_groups: vec![Vec::new(), Vec::new(), Vec::new()],
        unsupported_extended_groups: Vec::new(),
        unsupported_extended_character_groups: vec![false, false, true],
    };
    let character_float = b"1\r\n2\r\nslot\r\n1\r\n__EMUERA_1808_STRAT__\r\n__EMU_SEPARATOR__\r\n__EMU_SEPARATOR__\r\nFLOAT:1\r\n__EMU_SEPARATOR__\r\n";
    assert!(
        decode_text_with_layout(
            character_float,
            &character_layout,
            SaveCodecLimits::default(),
        )
        .is_err()
    );
}

#[test]
fn schema_aware_global_text_has_no_description_or_characters() {
    let layout = Text1808Layout {
        kind: SaveFileKind::Global,
        base_variables: vec![text_variable("GLOBAL", Text1808ValueType::Integer, &[2])],
        base_character_variables: Vec::new(),
        extended_groups: vec![Vec::new()],
        extended_character_groups: Vec::new(),
        unsupported_extended_groups: Vec::new(),
        unsupported_extended_character_groups: Vec::new(),
    };
    let mut document = binary_document(SaveFormat::Text1808);
    document.kind = SaveFileKind::Global;
    document.characters.clear();
    document.variables = vec![SaveEntry {
        name: "GLOBAL".into(),
        value: SaveValue::Integers {
            dimensions: vec![2],
            values: vec![5, 0],
        },
    }];
    let bytes = encode_text_with_layout(&document, &layout, SaveCodecLimits::default()).unwrap();
    let decoded = decode_text_with_layout(&bytes, &layout, SaveCodecLimits::default()).unwrap();
    assert_eq!(decoded.kind, SaveFileKind::Global);
    assert_eq!(decoded.metadata.description, "");
    assert!(decoded.characters.is_empty());
}

#[test]
fn schema_aware_text_ignores_variables_removed_from_the_project() {
    let layout = Text1808Layout {
        kind: SaveFileKind::Normal,
        base_variables: Vec::new(),
        base_character_variables: Vec::new(),
        extended_groups: vec![Vec::new(), Vec::new()],
        extended_character_groups: Vec::new(),
        unsupported_extended_groups: Vec::new(),
        unsupported_extended_character_groups: Vec::new(),
    };
    let bytes = b"1\r\n1\r\nslot\r\n0\r\n__EMUERA_1808_STRAT__\r\nOLD_STRING:value\r\n__EMU_SEPARATOR__\r\nOLD_ARRAY\r\n1\r\n2\r\n__FINISHED\r\n__EMU_SEPARATOR__\r\n";
    let decoded = decode_text_with_layout(bytes, &layout, SaveCodecLimits::default()).unwrap();
    assert!(decoded.variables.is_empty());
}

#[test]
fn schema_aware_text_restores_eramaker_prefix_without_an_extension_marker() {
    let layout = Text1808Layout {
        kind: SaveFileKind::Normal,
        base_variables: vec![text_variable("DAY", Text1808ValueType::Integer, &[])],
        base_character_variables: vec![text_variable("NAME", Text1808ValueType::String, &[])],
        extended_groups: vec![vec![text_variable(
            "SAVED",
            Text1808ValueType::Integer,
            &[],
        )]],
        extended_character_groups: Vec::new(),
        unsupported_extended_groups: Vec::new(),
        unsupported_extended_character_groups: Vec::new(),
    };
    let bytes = b"42\r\n7\r\nold slot\r\n1\r\nAlice\r\n12\r\n";
    let decoded = decode_text_with_layout(bytes, &layout, SaveCodecLimits::default()).unwrap();
    assert_eq!(decoded.metadata.description, "old slot");
    assert_eq!(
        decoded.characters[0][0].value,
        SaveValue::String("Alice".into())
    );
    assert_eq!(decoded.variables[0].value, SaveValue::Integer(12));
    assert!(decoded.variables.iter().all(|entry| entry.name != "SAVED"));
}

#[test]
fn schema_aware_text_reads_known_historical_extension_versions() {
    let layout = Text1808Layout {
        kind: SaveFileKind::Normal,
        base_variables: Vec::new(),
        base_character_variables: Vec::new(),
        extended_groups: Vec::new(),
        extended_character_groups: Vec::new(),
        unsupported_extended_groups: Vec::new(),
        unsupported_extended_character_groups: Vec::new(),
    };
    let bytes = b"42\r\n7\r\nold slot\r\n0\r\n__EMUERA_1803_STRAT__\r\n";
    let decoded = decode_text_with_layout(bytes, &layout, SaveCodecLimits::default()).unwrap();
    assert_eq!(decoded.metadata.description, "old slot");
}
