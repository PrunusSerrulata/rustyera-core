#[allow(clippy::wildcard_imports)]
use super::*;
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
