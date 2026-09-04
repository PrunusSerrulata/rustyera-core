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

mod bit_arrays;
mod storage;
#[cfg(test)]
mod tests;
mod validation;

pub(crate) use validation::ArrayLeaseStamp;
