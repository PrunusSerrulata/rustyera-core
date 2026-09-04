#[allow(clippy::wildcard_imports)]
use super::*;
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
