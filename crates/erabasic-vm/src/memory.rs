use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
use std::mem::size_of;
use std::ops::{Deref, DerefMut};

use erabasic_bytecode::{
    BytecodeArtifact, BytecodeConstant, BytecodeGlobal, BytecodeStorage, BytecodeType, SymbolKey,
};
use erabasic_data::{CharacterSelection, CharacterTemplate, RuntimeDefaults};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{GenerationId, PlaceDescriptor, VmValue};

mod storage_serde;
mod values;

#[cfg(test)]
use storage_serde::SparseVariableValues;
use values::{VariableValues, collect_sparse_values, collect_values};

pub(crate) struct SymbolKeyHasher(u64);

impl Default for SymbolKeyHasher {
    fn default() -> Self {
        Self(0x517c_c1b7_2722_0a95)
    }
}

impl Hasher for SymbolKeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut hash = self.0;
        for chunk in bytes.chunks(8) {
            let mut word = [0_u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            hash ^= u64::from_le_bytes(word);
            hash = hash.rotate_left(27).wrapping_mul(0x3c79_ac49_2ba7_b653);
        }
        self.0 = hash;
    }
}

pub(crate) type VariableHashMap =
    HashMap<SymbolKey, VariableCell, BuildHasherDefault<SymbolKeyHasher>>;

pub(crate) fn shared_definition<'a>(
    artifact: &'a BytecodeArtifact,
    name: &str,
) -> Option<&'a BytecodeGlobal> {
    artifact.globals.iter().find(|definition| {
        definition.owner.is_none()
            && matches!(
                definition.storage,
                BytecodeStorage::Project | BytecodeStorage::Constant | BytecodeStorage::Calculated
            )
            && definition.name.eq_ignore_ascii_case(name)
    })
}

pub(crate) fn character_definition<'a>(
    artifact: &'a BytecodeArtifact,
    name: &str,
) -> Option<&'a BytecodeGlobal> {
    artifact.globals.iter().find(|definition| {
        definition.owner.is_none()
            && definition.storage == BytecodeStorage::Character
            && definition.name.eq_ignore_ascii_case(name)
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct VariableMap(VariableHashMap);

impl Deref for VariableMap {
    type Target = VariableHashMap;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for VariableMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl FromIterator<(SymbolKey, VariableCell)> for VariableMap {
    fn from_iter<T: IntoIterator<Item = (SymbolKey, VariableCell)>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl Serialize for VariableMap {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap as _;

        let mut values = self.0.iter().collect::<Vec<_>>();
        values.sort_unstable_by_key(|(key, _)| **key);
        let mut map = serializer.serialize_map(Some(values.len()))?;
        for (key, value) in values {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for VariableMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = BTreeMap::<SymbolKey, VariableCell>::deserialize(deserializer)?;
        Ok(values.into_iter().collect())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct VariableCell {
    pub value_type: BytecodeType,
    pub dimensions: Vec<u64>,
    values: VariableValues,
    revision: u64,
}

impl PartialEq for VariableCell {
    fn eq(&self, other: &Self) -> bool {
        self.value_type == other.value_type
            && self.dimensions == other.dimensions
            && self.values == other.values
    }
}

impl Eq for VariableCell {}

impl VariableCell {
    pub fn new(definition: &BytecodeGlobal) -> Self {
        let length = element_count(&definition.dimensions).unwrap_or(0);
        // Era projects declare many large arrays whose initial state is almost entirely the
        // language default. Keeping those arrays sparse is important for project and character
        // storage too: eagerly allocating every zero or empty string can consume gigabytes before
        // the title script executes. Dense initialization remains preferable for tables that are
        // already at least half populated, and RANDDATA must expose a contiguous integer slice to
        // the random-state native service.
        let densely_initialized = definition.initial_values.len().saturating_mul(2) >= length;
        let requires_contiguous_storage = definition.name.eq_ignore_ascii_case("RANDDATA");
        let mut values = if densely_initialized || requires_contiguous_storage {
            VariableValues::with_default(definition.value_type, length)
        } else {
            VariableValues::with_lazy_default(definition.value_type, length)
        };
        for (index, value) in definition.initial_values.iter().enumerate() {
            let value = match value {
                BytecodeConstant::Integer(value) => VmValue::Integer(*value),
                BytecodeConstant::String(value) => VmValue::String(value.clone()),
            };
            values
                .set(index, value)
                .expect("validated global initial value matches its declaration");
        }
        Self {
            value_type: definition.value_type,
            dimensions: definition.dimensions.clone(),
            values,
            revision: 0,
        }
    }

    #[inline]
    pub fn read(&self, indices: &[u64]) -> Result<VmValue, String> {
        let offset = flatten(&self.dimensions, indices)?;
        self.values
            .get(offset)
            .ok_or_else(|| "variable offset is outside its storage".into())
    }

    #[inline]
    pub fn write(&mut self, indices: &[u64], value: VmValue) -> Result<(), String> {
        let offset = flatten(&self.dimensions, indices)?;
        self.values.set(offset, value)?;
        self.bump_revision();
        Ok(())
    }

    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            .saturating_add(self.dimensions.capacity().saturating_mul(size_of::<u64>()))
            .saturating_add(self.values.retained_allocation_bytes())
    }

    #[inline]
    pub(crate) fn first(&self) -> Option<VmValue> {
        self.values.get(0)
    }

    #[inline]
    pub(crate) fn get(&self, index: usize) -> Option<VmValue> {
        self.values.get(index)
    }

    #[inline]
    pub(crate) fn set(&mut self, index: usize, value: VmValue) -> Result<(), String> {
        self.values.set(index, value)?;
        self.bump_revision();
        Ok(())
    }

    pub(crate) fn fill(&mut self, value: VmValue) -> Result<(), String> {
        self.values.fill(value)?;
        self.bump_revision();
        Ok(())
    }

    pub(crate) fn fill_range(
        &mut self,
        start: usize,
        end: usize,
        value: VmValue,
    ) -> Result<(), String> {
        self.values.fill_range(start, end, value)?;
        self.bump_revision();
        Ok(())
    }

    pub(crate) fn to_values(&self) -> Vec<VmValue> {
        self.values.to_vm_values()
    }

    pub(crate) fn to_values_range(&self, start: usize, end: usize) -> Option<Vec<VmValue>> {
        self.values.to_values_range(start, end)
    }

    pub(crate) fn integers(&self) -> Option<&[i64]> {
        match &self.values {
            VariableValues::Integers(values) => Some(values),
            _ => None,
        }
    }

    pub(crate) fn integers_mut(&mut self) -> Option<&mut [i64]> {
        self.bump_revision();
        match &mut self.values {
            VariableValues::Integers(values) => Some(values),
            _ => None,
        }
    }

    pub(crate) fn replace_values(&mut self, values: Vec<VmValue>) -> Result<(), String> {
        if values.len() != self.len()
            || values
                .iter()
                .any(|value| value.value_type() != self.value_type)
        {
            return Err("array replacement differs from its storage shape or type".into());
        }
        let mut replacement = VariableValues::with_default(self.value_type, values.len());
        for (index, value) in values.into_iter().enumerate() {
            replacement.set(index, value)?;
        }
        self.values = replacement;
        self.bump_revision();
        Ok(())
    }

    pub(crate) fn replace_contents_from(&mut self, source: &Self) -> Result<(), String> {
        if self.value_type != source.value_type || self.dimensions != source.dimensions {
            return Err("array replacement differs from its storage shape or type".into());
        }
        self.values = source.values.clone();
        self.bump_revision();
        Ok(())
    }

    fn replace_values_from_slice(&mut self, values: &[VmValue]) -> Result<(), String> {
        if values.len() != self.len() {
            return Err("array replacement differs from its storage shape or type".into());
        }
        let replacement = match &self.values {
            VariableValues::Integers(_) => {
                VariableValues::Integers(collect_values(values, |value| {
                    let VmValue::Integer(value) = value else {
                        return None;
                    };
                    Some(*value)
                })?)
            }
            VariableValues::Strings(_) => {
                VariableValues::Strings(collect_values(values, |value| {
                    let VmValue::String(value) = value else {
                        return None;
                    };
                    Some(value.clone())
                })?)
            }
            VariableValues::IntegerPlaces(_) => {
                VariableValues::IntegerPlaces(collect_values(values, |value| {
                    let VmValue::IntegerPlace(value) = value else {
                        return None;
                    };
                    Some(value.as_ref().clone())
                })?)
            }
            VariableValues::StringPlaces(_) => {
                VariableValues::StringPlaces(collect_values(values, |value| {
                    let VmValue::StringPlace(value) = value else {
                        return None;
                    };
                    Some(value.as_ref().clone())
                })?)
            }
            VariableValues::SparseIntegers { length, .. } => VariableValues::SparseIntegers {
                length: *length,
                entries: collect_sparse_values(values, |value| {
                    let VmValue::Integer(value) = value else {
                        return None;
                    };
                    Some(*value)
                })?,
            },
            VariableValues::SparseStrings { length, .. } => VariableValues::SparseStrings {
                length: *length,
                entries: collect_sparse_values(values, |value| {
                    let VmValue::String(value) = value else {
                        return None;
                    };
                    Some(value.clone())
                })?,
            },
            VariableValues::SparseIntegerPlaces { length, .. } => {
                VariableValues::SparseIntegerPlaces {
                    length: *length,
                    entries: collect_sparse_values(values, |value| {
                        let VmValue::IntegerPlace(value) = value else {
                            return None;
                        };
                        Some(value.as_ref().clone())
                    })?,
                }
            }
            VariableValues::SparseStringPlaces { length, .. } => {
                VariableValues::SparseStringPlaces {
                    length: *length,
                    entries: collect_sparse_values(values, |value| {
                        let VmValue::StringPlace(value) = value else {
                            return None;
                        };
                        Some(value.as_ref().clone())
                    })?,
                }
            }
        };
        self.values = replacement;
        self.bump_revision();
        Ok(())
    }

    pub(crate) fn replace_shape(
        &mut self,
        value_type: BytecodeType,
        dimensions: Vec<u64>,
        values: Vec<VmValue>,
    ) -> Result<(), String> {
        self.value_type = value_type;
        self.dimensions = dimensions;
        self.values = VariableValues::with_default(value_type, values.len());
        self.replace_values(values)
    }

    pub(crate) fn storage_is_valid(&self) -> bool {
        self.values.value_type() == self.value_type
    }

    pub(crate) const fn revision(&self) -> u64 {
        self.revision
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    pub(crate) fn materialize_snapshot(&mut self) -> Result<(), String> {
        self.values.materialize()
    }

    pub fn migrate(&self, definition: &BytecodeGlobal) -> Self {
        let mut target = Self::new(definition);
        let target_len = target.len();
        for target_offset in 0..target_len {
            let coordinates = unflatten(&target.dimensions, target_offset);
            if coordinates
                .iter()
                .zip(&self.dimensions)
                .all(|(index, length)| index < length)
                && coordinates.len() == self.dimensions.len()
                && let Ok(source_offset) = flatten(&self.dimensions, &coordinates)
                && let Some(value) = self.get(source_offset)
            {
                target
                    .set(target_offset, value)
                    .expect("migration only copies identical variable types");
            }
        }
        target
    }

    pub fn overlay(&mut self, dimensions: &[u64], values: &[VmValue]) -> Result<(), String> {
        if dimensions == self.dimensions && values.len() == self.len() {
            return self.replace_values_from_slice(values);
        }
        for (source_offset, value) in values.iter().enumerate() {
            let coordinates = unflatten(dimensions, source_offset);
            if coordinates.len() != self.dimensions.len()
                || !coordinates
                    .iter()
                    .zip(&self.dimensions)
                    .all(|(index, length)| index < length)
            {
                continue;
            }
            let target_offset = flatten(&self.dimensions, &coordinates)?;
            if value.value_type() != self.value_type {
                return Err("saved variable value type does not match its schema".into());
            }
            if target_offset < self.len() {
                self.set(target_offset, value.clone())?;
            }
        }
        Ok(())
    }

    pub fn overlay_sparse(
        &mut self,
        dimensions: &[u64],
        values: &[(u64, VmValue)],
    ) -> Result<(), String> {
        let source_len = element_count(dimensions)
            .ok_or_else(|| "saved variable dimensions exceed this platform".to_owned())?;
        if dimensions != self.dimensions {
            let default = match self.value_type {
                BytecodeType::Integer => VmValue::Integer(0),
                BytecodeType::String => VmValue::String(String::new()),
                BytecodeType::IntegerPlace | BytecodeType::StringPlace => {
                    return Err("saved variables cannot contain places".into());
                }
            };
            let mut dense = vec![default; source_len];
            for (index, value) in values {
                let index = usize::try_from(*index)
                    .map_err(|_| "saved variable offset exceeds this platform")?;
                let target = dense
                    .get_mut(index)
                    .ok_or_else(|| "saved variable offset exceeds its dimensions".to_owned())?;
                if value.value_type() != self.value_type {
                    return Err("saved variable value type does not match its schema".into());
                }
                target.clone_from(value);
            }
            return self.overlay(dimensions, &dense);
        }
        if source_len != self.len() {
            return Err("saved variable dimensions differ from their element count".into());
        }
        let mut replacement = match &self.values {
            VariableValues::Integers(_) => VariableValues::Integers(vec![0; source_len]),
            VariableValues::Strings(_) => VariableValues::Strings(vec![String::new(); source_len]),
            VariableValues::SparseIntegers { .. } => VariableValues::SparseIntegers {
                length: source_len,
                entries: Vec::new(),
            },
            VariableValues::SparseStrings { .. } => VariableValues::SparseStrings {
                length: source_len,
                entries: Vec::new(),
            },
            VariableValues::IntegerPlaces(_)
            | VariableValues::StringPlaces(_)
            | VariableValues::SparseIntegerPlaces { .. }
            | VariableValues::SparseStringPlaces { .. } => {
                return Err("saved variables cannot contain places".into());
            }
        };
        for (index, value) in values {
            let index = usize::try_from(*index)
                .map_err(|_| "saved variable offset exceeds this platform")?;
            if index >= source_len {
                return Err("saved variable offset exceeds its dimensions".into());
            }
            replacement.set(index, value.clone())?;
        }
        self.values = replacement;
        self.bump_revision();
        Ok(())
    }
}

fn element_count(dimensions: &[u64]) -> Option<usize> {
    dimensions
        .iter()
        .copied()
        .try_fold(1u64, u64::checked_mul)
        .and_then(|length| usize::try_from(length).ok())
}

#[inline]
fn flatten(dimensions: &[u64], indices: &[u64]) -> Result<usize, String> {
    if indices.len() > dimensions.len() {
        return Err("too many variable indices".into());
    }
    if dimensions.is_empty() {
        return Ok(0);
    }
    if let [length] = dimensions {
        let index = indices.first().copied().unwrap_or(0);
        if index >= *length {
            return Err(format!(
                "index {index} is outside dimension 0 of length {length}"
            ));
        }
        return usize::try_from(index).map_err(|_| "variable offset exceeds this platform".into());
    }
    let mut offset = 0u64;
    for (dimension, length) in dimensions.iter().enumerate() {
        let index = indices.get(dimension).copied().unwrap_or(0);
        if index >= *length {
            return Err(format!(
                "index {index} is outside dimension {dimension} of length {length}"
            ));
        }
        offset = offset
            .checked_mul(*length)
            .and_then(|value| value.checked_add(index))
            .ok_or_else(|| "variable offset overflow".to_owned())?;
    }
    usize::try_from(offset).map_err(|_| "variable offset exceeds this platform".into())
}

fn unflatten(dimensions: &[u64], mut offset: usize) -> Vec<u64> {
    let mut result = vec![0; dimensions.len()];
    for dimension in (0..dimensions.len()).rev() {
        let length = usize::try_from(dimensions[dimension]).unwrap_or(usize::MAX);
        if length != 0 {
            result[dimension] = (offset % length) as u64;
            offset /= length;
        }
    }
    result
}

mod store;
#[cfg(test)]
mod tests;

pub(crate) use store::Memory;
