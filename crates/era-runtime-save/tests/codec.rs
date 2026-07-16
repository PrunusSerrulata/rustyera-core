use era_runtime_save::{
    SaveCodecLimits, SaveDocument, SaveEntry, SaveFileKind, SaveFormat, SaveMetadata, SaveValue,
    Text1808Layout, Text1808ValueType, Text1808Variable, decode, decode_text_with_layout, encode,
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
            SaveFileKind::Global | SaveFileKind::Variable => document.characters.clear(),
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
fn text_rejects_non_utf8_and_old_envelopes() {
    assert!(decode(&[0xff], SaveCodecLimits::default()).is_err());
    assert!(decode(b"1\n2\nx\n", SaveCodecLimits::default()).is_err());
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
fn schema_aware_global_text_has_no_description_or_characters() {
    let layout = Text1808Layout {
        kind: SaveFileKind::Global,
        base_variables: vec![text_variable("GLOBAL", Text1808ValueType::Integer, &[2])],
        base_character_variables: Vec::new(),
        extended_groups: vec![Vec::new()],
        extended_character_groups: Vec::new(),
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
    };
    let bytes = b"1\r\n1\r\nslot\r\n0\r\n__EMUERA_1808_STRAT__\r\nOLD_STRING:value\r\n__EMU_SEPARATOR__\r\nOLD_ARRAY\r\n1\r\n2\r\n__FINISHED\r\n__EMU_SEPARATOR__\r\n";
    let decoded = decode_text_with_layout(bytes, &layout, SaveCodecLimits::default()).unwrap();
    assert!(decoded.variables.is_empty());
}
