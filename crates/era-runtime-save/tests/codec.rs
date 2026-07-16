use era_runtime_save::{
    SaveCodecLimits, SaveDocument, SaveEntry, SaveFileKind, SaveFormat, SaveMetadata, SaveValue,
    decode, encode,
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
