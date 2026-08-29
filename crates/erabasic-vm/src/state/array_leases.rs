//! Scoped backing identities for staged bit-array calls. A token resolves once,
//! before later arguments run; subsequent REF rebinding cannot select another array.

use std::collections::{BTreeMap, BTreeSet};

use erabasic_bytecode::{BytecodeStorage, BytecodeType, SymbolKey};
use serde::{Deserialize, Serialize};

use super::{Fiber, Vm, find_frame, find_frame_mut};
use crate::{
    FiberId, FrameId, GenerationId, Memory, PlaceDescriptor, VariableCell, VariableMap, VmError,
    VmValue,
};

pub(crate) use crate::ArrayBackingId as ArrayLeaseId;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum ArrayLeaseOrigin {
    Bytecode {
        begin: usize,
    },
    RuntimeForm {
        instruction: usize,
        slot: u64,
    },
    UserBytecode {
        resolve: usize,
        slot: usize,
    },
    UserForm {
        instruction: usize,
        call: u64,
        slot: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ArrayLeaseOwner {
    pub fiber: FiberId,
    pub frame: FrameId,
    pub generation: GenerationId,
    pub function: SymbolKey,
    pub origin: ArrayLeaseOrigin,
}

/// A physical location, rather than a script character number or REF token.
/// Character permutations update these locations before exposing the new order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub(crate) enum ArrayLocation {
    Shared {
        legacy: Option<GenerationId>,
        key: SymbolKey,
    },
    Static {
        legacy: Option<GenerationId>,
        key: SymbolKey,
    },
    Character {
        legacy: Option<GenerationId>,
        index: usize,
        key: SymbolKey,
    },
    Local {
        frame: FrameId,
        key: SymbolKey,
    },
    Detached(ArrayLeaseId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ArrayLease {
    pub owner: ArrayLeaseOwner,
    pub input: SymbolKey,
    pub location: ArrayLocation,
    pub length: usize,
    pub value_type: BytecodeType,
    pub dimensions: Vec<u64>,
    pub character_disposal: erabasic_bytecode::CharacterArrayDisposal,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ArrayLeases {
    next: u64,
    revision: u64,
    #[serde(skip)]
    pub(crate) protected: BTreeSet<ArrayLeaseId>,
    pub entries: BTreeMap<ArrayLeaseId, ArrayLease>,
    pub detached: BTreeMap<ArrayLeaseId, VariableCell>,
}

fn invalid(message: &str) -> VmError {
    VmError::InvalidState(message.into())
}

fn argument(message: &str) -> VmError {
    VmError::ScriptFailure(crate::ExecutionFailure::script(
        crate::ScriptFaultKind::Argument,
        crate::VmFaultCode::InvalidInstruction,
        message,
    ))
}

impl ArrayLeases {
    fn bump(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
    /// Whole-row copies preserve logical backing locations but can replace a
    /// destination cell with a source cell having the same revision. Invalidate
    /// prepared timelines independently of those per-cell revision numbers.
    pub(crate) fn character_values_replaced(&mut self) {
        self.bump();
    }
    pub(crate) fn release(&mut self, id: ArrayLeaseId) {
        let live = self
            .entries
            .keys()
            .copied()
            .filter(|candidate| *candidate != id)
            .collect();
        self.retain(&live);
    }
    pub(crate) fn materialize_snapshot(&mut self) -> Result<(), String> {
        for cell in self.detached.values_mut() {
            cell.materialize_snapshot()?;
        }
        Ok(())
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        self.entries
            .len()
            .saturating_mul(std::mem::size_of::<ArrayLease>())
            .saturating_add(self.entries.values().fold(0usize, |bytes, lease| {
                bytes.saturating_add(
                    lease
                        .dimensions
                        .capacity()
                        .saturating_mul(std::mem::size_of::<u64>()),
                )
            }))
            .saturating_add(self.detached.values().fold(0_usize, |bytes, cell| {
                bytes.saturating_add(cell.retained_bytes())
            }))
    }

    pub(crate) fn retained_generations(&self) -> BTreeSet<GenerationId> {
        let mut retained = BTreeSet::new();
        for lease in self.entries.values() {
            retained.insert(lease.owner.generation);
            match lease.location {
                ArrayLocation::Shared {
                    legacy: Some(generation),
                    ..
                }
                | ArrayLocation::Static {
                    legacy: Some(generation),
                    ..
                }
                | ArrayLocation::Character {
                    legacy: Some(generation),
                    ..
                } => {
                    retained.insert(generation);
                }
                _ => {}
            }
        }
        retained
    }

    pub(crate) fn insert(&mut self, lease: ArrayLease) -> Result<ArrayLeaseId, VmError> {
        let next = self.next.checked_add(1).ok_or_else(|| {
            VmError::ScriptFailure(crate::ExecutionFailure::new(
                crate::VmFaultCode::ResourceLimit,
                "array backing identity space is exhausted",
            ))
        })?;
        let id = ArrayLeaseId(next);
        self.entries.insert(id, lease);
        self.next = next;
        self.bump();
        Ok(id)
    }

    /// Removed objects with multiple leases retain one authoritative cell.
    /// A later nested mutation through another lease remains visible to all owners.
    pub(crate) fn detach(&mut self, location: ArrayLocation, cell: &VariableCell) {
        let first = self
            .entries
            .iter()
            .find_map(|(id, lease)| (lease.location == location).then_some(*id));
        let Some(identity) = first else { return };
        self.detached.insert(identity, cell.clone());
        self.bump();
        for lease in self.entries.values_mut() {
            if lease.location == location {
                lease.location = ArrayLocation::Detached(identity);
            }
        }
    }

    pub(crate) fn retain(&mut self, live: &BTreeSet<ArrayLeaseId>) {
        let before = self.entries.len();
        self.entries
            .retain(|id, _| live.contains(id) || self.protected.contains(id));
        if before != self.entries.len() {
            self.bump();
        }
        let retained = self
            .entries
            .values()
            .filter_map(|lease| match lease.location {
                ArrayLocation::Detached(id) => Some(id),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        self.detached.retain(|id, _| retained.contains(id));
    }

    /// Call once with the validated old-to-new permutation before the actual move.
    /// Missing old rows are disposed and detached, not rebound to replacement rows.
    /// The fixed snake CharacterData.Dispose clears its one-dimensional sparse
    /// arrays without changing their length, including already captured REF arrays.
    pub(crate) fn remap_characters(
        &mut self,
        legacy: Option<GenerationId>,
        old: &[VariableMap],
        new_order: &[usize],
    ) -> Result<(), VmError> {
        let mut new_indices = BTreeMap::new();
        for (new, old_index) in new_order.iter().copied().enumerate() {
            if old_index >= old.len() || new_indices.insert(old_index, new).is_some() {
                return Err(invalid("array lease character permutation is invalid"));
            }
        }
        let locations = self
            .entries
            .values()
            .filter_map(|lease| match lease.location {
                location @ ArrayLocation::Character {
                    legacy: scope,
                    index,
                    key,
                } if scope == legacy => Some((location, index, key)),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        // Validate every removed backing before changing any lease identity.
        for (location, index, key) in &locations {
            let mut disposal = self
                .entries
                .values()
                .filter(|lease| lease.location == *location)
                .map(|lease| lease.character_disposal);
            let first = disposal.next();
            if disposal.any(|value| Some(value) != first) {
                return Err(invalid(
                    "shared character backing has inconsistent disposal provenance",
                ));
            }
            let cell = old
                .get(*index)
                .and_then(|row| row.get(key))
                .ok_or_else(|| invalid("captured character array is unavailable"))?;
            if !matches!(
                cell.value_type,
                BytecodeType::Integer | BytecodeType::String
            ) || !(1..=3).contains(&cell.dimensions.len())
            {
                return Err(invalid("captured character array storage is invalid"));
            }
        }
        for (location, index, key) in locations {
            if !new_indices.contains_key(&index) {
                let mut disposed = old[index][&key].clone();
                let clear = self
                    .entries
                    .values()
                    .filter(|lease| lease.location == location)
                    .map(|lease| lease.character_disposal)
                    .collect::<Vec<_>>();
                if clear.first() == Some(&erabasic_bytecode::CharacterArrayDisposal::ClearSparse) {
                    disposed
                        .fill(VmValue::default_for(disposed.value_type))
                        .map_err(|_| {
                            invalid("captured character sparse storage cannot be cleared")
                        })?;
                }
                self.detach(location, &disposed);
            }
        }
        self.bump();
        for lease in self.entries.values_mut() {
            if let ArrayLocation::Character {
                legacy: scope,
                index,
                ..
            } = &mut lease.location
                && *scope == legacy
            {
                *index = *new_indices
                    .get(index)
                    .ok_or_else(|| invalid("removed array lease was not detached"))?;
            }
        }
        Ok(())
    }

    /// Memory migration preserves changed old-generation cells in legacy storage.
    /// Only those backing locations move; unchanged shared cells remain shared.
    pub(crate) fn migrate_generation(
        &mut self,
        old_generation: GenerationId,
        moved_shared: &BTreeSet<SymbolKey>,
        moved_static: &BTreeSet<SymbolKey>,
        moved_character: &BTreeSet<SymbolKey>,
    ) {
        self.bump();
        for lease in self.entries.values_mut() {
            match &mut lease.location {
                ArrayLocation::Shared { legacy, key }
                    if legacy.is_none() && moved_shared.contains(key) =>
                {
                    *legacy = Some(old_generation);
                }
                ArrayLocation::Static { legacy, key }
                    if legacy.is_none() && moved_static.contains(key) =>
                {
                    *legacy = Some(old_generation);
                }
                ArrayLocation::Character { legacy, key, .. }
                    if legacy.is_none() && moved_character.contains(key) =>
                {
                    *legacy = Some(old_generation);
                }
                _ => {}
            }
        }
    }
}

impl Vm {
    pub(crate) fn bit_array_word(
        &self,
        fiber: &Fiber,
        lease: ArrayLeaseId,
        index: usize,
    ) -> Result<i64, VmError> {
        let (_, cell) = self.checked_bit_lease(fiber, lease)?;
        match cell.get(index) {
            Some(VmValue::Integer(value)) => Ok(value),
            _ => Err(invalid("bit-array word is outside its captured storage")),
        }
    }

    /// Prepared word changes commit through the same `VariableCell` type/shape
    /// boundary as ordinary VM place writes. All type/index checks precede the
    /// first write; sparse storage allocation keeps the existing cell contract.
    pub(crate) fn commit_bit_words(
        &mut self,
        fiber: &mut Fiber,
        lease: ArrayLeaseId,
        updates: &[(usize, i64)],
    ) -> Result<(), VmError> {
        let (location, cell) = self.checked_bit_lease(fiber, lease)?;
        let mut previous = None;
        for (index, _) in updates {
            if previous.is_some_and(|old| old >= *index)
                || !matches!(cell.get(*index), Some(VmValue::Integer(_)))
            {
                return Err(invalid("bit-array transaction has invalid word indices"));
            }
            previous = Some(*index);
        }
        if updates.is_empty() {
            return Ok(());
        }
        if updates.len() > 256 {
            return Err(invalid("bit-array chunk exceeds its word budget"));
        }
        self.invalidate_path_memo(fiber.id);
        let cell = self.memory.array_cell_mut(fiber, location)?;
        // VariableValues::set can return Err only for scalar type/index mismatch.
        // Both were checked above; no user code or yield occurs between these
        // checks and writes. Sparse capacity growth retains VariableCell's normal
        // allocator contract; allocation failure is not reclassified as Script.
        for (index, value) in updates {
            cell.set(*index, VmValue::Integer(*value))
                .map_err(|_| invalid("validated bit-array word unexpectedly rejected its write"))?;
        }
        Ok(())
    }

    fn checked_bit_lease<'a>(
        &'a self,
        fiber: &'a Fiber,
        id: ArrayLeaseId,
    ) -> Result<(ArrayLocation, &'a VariableCell), VmError> {
        let lease = self
            .memory
            .array_leases
            .entries
            .get(&id)
            .ok_or_else(|| invalid("bit-array lease is missing"))?;
        let owner = fiber
            .frames
            .last()
            .ok_or_else(|| invalid("bit-array caller is missing"))?;
        if lease.owner.fiber != fiber.id
            || lease.owner.frame != owner.id
            || lease.owner.generation != owner.generation
            || lease.owner.function != owner.function
        {
            return Err(invalid(
                "bit-array lease owner differs from the active call",
            ));
        }
        let cell = self.memory.array_cell(fiber, lease.location)?;
        if cell.value_type != BytecodeType::Integer
            || cell.dimensions.len() != 1
            || cell.len() != lease.length
        {
            return Err(invalid("bit-array backing shape differs from capture"));
        }
        Ok((lease.location, cell))
    }

    pub(crate) fn capture_bit_array(
        &mut self,
        fiber: &Fiber,
        token: &PlaceDescriptor,
        origin: ArrayLeaseOrigin,
    ) -> Result<ArrayLeaseId, VmError> {
        let (_, definition) = self.place_definition(fiber, token)?;
        if definition.value_type != BytecodeType::Integer
            || definition.dimensions.len() != 1
            || !definition.mutable
        {
            return Err(argument(
                "bit operation requires a mutable one-dimensional integer array",
            ));
        }
        if definition.storage == BytecodeStorage::Character {
            return Err(argument(
                "bit operation requires an array REF for character storage",
            ));
        }
        let mut token = token.clone();
        token.indices.clear();
        let place = self.capture_array_reference(fiber, &token, origin)?;
        let id = place.backing.expect("capture creates a backing identity");
        let length = self.memory.array_leases.entries[&id].length;
        if length as u128 * 64 > i64::MAX as u128 {
            self.memory.array_leases.release(id);
            return Err(VmError::ResourceLimit(
                "bit capacity exceeds integer index space",
            ));
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
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
}

impl Memory {
    pub(crate) fn array_location(
        &self,
        generation: GenerationId,
        key: SymbolKey,
        storage: BytecodeStorage,
        index: usize,
    ) -> Result<ArrayLocation, VmError> {
        let legacy = self.legacy.get(&generation);
        Ok(match storage {
            BytecodeStorage::Project => ArrayLocation::Shared {
                legacy: legacy
                    .filter(|memory| memory.shared.contains_key(&key))
                    .map(|_| generation),
                key,
            },
            BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                ArrayLocation::Static {
                    legacy: legacy
                        .filter(|memory| memory.statics.contains_key(&key))
                        .map(|_| generation),
                    key,
                }
            }
            BytecodeStorage::Character => ArrayLocation::Character {
                legacy: legacy
                    .filter(|memory| {
                        memory
                            .characters
                            .get(index)
                            .is_some_and(|row| row.contains_key(&key))
                    })
                    .map(|_| generation),
                index,
                key,
            },
            _ => {
                return Err(argument(
                    "bit operation cannot capture calculated or constant storage",
                ));
            }
        })
    }

    pub(crate) fn array_cell<'a>(
        &'a self,
        fiber: &'a Fiber,
        location: ArrayLocation,
    ) -> Result<&'a VariableCell, VmError> {
        match location {
            ArrayLocation::Shared { legacy, key } => legacy
                .map_or(Some(&self.shared), |generation| {
                    self.legacy.get(&generation).map(|memory| &memory.shared)
                })
                .and_then(|map| map.get(&key)),
            ArrayLocation::Static { legacy, key } => legacy
                .map_or(Some(&self.statics), |generation| {
                    self.legacy.get(&generation).map(|memory| &memory.statics)
                })
                .and_then(|map| map.get(&key)),
            ArrayLocation::Character { legacy, index, key } => legacy
                .map_or(Some(&self.characters), |generation| {
                    self.legacy
                        .get(&generation)
                        .map(|memory| &memory.characters)
                })
                .and_then(|rows| rows.get(index))
                .and_then(|row| row.get(&key)),
            ArrayLocation::Local { frame, key } => {
                return find_frame(fiber, Some(frame), None)?
                    .locals
                    .get(&key)
                    .ok_or_else(|| invalid("captured local array is unavailable"));
            }
            ArrayLocation::Detached(id) => self.array_leases.detached.get(&id),
        }
        .ok_or_else(|| invalid("captured array backing is unavailable"))
    }

    pub(crate) fn array_cell_mut<'a>(
        &'a mut self,
        fiber: &'a mut Fiber,
        location: ArrayLocation,
    ) -> Result<&'a mut VariableCell, VmError> {
        match location {
            ArrayLocation::Shared { legacy, key } => match legacy {
                Some(generation) => self
                    .legacy
                    .get_mut(&generation)
                    .and_then(|memory| memory.shared.get_mut(&key)),
                None => self.shared.get_mut(&key),
            },
            ArrayLocation::Static { legacy, key } => match legacy {
                Some(generation) => self
                    .legacy
                    .get_mut(&generation)
                    .and_then(|memory| memory.statics.get_mut(&key)),
                None => self.statics.get_mut(&key),
            },
            ArrayLocation::Character { legacy, index, key } => match legacy {
                Some(generation) => self
                    .legacy
                    .get_mut(&generation)
                    .and_then(|memory| memory.characters.get_mut(index))
                    .and_then(|row| row.get_mut(&key)),
                None => self
                    .characters
                    .get_mut(index)
                    .and_then(|row| row.get_mut(&key)),
            },
            ArrayLocation::Local { frame, key } => {
                return find_frame_mut(fiber, Some(frame), None)?
                    .locals
                    .get_mut(&key)
                    .ok_or_else(|| invalid("captured local array is unavailable"));
            }
            ArrayLocation::Detached(id) => self.array_leases.detached.get_mut(&id),
        }
        .ok_or_else(|| invalid("captured array backing is unavailable"))
    }
}

/// Runtime-only prepare/commit guard. Do not serialize this as a restore token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArrayLeaseStamp {
    revision: u64,
    cells: Vec<(ArrayLocation, u64)>,
}
impl Vm {
    pub(crate) fn array_lease_stamp(&self) -> Result<ArrayLeaseStamp, VmError> {
        let book = &self.memory.array_leases;
        if book.revision == u64::MAX {
            return Err(invalid("array lease revision space is exhausted"));
        }
        let mut cells = BTreeMap::new();
        for (id, lease) in &book.entries {
            let fiber = self.fibers.get(&lease.owner.fiber);
            if fiber.is_none_or(|fiber| {
                !fiber
                    .frames
                    .iter()
                    .any(|frame| frame.id == lease.owner.frame)
            }) && book.protected.contains(id)
            {
                // An isolated candidate has no parent's local frames. Its inherited
                // leases are protected, and the parent's full stamp gates commit.
                continue;
            }
            let fiber = fiber.ok_or_else(|| invalid("array lease stamp owner is missing"))?;
            let cell = self.memory.array_cell(fiber, lease.location)?;
            if cell.revision() == u64::MAX {
                return Err(invalid("array backing revision space is exhausted"));
            }
            cells.insert(lease.location, cell.revision());
        }
        Ok(ArrayLeaseStamp {
            revision: book.revision,
            cells: cells.into_iter().collect(),
        })
    }
    pub(crate) fn validate_array_lease_stamp(
        &self,
        stamp: &ArrayLeaseStamp,
    ) -> Result<(), VmError> {
        if &self.array_lease_stamp()? != stamp {
            return Err(invalid(
                "prepared memory belongs to a stale array lease timeline",
            ));
        }
        Ok(())
    }
}
impl Memory {
    pub(crate) fn validate_array_leases(
        &self,
        fibers: &BTreeMap<FiberId, Fiber>,
        expected: &BTreeMap<ArrayLeaseId, ArrayLeaseOwner>,
        maximum: usize,
    ) -> Result<(), VmError> {
        let book = &self.array_leases;
        let roots = expected
            .keys()
            .chain(book.protected.iter())
            .copied()
            .collect::<BTreeSet<_>>();
        if book.entries.keys().copied().collect::<BTreeSet<_>>() != roots
            || book.entries.len()
                > maximum
                    .saturating_mul(
                        fibers
                            .values()
                            .map(|fiber| fiber.frames.len())
                            .sum::<usize>()
                            .max(1),
                    )
                    .saturating_add(book.protected.len())
        {
            return Err(invalid(
                "array leases do not exactly match reachable call roots",
            ));
        }
        let mut detached = BTreeSet::new();
        for (id, lease) in &book.entries {
            if id.0 == 0 || id.0 > book.next {
                return Err(invalid("array lease counter is invalid"));
            }
            if let ArrayLocation::Detached(backing) = lease.location {
                if backing.0 == 0 || backing.0 > book.next {
                    return Err(invalid("detached backing identity is invalid"));
                }
                detached.insert(backing);
            }
            let Some(owner) = expected.get(id) else {
                continue;
            };
            if lease.owner != *owner {
                return Err(invalid("array lease owner/origin differs"));
            }
            let fiber = fibers
                .get(&owner.fiber)
                .ok_or_else(|| invalid("array lease fiber is missing"))?;
            let position = fiber
                .frames
                .iter()
                .position(|frame| {
                    frame.id == owner.frame
                        && frame.generation == owner.generation
                        && frame.function == owner.function
                })
                .ok_or_else(|| invalid("array lease frame is missing"))?;
            if let ArrayLocation::Local { frame, .. } = lease.location
                && !fiber.frames[..=position]
                    .iter()
                    .any(|owner| owner.id == frame)
            {
                return Err(invalid(
                    "array lease local backing is outside the live owner chain",
                ));
            }
            let cell = self.array_cell(fiber, lease.location)?;
            if !cell.storage_is_valid()
                || !matches!(
                    cell.value_type,
                    BytecodeType::Integer | BytecodeType::String
                )
                || !(1..=3).contains(&cell.dimensions.len())
                || cell.value_type != lease.value_type
                || cell.dimensions != lease.dimensions
                || cell.len() != lease.length
            {
                return Err(invalid("array lease storage shape or capacity differs"));
            }
        }
        if detached != book.detached.keys().copied().collect() {
            return Err(invalid(
                "array lease book contains an orphan detached backing",
            ));
        }
        Ok(())
    }
}

impl Vm {
    #[allow(clippy::too_many_lines)] // Validation keeps lease and symbol invariants in one audit pass.
    pub(crate) fn validate_bit_lease_symbols(&self) -> Result<(), VmError> {
        for (id, lease) in &self.memory.array_leases.entries {
            if self.memory.array_leases.protected.contains(id) {
                continue;
            }
            if matches!(
                lease.owner.origin,
                ArrayLeaseOrigin::UserBytecode { .. } | ArrayLeaseOrigin::UserForm { .. }
            ) {
                continue;
            }
            let program = self
                .generations
                .get(&lease.owner.generation)
                .ok_or_else(|| invalid("BIT input generation is missing"))?;
            let input = program
                .artifact
                .globals
                .iter()
                .find(|definition| definition.key == lease.input)
                .ok_or_else(|| invalid("BIT input symbol is missing"))?;
            if !input.mutable
                || input.value_type != BytecodeType::Integer
                || input.dimensions.len() != 1
                || input
                    .owner
                    .is_some_and(|owner| owner != lease.owner.function)
                    && input.storage != BytecodeStorage::FunctionPersistent
            {
                return Err(invalid("BIT input symbol is outside its owner schema"));
            }
            let reference = program.is_reference_variable(input.key);
            let (generation, key, storage, local_owner) = match lease.location {
                ArrayLocation::Shared { legacy, key } => (
                    legacy.unwrap_or(self.current_generation),
                    key,
                    BytecodeStorage::Project,
                    None,
                ),
                ArrayLocation::Static { legacy, key } => (
                    legacy.unwrap_or(self.current_generation),
                    key,
                    BytecodeStorage::FunctionStatic,
                    None,
                ),
                ArrayLocation::Character { legacy, key, .. } => (
                    legacy.unwrap_or(self.current_generation),
                    key,
                    BytecodeStorage::Character,
                    None,
                ),
                ArrayLocation::Local { frame, key } => {
                    let frame = self
                        .fibers
                        .get(&lease.owner.fiber)
                        .and_then(|fiber| fiber.frames.iter().find(|owner| owner.id == frame))
                        .ok_or_else(|| invalid("BIT local symbol owner is missing"))?;
                    (
                        frame.generation,
                        key,
                        BytecodeStorage::FunctionLocal,
                        Some(frame.function),
                    )
                }
                ArrayLocation::Detached(_) => {
                    if !reference {
                        return Err(invalid(
                            "a direct BIT token cannot own detached character storage",
                        ));
                    }
                    continue;
                }
            };
            if !reference && key != input.key {
                return Err(invalid("BIT direct input backing key differs"));
            }
            if !reference && storage == BytecodeStorage::Character {
                return Err(invalid("direct character BIT capture is invalid"));
            }
            let target = self
                .generations
                .get(&generation)
                .and_then(|program| {
                    program
                        .artifact
                        .globals
                        .iter()
                        .find(|definition| definition.key == key)
                })
                .ok_or_else(|| invalid("BIT captured backing generation or symbol is missing"))?;
            let storage_matches = target.storage == storage
                || storage == BytecodeStorage::FunctionStatic
                    && target.storage == BytecodeStorage::FunctionPersistent;
            if !storage_matches
                || !target.mutable
                || target.value_type != BytecodeType::Integer
                || target.dimensions.len() != 1
                || local_owner.is_some_and(|owner| target.owner != Some(owner))
            {
                return Err(invalid("BIT captured backing symbol schema differs"));
            }
        }
        Ok(())
    }
}
