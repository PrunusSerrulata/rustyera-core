use erabasic_bytecode::{BytecodePersistence, BytecodeStorage};

use super::*;

fn global(value_type: BytecodeType, dimensions: Vec<u64>) -> BytecodeGlobal {
    BytecodeGlobal {
        key: SymbolKey::derive("memory.test", format!("{value_type:?}").as_bytes()),
        name: "VALUE".into(),
        value_type,
        dimensions,
        mutable: true,
        storage: BytecodeStorage::Project,
        persistence: BytecodePersistence::GameSave,
        initial_values: Vec::new(),
        owner: None,
    }
}

#[test]
fn variable_map_serialization_is_key_ordered_and_accepts_legacy_maps() {
    let definition = global(BytecodeType::Integer, vec![1]);
    let entries = [
        (
            SymbolKey::derive("memory.test", b"second"),
            VariableCell::new(&definition),
        ),
        (
            SymbolKey::derive("memory.test", b"first"),
            VariableCell::new(&definition),
        ),
    ];
    let forward = entries.clone().into_iter().collect::<VariableMap>();
    let reverse = entries.into_iter().rev().collect::<VariableMap>();

    let encoded = serde_json::to_vec(&forward).unwrap();
    assert_eq!(encoded, serde_json::to_vec(&reverse).unwrap());
    let legacy = forward
        .iter()
        .map(|(key, value)| (*key, value.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(encoded, serde_json::to_vec(&legacy).unwrap());
    let decoded: VariableMap = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded.len(), forward.len());
    for (key, cell) in &*forward {
        assert_eq!(
            decoded.get(key).map(VariableCell::to_values),
            Some(cell.to_values())
        );
    }
}

fn storage_definitions() -> [BytecodeGlobal; 4] {
    let mut definitions = [
        global(BytecodeType::Integer, vec![1]),
        global(BytecodeType::Integer, vec![1]),
        global(BytecodeType::Integer, vec![1]),
        global(BytecodeType::Integer, vec![1]),
    ];
    for (index, (definition, storage)) in definitions
        .iter_mut()
        .zip([
            BytecodeStorage::Project,
            BytecodeStorage::FunctionStatic,
            BytecodeStorage::FunctionPersistent,
            BytecodeStorage::Character,
        ])
        .enumerate()
    {
        definition.key = SymbolKey::derive(
            "memory.storage",
            &[u8::try_from(index).expect("four storage definitions")],
        );
        definition.storage = storage;
    }
    definitions
}

fn memory_with_storage_cells(definitions: &[BytecodeGlobal; 4]) -> Memory {
    let mut memory = Memory::default();
    memory.characters.push(VariableMap::default());
    for definition in definitions {
        let cell = VariableCell::new(definition);
        match definition.storage {
            BytecodeStorage::Project => {
                memory.shared.insert(definition.key, cell);
            }
            BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                memory.statics.insert(definition.key, cell);
            }
            BytecodeStorage::Character => {
                memory.characters[0].insert(definition.key, cell);
            }
            _ => unreachable!("the fixture contains only mutable storage classes"),
        }
    }
    memory
}

#[test]
fn mutable_cell_resolution_preserves_storage_classes_and_legacy_generations() {
    let current = GenerationId(2);
    let legacy_generation = GenerationId(1);
    let definitions = storage_definitions();
    let mut memory = memory_with_storage_cells(&definitions);

    for (definition, expected) in definitions.iter().zip(1_i64..) {
        memory
            .cell_mut(current, definition.key, definition.storage, 0)
            .expect("storage-class cell")
            .write(&[0], VmValue::Integer(expected))
            .unwrap();
        assert_eq!(
            memory.cell(current, definition, 0).unwrap().read(&[0]),
            Ok(VmValue::Integer(expected))
        );
    }
    assert!(
        memory
            .cell_mut(
                current,
                definitions[0].key,
                BytecodeStorage::FunctionLocal,
                0,
            )
            .is_none()
    );

    let legacy_cells = memory_with_storage_cells(&definitions);
    memory.legacy.insert(
        legacy_generation,
        super::store::LegacyMemory {
            shared: legacy_cells.shared,
            statics: legacy_cells.statics,
            characters: legacy_cells.characters,
        },
    );
    for (definition, expected) in definitions.iter().zip(41_i64..) {
        memory
            .cell_mut(legacy_generation, definition.key, definition.storage, 0)
            .expect("legacy storage-class cell")
            .write(&[0], VmValue::Integer(expected))
            .unwrap();
        assert_eq!(
            memory
                .cell(legacy_generation, definition, 0)
                .unwrap()
                .read(&[0]),
            Ok(VmValue::Integer(expected))
        );
    }
    assert_eq!(
        memory.shared.get(&definitions[0].key).unwrap().read(&[0]),
        Ok(VmValue::Integer(1))
    );
}

#[test]
fn dense_integer_cell_preserves_public_vm_value_behavior() {
    let mut cell = VariableCell::new(&global(BytecodeType::Integer, vec![4]));
    cell.write(&[2], VmValue::Integer(41)).unwrap();
    cell.set(3, VmValue::Integer(42)).unwrap();

    assert_eq!(cell.read(&[2]).unwrap(), VmValue::Integer(41));
    assert_eq!(
        cell.to_values(),
        vec![
            VmValue::Integer(0),
            VmValue::Integer(0),
            VmValue::Integer(41),
            VmValue::Integer(42),
        ]
    );
    assert!(cell.set(0, VmValue::String("wrong".into())).is_err());
    assert_eq!(cell.read(&[0]).unwrap(), VmValue::Integer(0));
}

#[test]
fn dense_place_cell_boxes_only_values_crossing_the_vm_boundary() {
    let mut cell = VariableCell::new(&global(BytecodeType::IntegerPlace, vec![1]));
    let place = PlaceDescriptor {
        variable: SymbolKey::derive("memory.test", b"target"),
        indices: vec![2, 3],
        ..PlaceDescriptor::default()
    };
    cell.set(0, VmValue::IntegerPlace(Box::new(place.clone())))
        .unwrap();

    assert_eq!(cell.first(), Some(VmValue::IntegerPlace(Box::new(place))));
    assert!(cell.storage_is_valid());
}

#[test]
fn large_cells_keep_default_storage_sparse_during_point_updates() {
    let integer_definition = global(BytecodeType::Integer, vec![1_000_000]);
    let mut integer = VariableCell::new(&integer_definition);
    assert!(matches!(
        integer.values,
        VariableValues::SparseIntegers { ref entries, .. } if entries.is_empty()
    ));
    assert_eq!(integer.read(&[999_999]).unwrap(), VmValue::Integer(0));
    integer.set(999_999, VmValue::Integer(42)).unwrap();
    integer.set(10, VmValue::Integer(11)).unwrap();
    integer.fill_range(0, 100, VmValue::Integer(0)).unwrap();
    assert_eq!(integer.get(10), Some(VmValue::Integer(0)));
    assert_eq!(integer.get(999_999), Some(VmValue::Integer(42)));
    assert!(matches!(
        integer.values,
        VariableValues::SparseIntegers { ref entries, .. } if entries.len() == 1
    ));
    integer.set(999_999, VmValue::Integer(0)).unwrap();
    assert!(matches!(
        integer.values,
        VariableValues::SparseIntegers { ref entries, .. } if entries.is_empty()
    ));

    let string_definition = global(BytecodeType::String, vec![1_000_000]);
    let mut string = VariableCell::new(&string_definition);
    string
        .set(750_000, VmValue::String("value".into()))
        .unwrap();
    string.fill(VmValue::String(String::new())).unwrap();
    assert_eq!(string.get(750_000), Some(VmValue::String(String::new())));
    assert!(matches!(
        string.values,
        VariableValues::SparseStrings { ref entries, .. } if entries.is_empty()
    ));
}

#[test]
fn exact_overlay_preserves_sparse_and_dense_storage_and_is_atomic_on_type_errors() {
    let mut sparse = VariableCell::new(&global(BytecodeType::Integer, vec![1_000_000]));
    let mut saved = vec![VmValue::Integer(0); 1_000_000];
    saved[17] = VmValue::Integer(7);
    saved[999_999] = VmValue::Integer(9);
    sparse.overlay(&[1_000_000], &saved).unwrap();
    assert!(matches!(
        sparse.values,
        VariableValues::SparseIntegers { ref entries, .. }
            if entries == &[(17, 7), (999_999, 9)]
    ));
    assert_eq!(sparse.get(17), Some(VmValue::Integer(7)));

    let mut dense = VariableCell::new(&global(BytecodeType::Integer, vec![4]));
    dense
        .overlay(
            &[4],
            &[
                VmValue::Integer(1),
                VmValue::Integer(2),
                VmValue::Integer(3),
                VmValue::Integer(4),
            ],
        )
        .unwrap();
    assert!(
        matches!(dense.values, VariableValues::Integers(ref values) if values == &[1, 2, 3, 4])
    );

    let before = dense.clone();
    assert!(
        dense
            .overlay(
                &[4],
                &[
                    VmValue::Integer(5),
                    VmValue::String("wrong".into()),
                    VmValue::Integer(7),
                    VmValue::Integer(8),
                ],
            )
            .is_err()
    );
    assert_eq!(dense, before);
}

#[test]
fn exact_sparse_overlay_avoids_materializing_skipped_defaults() {
    let mut sparse = VariableCell::new(&global(BytecodeType::Integer, vec![1_000_000]));
    sparse
        .overlay_sparse(
            &[1_000_000],
            &[(17, VmValue::Integer(7)), (999_999, VmValue::Integer(9))],
        )
        .unwrap();
    assert!(matches!(
        sparse.values,
        VariableValues::SparseIntegers { ref entries, .. }
            if entries == &[(17, 7), (999_999, 9)]
    ));

    let mut dense = VariableCell::new(&global(BytecodeType::Integer, vec![4]));
    dense
        .overlay_sparse(&[4], &[(1, VmValue::Integer(2)), (3, VmValue::Integer(4))])
        .unwrap();
    assert!(matches!(
        dense.values,
        VariableValues::Integers(ref values) if values == &[0, 2, 0, 4]
    ));

    let before = dense.clone();
    assert!(
        dense
            .overlay_sparse(&[4], &[(2, VmValue::String("wrong".into()))])
            .is_err()
    );
    assert_eq!(dense, before);
}

#[test]
fn dense_initial_values_and_randdata_keep_contiguous_storage() {
    let mut initialized = global(BytecodeType::Integer, vec![256]);
    initialized.initial_values = vec![BytecodeConstant::Integer(1); 128];
    let initialized = VariableCell::new(&initialized);
    assert!(matches!(initialized.values, VariableValues::Integers(_)));

    let mut randdata = global(BytecodeType::Integer, vec![625]);
    randdata.name = "RANDDATA".into();
    let randdata = VariableCell::new(&randdata);
    assert!(randdata.integers().is_some());
}

#[test]
fn snapshot_cells_use_sparse_round_trippable_storage() {
    let mut integer = VariableCell::new(&global(BytecodeType::Integer, vec![1_000_000]));
    integer.set(999_999, VmValue::Integer(42)).unwrap();
    let encoded = rmp_serde::to_vec(&integer).unwrap();
    assert!(encoded.len() < 128);
    let mut decoded = rmp_serde::from_slice::<VariableCell>(&encoded).unwrap();
    decoded.materialize_snapshot().unwrap();
    integer.materialize_snapshot().unwrap();
    assert_eq!(decoded, integer);

    let mut string = VariableCell::new(&global(BytecodeType::String, vec![8]));
    string.set(5, VmValue::String("preserved".into())).unwrap();
    let encoded = rmp_serde::to_vec(&string).unwrap();
    let mut decoded = rmp_serde::from_slice::<VariableCell>(&encoded).unwrap();
    decoded.materialize_snapshot().unwrap();
    assert_eq!(decoded, string);

    let mut place = VariableCell::new(&global(BytecodeType::IntegerPlace, vec![3]));
    place
        .set(
            2,
            VmValue::IntegerPlace(Box::new(PlaceDescriptor {
                variable: SymbolKey::derive("memory.test", b"snapshot-place"),
                indices: vec![4],
                ..PlaceDescriptor::default()
            })),
        )
        .unwrap();
    let encoded = rmp_serde::to_vec(&place).unwrap();
    let mut decoded = rmp_serde::from_slice::<VariableCell>(&encoded).unwrap();
    decoded.materialize_snapshot().unwrap();
    assert_eq!(decoded, place);

    for malformed in [
        SparseVariableValues::Integers(vec![(1, 1), (1, 2)]),
        SparseVariableValues::Integers(vec![(2, 1)]),
        SparseVariableValues::Strings(vec![(1, "wrong type".into())]),
    ] {
        let encoded = rmp_serde::to_vec(&(BytecodeType::Integer, vec![2], malformed)).unwrap();
        assert!(rmp_serde::from_slice::<VariableCell>(&encoded).is_err());
    }
}

#[test]
#[cfg(target_pointer_width = "64")]
fn sparse_snapshot_decode_defers_untrusted_dense_allocation() {
    let encoded = rmp_serde::to_vec(&(
        BytecodeType::Integer,
        vec![u64::MAX],
        SparseVariableValues::Integers(Vec::new()),
    ))
    .unwrap();
    let decoded = rmp_serde::from_slice::<VariableCell>(&encoded).unwrap();
    assert_eq!(decoded.len(), usize::MAX);
    assert!(decoded.values.get(usize::MAX - 1).is_some());
}

#[test]
fn common_variable_shapes_preserve_flattening_and_bounds() {
    assert_eq!(flatten(&[], &[]).unwrap(), 0);
    assert_eq!(flatten(&[8], &[]).unwrap(), 0);
    assert_eq!(flatten(&[8], &[7]).unwrap(), 7);
    assert_eq!(flatten(&[2, 3], &[1, 2]).unwrap(), 5);
    assert_eq!(
        flatten(&[8], &[8]).unwrap_err(),
        "index 8 is outside dimension 0 of length 8"
    );
    assert_eq!(
        flatten(&[8], &[1, 0]).unwrap_err(),
        "too many variable indices"
    );
}

#[test]
#[cfg(target_pointer_width = "64")]
fn public_vm_value_stays_small_enough_for_transient_stacks() {
    assert_eq!(std::mem::size_of::<VmValue>(), 24);
    assert_eq!(std::mem::size_of::<i64>(), 8);
}

#[test]
fn execution_index_failures_distinguish_script_bounds_from_bad_storage() {
    let mut cell = VariableCell::new(&global(BytecodeType::Integer, vec![2]));
    cell.write(&[1], VmValue::Integer(31)).unwrap();
    let prior = cell.clone();
    let failure = cell.write_execution(&[2], VmValue::Integer(9)).unwrap_err();
    assert_eq!(
        failure.category,
        crate::FaultCategory::Script(crate::ScriptFaultKind::Bounds)
    );
    assert_eq!(cell, prior);
    assert!(cell.read_execution(&[0, 0]).unwrap_err().is_script());
    assert!(
        !cell
            .write_execution(&[0], VmValue::String("wrong physical type".into()))
            .unwrap_err()
            .is_script()
    );
    assert_eq!(cell, prior);

    // A forged declared shape is not a script request to access an absent element.
    cell.dimensions = vec![3];
    let broken = cell.read_execution(&[2]).unwrap_err();
    assert_eq!(broken.category, crate::FaultCategory::InternalInvariant);
    assert_eq!(broken.code, crate::VmFaultCode::InvalidInstruction);
    assert_eq!(failure.code, crate::VmFaultCode::Bounds);
}

#[test]
fn execution_index_offset_overflow_is_a_resource_failure() {
    let failure = flatten_execution(&[u64::MAX, u64::MAX], &[2, 1]).unwrap_err();
    assert_eq!(failure.category, crate::FaultCategory::ResourceLimit);
    assert!(!failure.is_script());
}
