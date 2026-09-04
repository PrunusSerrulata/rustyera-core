use super::*;
use erabasic_bytecode::{BytecodeGlobal, BytecodePersistence};
fn cell() -> (SymbolKey, VariableCell) {
    let key = SymbolKey::derive("array-lease-test", b"BASE");
    let definition = BytecodeGlobal {
        key,
        name: "BASE".into(),
        value_type: BytecodeType::Integer,
        dimensions: vec![2],
        mutable: true,
        storage: BytecodeStorage::Character,
        persistence: BytecodePersistence::GameSave,
        initial_values: Vec::new(),
        owner: None,
    };
    let mut cell = VariableCell::new(&definition);
    cell.set(0, VmValue::Integer(7)).unwrap();
    (key, cell)
}

fn owner(frame: u64) -> ArrayLeaseOwner {
    ArrayLeaseOwner {
        fiber: FiberId(1),
        frame: FrameId(frame),
        generation: GenerationId(1),
        function: SymbolKey::derive("array-lease-test", b"function"),
        origin: ArrayLeaseOrigin::Bytecode { begin: 5 },
    }
}

#[test]
fn removed_character_backing_is_cleared_shared_and_released_with_its_last_owner() {
    let (key, cell) = cell();
    let mut leases = ArrayLeases::default();
    let location = ArrayLocation::Character {
        legacy: None,
        index: 0,
        key,
    };
    let first = leases
        .insert(ArrayLease {
            owner: owner(1),
            input: key,
            location,
            length: 2,
            value_type: BytecodeType::Integer,
            dimensions: vec![2],
            character_disposal: erabasic_bytecode::CharacterArrayDisposal::ClearSparse,
        })
        .unwrap();
    let second = leases
        .insert(ArrayLease {
            owner: owner(2),
            input: key,
            location,
            length: 2,
            value_type: BytecodeType::Integer,
            dimensions: vec![2],
            character_disposal: erabasic_bytecode::CharacterArrayDisposal::ClearSparse,
        })
        .unwrap();
    let old = vec![([(key, cell)]).into_iter().collect()];
    leases.remap_characters(None, &old, &[]).unwrap();
    let ArrayLocation::Detached(backing) = leases.entries[&first].location else {
        panic!("deleted character must retain a detached backing");
    };
    assert_eq!(
        leases.entries[&second].location,
        ArrayLocation::Detached(backing)
    );
    assert_eq!(
        leases.detached[&backing].to_values(),
        vec![VmValue::Integer(0); 2]
    );
    leases
        .detached
        .get_mut(&backing)
        .unwrap()
        .set(0, VmValue::Integer(9))
        .unwrap();
    leases.retain(&BTreeSet::from([second]));
    assert_eq!(leases.detached[&backing].get(0), Some(VmValue::Integer(9)));
    leases.retain(&BTreeSet::new());
    assert!(leases.detached.is_empty());
}

#[test]
fn character_permutation_retains_backing_and_rejects_invalid_order_atomically() {
    let (key, cell) = cell();
    let mut leases = ArrayLeases::default();
    let location = ArrayLocation::Character {
        legacy: None,
        index: 0,
        key,
    };
    let id = leases
        .insert(ArrayLease {
            owner: owner(1),
            input: key,
            location,
            length: 2,
            value_type: BytecodeType::Integer,
            dimensions: vec![2],
            character_disposal: erabasic_bytecode::CharacterArrayDisposal::ClearSparse,
        })
        .unwrap();
    let old = vec![
        [(key, cell.clone())].into_iter().collect(),
        [(key, cell)].into_iter().collect(),
    ];
    let before = leases.clone();
    assert!(leases.remap_characters(None, &old, &[1, 1]).is_err());
    assert_eq!(leases, before);
    leases.remap_characters(None, &old, &[1, 0]).unwrap();
    assert_eq!(
        leases.entries[&id].location,
        ArrayLocation::Character {
            legacy: None,
            index: 1,
            key
        }
    );
    assert!(leases.detached.is_empty());
    leases.migrate_generation(
        GenerationId(1),
        &BTreeSet::new(),
        &BTreeSet::new(),
        &BTreeSet::from([key]),
    );
    assert_eq!(
        leases.entries[&id].location,
        ArrayLocation::Character {
            legacy: Some(GenerationId(1)),
            index: 1,
            key
        }
    );
}
#[test]
fn detached_preserve_policy_keeps_string_and_multidimensional_cells_shared() {
    for (value_type, dimensions, value) in [
        (
            BytecodeType::String,
            vec![2],
            VmValue::String("kept".into()),
        ),
        (BytecodeType::Integer, vec![2, 2], VmValue::Integer(17)),
        (BytecodeType::Integer, vec![2, 2, 2], VmValue::Integer(19)),
    ] {
        let key = SymbolKey::derive("array-lease-test", b"USER_OR_DENSE");
        let definition = BytecodeGlobal {
            key,
            name: "USER_OR_DENSE".into(),
            value_type,
            dimensions: dimensions.clone(),
            mutable: true,
            storage: BytecodeStorage::Character,
            persistence: BytecodePersistence::GameSave,
            initial_values: Vec::new(),
            owner: None,
        };
        let mut cell = VariableCell::new(&definition);
        cell.set(0, value.clone()).unwrap();
        let mut leases = ArrayLeases::default();
        let location = ArrayLocation::Character {
            legacy: None,
            index: 0,
            key,
        };
        let first = leases
            .insert(ArrayLease {
                owner: owner(1),
                input: key,
                location,
                length: cell.len(),
                value_type,
                dimensions: dimensions.clone(),
                character_disposal: erabasic_bytecode::CharacterArrayDisposal::Preserve,
            })
            .unwrap();
        let mut alias = leases.entries[&first].clone();
        alias.owner = owner(2);
        let second = leases.insert(alias).unwrap();
        leases
            .remap_characters(None, &[[(key, cell)].into_iter().collect()], &[])
            .unwrap();
        let ArrayLocation::Detached(backing) = leases.entries[&first].location else {
            panic!("missing detached object");
        };
        assert_eq!(
            leases.entries[&second].location,
            ArrayLocation::Detached(backing)
        );
        assert_eq!(leases.detached[&backing].get(0), Some(value));
        assert_eq!(leases.detached[&backing].dimensions, dimensions);
        leases.retain(&BTreeSet::from([second]));
        assert_eq!(leases.detached.len(), 1);
        leases.retain(&BTreeSet::new());
        assert!(leases.detached.is_empty());
    }
}
