#[allow(clippy::wildcard_imports)]
use super::*;
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
