use std::collections::BTreeMap;

use era_runtime_save::{SaveEntry, SaveValue};
use erabasic_bytecode::{
    ArtifactManifest, BytecodeCallCompatibility, BytecodeGlobal, Digest, SourceMap, SymbolKey,
};
use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};
use erabasic_data::{
    Persistence, StorageScope, ValueType, VariableId as DataVariableId, VariableSchema,
};
use erabasic_vm::{EraVariableState, VmValue};

use super::entries::decode_value;
use super::*;

fn text_variable_for_mapping(value_type: Text1808ValueType, rank: usize) -> Text1808Variable {
    Text1808Variable {
        name: "MAPPED".into(),
        value_type,
        dimensions: vec![2; rank],
    }
}

#[test]
fn text_dialects_freeze_reference_and_snake_group_mappings() {
    let reference = Text1808Dialect::Reference1808;
    let snake = Text1808Dialect::Snake1808;
    assert_eq!(reference.shared_group_count(SaveFileKind::Normal), 14);
    assert_eq!(reference.shared_group_count(SaveFileKind::Global), 6);
    assert_eq!(snake.shared_group_count(SaveFileKind::Normal), 17);
    assert_eq!(snake.shared_group_count(SaveFileKind::Global), 9);
    assert_eq!(reference.character_group_count(), 6);
    assert_eq!(snake.character_group_count(), 9);

    for (rank, reference_pair, snake_pair) in [
        (1, [8, 9], [8, 9]),
        (2, [10, 11], [11, 12]),
        (3, [12, 13], [14, 15]),
    ] {
        for (value_type, offset) in [
            (Text1808ValueType::String, 0),
            (Text1808ValueType::Integer, 1),
        ] {
            let variable = text_variable_for_mapping(value_type, rank);
            assert_eq!(
                reference.user_shared_group(&variable),
                Some(reference_pair[offset])
            );
            assert_eq!(snake.user_shared_group(&variable), Some(snake_pair[offset]));
            assert_eq!(
                reference.global_group(&variable),
                Some((rank - 1) * 2 + offset)
            );
            assert_eq!(snake.global_group(&variable), Some((rank - 1) * 3 + offset));
        }
    }
    for rank in 0..=2 {
        for (value_type, offset) in [
            (Text1808ValueType::String, 0),
            (Text1808ValueType::Integer, 1),
        ] {
            let variable = text_variable_for_mapping(value_type, rank);
            assert_eq!(snake.character_group(&variable), Some(rank * 3 + offset));
        }
    }
    assert_eq!(
        snake.float_shared_groups(SaveFileKind::Normal),
        &[10, 13, 16]
    );
    assert_eq!(snake.float_shared_groups(SaveFileKind::Global), &[2, 5, 8]);
    assert_eq!(snake.float_character_groups(), &[2, 5, 8]);
    assert!(
        reference
            .float_shared_groups(SaveFileKind::Normal)
            .is_empty()
    );
    assert!(reference.float_character_groups().is_empty());
}

#[test]
#[allow(clippy::too_many_lines)]
fn snake_scoped_saves_use_standard_emuera_1808_only() {
    use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};
    let mut artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        call_compatibility: BytecodeCallCompatibility::default(),
        runtime_builtins: Vec::new(),
        runtime_native_authorizations: Vec::new(),
        runtime_host_authorizations: Vec::new(),
        runtime_staged_authorizations: Vec::new(),
        runtime_variables: Vec::new(),
        project_data: load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
            .data
            .unwrap(),
        globals: Vec::new(),
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions: Vec::new(),
        event_groups: Vec::new(),
        source_map: SourceMap::default(),
    };
    let reference = artifact.clone();
    artifact.manifest.compatibility =
        CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
    let state = EraState {
        unique_code: 1,
        version: 2,
        variables: BTreeMap::new(),
        characters: Vec::new(),
    };
    for kind in [
        SaveFileKind::Normal,
        SaveFileKind::Global,
        SaveFileKind::Variable,
        SaveFileKind::Character,
    ] {
        for format in [SaveFormat::Binary1808, SaveFormat::Binary1808Gzip] {
            let encoded = encode_scoped_save(
                &state,
                &artifact,
                kind,
                "profile fixture".into(),
                Vec::new(),
                format,
            )
            .unwrap();
            assert!(matches!(
                era_runtime_save::inspect_metadata(&encoded, true, SaveCodecLimits::default()),
                Ok(era_runtime_save::SaveMetadataInspection::Complete { .. })
            ));
            let restored = decode_scoped_save(&encoded, &artifact, kind).unwrap();
            assert_eq!(restored.state.unique_code, 1);
            assert_eq!(restored.description, "profile fixture");
            assert!(decode_scoped_save(&encoded, &reference, kind).is_ok());
        }
    }
    for (kind, expected_groups) in [(SaveFileKind::Normal, 17), (SaveFileKind::Global, 9)] {
        let encoded = encode_scoped_save(
            &state,
            &artifact,
            kind,
            "text topology".into(),
            Vec::new(),
            SaveFormat::Text1808,
        )
        .unwrap();
        let source = std::str::from_utf8(&encoded).unwrap();
        assert_eq!(source.matches("__EMU_SEPARATOR__").count(), expected_groups);
        decode_scoped_save(&encoded, &artifact, kind).unwrap();
    }
    for format in [
        SaveFormat::Text1808,
        SaveFormat::Binary1808,
        SaveFormat::Binary1808Gzip,
    ] {
        let bare =
            encode_era_save(&state, &reference, "reference".into(), Vec::new(), format).unwrap();
        if format == SaveFormat::Text1808 {
            assert!(decode_era_save(&bare, &artifact).is_err());
        } else {
            assert!(decode_era_save(&bare, &artifact).is_ok());
        }
        assert_eq!(
            decode_era_save(&bare, &reference).unwrap().description,
            "reference"
        );
    }
}

#[test]
fn sparse_binary_values_cross_the_adapter_without_dense_materialization() {
    let decoded = decode_value(&SaveValue::SparseIntegers {
        dimensions: vec![1_000_000],
        values: vec![(17, 7), (999_999, 9)],
    });
    assert_eq!(decoded.value_type, BytecodeType::Integer);
    assert_eq!(decoded.dimensions, [1_000_000]);
    assert!(decoded.values.is_empty());
    assert_eq!(
        decoded.sparse_values.unwrap(),
        [(17, VmValue::Integer(7)), (999_999, VmValue::Integer(9))]
    );
}

#[test]
fn structured_merge_replaces_declared_record_and_preserves_unknown_payload() {
    let unknown = OpaqueSaveExtension {
        type_tag: 0x7f,
        key: "future".into(),
        payload: vec![1, 2, 3],
    };
    let stale = encode_save_extension(
        &SaveExtension::Map {
            key: "state".into(),
            entries: vec![("old".into(), "value".into())],
        },
        SaveCodecLimits::default(),
    )
    .unwrap();
    let merged = merge_structured_extensions(
        &[unknown.clone(), stale],
        vec![StructuredExtension::Map {
            key: "state".into(),
            entries: vec![("new".into(), "value".into())],
        }],
    )
    .unwrap();

    assert!(merged.contains(&unknown));
    let map = merged
        .iter()
        .find(|value| value.type_tag == 0x20)
        .map(|value| decode_save_extension(value, SaveCodecLimits::default()).unwrap())
        .unwrap();
    assert_eq!(
        map,
        SaveExtension::Map {
            key: "state".into(),
            entries: vec![("new".into(), "value".into())],
        }
    );
}

#[test]
fn structured_merge_drops_recognized_records_absent_from_the_current_scope() {
    let ordinary = encode_save_extension(
        &SaveExtension::DataTable {
            key: "ordinary".into(),
            schema: "ordinary-schema".into(),
            data: "ordinary-data".into(),
        },
        SaveCodecLimits::default(),
    )
    .unwrap();
    let global = StructuredExtension::DataTable {
        key: "global".into(),
        schema: "global-schema".into(),
        data: "global-data".into(),
    };

    let merged = merge_structured_extensions(&[ordinary], vec![global]).unwrap();

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].key, "global");
    assert_eq!(merged[0].type_tag, 0x22);
}

#[test]
fn binary_entries_follow_reference_and_user_declaration_order() {
    let mut project_data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .unwrap();
    for name in ["Z_USER", "A_USER"] {
        project_data.schema.register_user_variable(VariableSchema {
            id: DataVariableId::user(name),
            value_type: ValueType::Integer,
            storage: StorageScope::Character,
            dimensions: Vec::new(),
            mutable: true,
            persistence: Persistence::GameSave,
            can_forbid: false,
        });
    }
    let artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        call_compatibility: BytecodeCallCompatibility::default(),
        runtime_builtins: Vec::new(),
        runtime_native_authorizations: Vec::new(),
        runtime_host_authorizations: Vec::new(),
        runtime_staged_authorizations: Vec::new(),
        runtime_variables: Vec::new(),
        project_data,
        globals: Vec::new(),
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions: Vec::new(),
        event_groups: Vec::new(),
        source_map: SourceMap::default(),
    };
    let mut variables = BTreeMap::new();
    for (name, value) in [("A_USER", 4), ("CFLAG", 3), ("Z_USER", 2), ("NO", 1)] {
        let key = SymbolKey::derive("save-order-test", name.as_bytes());
        variables.insert(
            key,
            EraVariableState {
                name: name.into(),
                value_type: BytecodeType::Integer,
                dimensions: Vec::new(),
                persistence: BytecodePersistence::GameSave,
                storage: BytecodeStorage::Character,
                values: vec![VmValue::Integer(value)],
                sparse_values: None,
            },
        );
    }

    let encoded = encode_entries(&variables, &artifact, true).unwrap();

    assert_eq!(
        encoded
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["NO", "CFLAG", "Z_USER", "A_USER"]
    );
    assert_eq!(encoded.user_defined_start, Some(2));
}

#[test]
fn save_definition_index_separates_shared_and_character_entries() {
    let project_data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .unwrap();
    let definitions = [
        BytecodeGlobal {
            key: erabasic_bytecode::SymbolKey::derive("save-index", b"shared"),
            name: "SHARED_VALUE".into(),
            value_type: BytecodeType::Integer,
            dimensions: Vec::new(),
            mutable: true,
            storage: BytecodeStorage::Project,
            persistence: BytecodePersistence::GameSave,
            initial_values: Vec::new(),
            owner: None,
        },
        BytecodeGlobal {
            key: erabasic_bytecode::SymbolKey::derive("save-index", b"character"),
            name: "CHARACTER_VALUE".into(),
            value_type: BytecodeType::Integer,
            dimensions: Vec::new(),
            mutable: true,
            storage: BytecodeStorage::Character,
            persistence: BytecodePersistence::GameSave,
            initial_values: Vec::new(),
            owner: None,
        },
    ];
    let artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        call_compatibility: BytecodeCallCompatibility::default(),
        runtime_builtins: Vec::new(),
        runtime_native_authorizations: Vec::new(),
        runtime_host_authorizations: Vec::new(),
        runtime_staged_authorizations: Vec::new(),
        runtime_variables: definitions
            .iter()
            .map(|global| erabasic_bytecode::RuntimeVariableSymbol {
                match_name_rejection: None,
                character_disposal: erabasic_bytecode::CharacterArrayDisposal::Preserve,
                key: global.key,
                reference: false,
                reference_semantics: erabasic_bytecode::RuntimeReferenceSemantics {
                    is_const: false,
                    can_restructure: false,
                },
            })
            .collect(),
        project_data,
        globals: definitions.to_vec(),
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions: Vec::new(),
        event_groups: Vec::new(),
        source_map: SourceMap::default(),
    };
    let index = SaveDefinitionIndex::new(&artifact);
    let shared = decode_entries(
        &[
            SaveEntry {
                name: "shared_value".into(),
                value: SaveValue::Integer(7),
            },
            SaveEntry {
                name: "CHARACTER_VALUE".into(),
                value: SaveValue::Integer(8),
            },
        ],
        &index.shared,
    )
    .unwrap();
    let character = decode_entries(
        &[
            SaveEntry {
                name: "SHARED_VALUE".into(),
                value: SaveValue::Integer(9),
            },
            SaveEntry {
                name: "character_value".into(),
                value: SaveValue::Integer(10),
            },
        ],
        &index.character,
    )
    .unwrap();

    assert_eq!(shared.len(), 1);
    assert_eq!(shared[&definitions[0].key].values, [VmValue::Integer(7)]);
    assert_eq!(character.len(), 1);
    assert_eq!(
        character[&definitions[1].key].values,
        [VmValue::Integer(10)]
    );
}
