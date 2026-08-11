use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
use std::ops::{Deref, DerefMut};

use erabasic_bytecode::{
    BytecodeArtifact, BytecodeConstant, BytecodeGlobal, BytecodeStorage, BytecodeType, SymbolKey,
};
use erabasic_data::{CharacterSelection, CharacterTemplate, RuntimeDefaults};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer, de::Error as _, ser::SerializeSeq as _,
};

use crate::{GenerationId, PlaceDescriptor, VmValue};

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

/// Variable storage is specialized by `EraBasic` value type.
///
/// Most game memory consists of large integer arrays, especially character
/// variables. Keeping every element in the public `VmValue` enum would retain
/// the enum's largest payload and waste two thirds of each integer allocation.
/// Large function-owned arrays additionally keep only non-default entries until
/// an operation truly needs dense storage, matching the reference's lazy local
/// allocation. The VM converts at its boundary, preserving the public value
/// model and snapshot semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
enum VariableValues {
    Integers(Vec<i64>),
    Strings(Vec<String>),
    IntegerPlaces(Vec<PlaceDescriptor>),
    StringPlaces(Vec<PlaceDescriptor>),
    SparseIntegers {
        length: usize,
        entries: Vec<(usize, i64)>,
    },
    SparseStrings {
        length: usize,
        entries: Vec<(usize, String)>,
    },
    SparseIntegerPlaces {
        length: usize,
        entries: Vec<(usize, PlaceDescriptor)>,
    },
    SparseStringPlaces {
        length: usize,
        entries: Vec<(usize, PlaceDescriptor)>,
    },
}

impl VariableValues {
    const SPARSE_DEFAULT_MINIMUM_LENGTH: usize = 256;

    fn with_default(value_type: BytecodeType, length: usize) -> Self {
        match value_type {
            BytecodeType::Integer => Self::Integers(vec![0; length]),
            BytecodeType::String => Self::Strings(vec![String::new(); length]),
            BytecodeType::IntegerPlace => {
                Self::IntegerPlaces(vec![PlaceDescriptor::default(); length])
            }
            BytecodeType::StringPlace => {
                Self::StringPlaces(vec![PlaceDescriptor::default(); length])
            }
        }
    }

    fn with_lazy_default(value_type: BytecodeType, length: usize) -> Self {
        if length < Self::SPARSE_DEFAULT_MINIMUM_LENGTH {
            return Self::with_default(value_type, length);
        }
        match value_type {
            BytecodeType::Integer => Self::SparseIntegers {
                length,
                entries: Vec::new(),
            },
            BytecodeType::String => Self::SparseStrings {
                length,
                entries: Vec::new(),
            },
            BytecodeType::IntegerPlace => Self::SparseIntegerPlaces {
                length,
                entries: Vec::new(),
            },
            BytecodeType::StringPlace => Self::SparseStringPlaces {
                length,
                entries: Vec::new(),
            },
        }
    }

    const fn value_type(&self) -> BytecodeType {
        match self {
            Self::Integers(_) | Self::SparseIntegers { .. } => BytecodeType::Integer,
            Self::Strings(_) | Self::SparseStrings { .. } => BytecodeType::String,
            Self::IntegerPlaces(_) | Self::SparseIntegerPlaces { .. } => BytecodeType::IntegerPlace,
            Self::StringPlaces(_) | Self::SparseStringPlaces { .. } => BytecodeType::StringPlace,
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Integers(values) => values.len(),
            Self::Strings(values) => values.len(),
            Self::IntegerPlaces(values) | Self::StringPlaces(values) => values.len(),
            Self::SparseIntegers { length, .. }
            | Self::SparseStrings { length, .. }
            | Self::SparseIntegerPlaces { length, .. }
            | Self::SparseStringPlaces { length, .. } => *length,
        }
    }

    fn to_values_range(&self, start: usize, end: usize) -> Option<Vec<VmValue>> {
        (start <= end && end <= self.len())
            .then(|| (start..end).filter_map(|index| self.get(index)).collect())
    }

    #[inline]
    fn get(&self, index: usize) -> Option<VmValue> {
        match self {
            Self::Integers(values) => values.get(index).copied().map(VmValue::Integer),
            Self::Strings(values) => values.get(index).cloned().map(VmValue::String),
            Self::IntegerPlaces(values) => values
                .get(index)
                .cloned()
                .map(Box::new)
                .map(VmValue::IntegerPlace),
            Self::StringPlaces(values) => values
                .get(index)
                .cloned()
                .map(Box::new)
                .map(VmValue::StringPlace),
            Self::SparseIntegers { length, entries } => (index < *length).then(|| {
                VmValue::Integer(sparse_value(entries, index).copied().unwrap_or_default())
            }),
            Self::SparseStrings { length, entries } => (index < *length).then(|| {
                VmValue::String(sparse_value(entries, index).cloned().unwrap_or_default())
            }),
            Self::SparseIntegerPlaces { length, entries } => (index < *length).then(|| {
                VmValue::IntegerPlace(Box::new(
                    sparse_value(entries, index).cloned().unwrap_or_default(),
                ))
            }),
            Self::SparseStringPlaces { length, entries } => (index < *length).then(|| {
                VmValue::StringPlace(Box::new(
                    sparse_value(entries, index).cloned().unwrap_or_default(),
                ))
            }),
        }
    }

    #[inline]
    fn set(&mut self, index: usize, value: VmValue) -> Result<(), String> {
        if value.value_type() != self.value_type() {
            return Err(format!(
                "variable expects {:?}, found {:?}",
                self.value_type(),
                value.value_type()
            ));
        }
        match (&mut *self, &value) {
            (Self::SparseIntegers { length, entries }, VmValue::Integer(value)) => {
                return set_sparse_slot(*length, entries, index, *value);
            }
            (Self::SparseStrings { length, entries }, VmValue::String(value)) => {
                return set_sparse_slot(*length, entries, index, value.clone());
            }
            (Self::SparseIntegerPlaces { length, entries }, VmValue::IntegerPlace(value))
            | (Self::SparseStringPlaces { length, entries }, VmValue::StringPlace(value)) => {
                return set_sparse_slot(*length, entries, index, value.as_ref().clone());
            }
            _ => {}
        }
        self.materialize()?;
        match (self, value) {
            (Self::Integers(values), VmValue::Integer(value)) => set_slot(values, index, value),
            (Self::Strings(values), VmValue::String(value)) => set_slot(values, index, value),
            (Self::IntegerPlaces(values), VmValue::IntegerPlace(value))
            | (Self::StringPlaces(values), VmValue::StringPlace(value)) => {
                set_slot(values, index, *value)
            }
            _ => unreachable!("materialized storage matches the checked value type"),
        }
    }

    fn fill(&mut self, value: VmValue) -> Result<(), String> {
        if value.value_type() != self.value_type() {
            return Err(format!(
                "variable expects {:?}, found {:?}",
                self.value_type(),
                value.value_type()
            ));
        }
        let cleared_sparse = match (&mut *self, &value) {
            (Self::SparseIntegers { entries, .. }, VmValue::Integer(0)) => {
                entries.clear();
                true
            }
            (Self::SparseStrings { entries, .. }, VmValue::String(value)) if value.is_empty() => {
                entries.clear();
                true
            }
            (Self::SparseIntegerPlaces { entries, .. }, VmValue::IntegerPlace(value))
            | (Self::SparseStringPlaces { entries, .. }, VmValue::StringPlace(value))
                if value.as_ref() == &PlaceDescriptor::default() =>
            {
                entries.clear();
                true
            }
            _ => false,
        };
        if cleared_sparse {
            return Ok(());
        }
        self.materialize()?;
        match (self, value) {
            (Self::Integers(values), VmValue::Integer(value)) => values.fill(value),
            (Self::Strings(values), VmValue::String(value)) => values.fill(value),
            (Self::IntegerPlaces(values), VmValue::IntegerPlace(value))
            | (Self::StringPlaces(values), VmValue::StringPlace(value)) => values.fill(*value),
            _ => unreachable!("materialized storage matches the checked value type"),
        }
        Ok(())
    }

    fn fill_range(&mut self, start: usize, end: usize, value: VmValue) -> Result<(), String> {
        if start > end || end > self.len() {
            return Err("variable fill range is outside its storage".into());
        }
        if value.value_type() != self.value_type() {
            return Err(format!(
                "variable expects {:?}, found {:?}",
                self.value_type(),
                value.value_type()
            ));
        }
        let cleared_sparse = match (&mut *self, &value) {
            (Self::SparseIntegers { entries, .. }, VmValue::Integer(0)) => {
                clear_sparse_range(entries, start, end);
                true
            }
            (Self::SparseStrings { entries, .. }, VmValue::String(value)) if value.is_empty() => {
                clear_sparse_range(entries, start, end);
                true
            }
            (Self::SparseIntegerPlaces { entries, .. }, VmValue::IntegerPlace(value))
            | (Self::SparseStringPlaces { entries, .. }, VmValue::StringPlace(value))
                if value.as_ref() == &PlaceDescriptor::default() =>
            {
                clear_sparse_range(entries, start, end);
                true
            }
            _ => false,
        };
        if cleared_sparse {
            return Ok(());
        }
        self.materialize()?;
        match (self, value) {
            (Self::Integers(values), VmValue::Integer(value)) => values[start..end].fill(value),
            (Self::Strings(values), VmValue::String(value)) => values[start..end].fill(value),
            (Self::IntegerPlaces(values), VmValue::IntegerPlace(value))
            | (Self::StringPlaces(values), VmValue::StringPlace(value)) => {
                values[start..end].fill(*value);
            }
            _ => unreachable!("materialized storage matches the checked value type"),
        }
        Ok(())
    }

    fn to_vm_values(&self) -> Vec<VmValue> {
        match self {
            Self::Integers(values) => values.iter().copied().map(VmValue::Integer).collect(),
            Self::Strings(values) => values.iter().cloned().map(VmValue::String).collect(),
            Self::IntegerPlaces(values) => values
                .iter()
                .cloned()
                .map(Box::new)
                .map(VmValue::IntegerPlace)
                .collect(),
            Self::StringPlaces(values) => values
                .iter()
                .cloned()
                .map(Box::new)
                .map(VmValue::StringPlace)
                .collect(),
            Self::SparseIntegers { length, entries } => (0..*length)
                .map(|index| sparse_value(entries, index).copied().unwrap_or_default())
                .map(VmValue::Integer)
                .collect(),
            Self::SparseStrings { length, entries } => (0..*length)
                .map(|index| sparse_value(entries, index).cloned().unwrap_or_default())
                .map(VmValue::String)
                .collect(),
            Self::SparseIntegerPlaces { length, entries } => (0..*length)
                .map(|index| sparse_value(entries, index).cloned().unwrap_or_default())
                .map(Box::new)
                .map(VmValue::IntegerPlace)
                .collect(),
            Self::SparseStringPlaces { length, entries } => (0..*length)
                .map(|index| sparse_value(entries, index).cloned().unwrap_or_default())
                .map(Box::new)
                .map(VmValue::StringPlace)
                .collect(),
        }
    }

    fn materialize(&mut self) -> Result<(), String> {
        match self {
            Self::SparseIntegers { length, entries } => {
                let mut values = try_default_vector::<i64>(*length)?;
                apply_sparse_entries(&mut values, std::mem::take(entries))?;
                *self = Self::Integers(values);
            }
            Self::SparseStrings { length, entries } => {
                let mut values = try_default_vector::<String>(*length)?;
                apply_sparse_entries(&mut values, std::mem::take(entries))?;
                *self = Self::Strings(values);
            }
            Self::SparseIntegerPlaces { length, entries } => {
                let mut values = try_default_vector::<PlaceDescriptor>(*length)?;
                apply_sparse_entries(&mut values, std::mem::take(entries))?;
                *self = Self::IntegerPlaces(values);
            }
            Self::SparseStringPlaces { length, entries } => {
                let mut values = try_default_vector::<PlaceDescriptor>(*length)?;
                apply_sparse_entries(&mut values, std::mem::take(entries))?;
                *self = Self::StringPlaces(values);
            }
            Self::Integers(_)
            | Self::Strings(_)
            | Self::IntegerPlaces(_)
            | Self::StringPlaces(_) => {}
        }
        Ok(())
    }
}

fn sparse_value<T>(entries: &[(usize, T)], index: usize) -> Option<&T> {
    entries
        .binary_search_by_key(&index, |(entry, _)| *entry)
        .ok()
        .map(|position| &entries[position].1)
}

fn set_sparse_slot<T>(
    length: usize,
    entries: &mut Vec<(usize, T)>,
    index: usize,
    value: T,
) -> Result<(), String>
where
    T: Default + PartialEq,
{
    if index >= length {
        return Err("variable offset is outside its storage".into());
    }
    match entries.binary_search_by_key(&index, |(entry, _)| *entry) {
        Ok(position) if value == T::default() => {
            entries.remove(position);
        }
        Ok(position) => entries[position].1 = value,
        Err(_) if value == T::default() => {}
        Err(position) => entries.insert(position, (index, value)),
    }
    Ok(())
}

fn clear_sparse_range<T>(entries: &mut Vec<(usize, T)>, start: usize, end: usize) {
    entries.retain(|(index, _)| *index < start || *index >= end);
}

fn collect_values<T>(
    values: &[VmValue],
    mut convert: impl FnMut(&VmValue) -> Option<T>,
) -> Result<Vec<T>, String> {
    values
        .iter()
        .map(|value| {
            convert(value)
                .ok_or_else(|| "array replacement differs from its storage shape or type".into())
        })
        .collect()
}

fn collect_sparse_values<T: Default + PartialEq>(
    values: &[VmValue],
    mut convert: impl FnMut(&VmValue) -> Option<T>,
) -> Result<Vec<(usize, T)>, String> {
    let mut entries = Vec::new();
    for (index, value) in values.iter().enumerate() {
        let value = convert(value)
            .ok_or_else(|| "array replacement differs from its storage shape or type".to_owned())?;
        if value != T::default() {
            entries.push((index, value));
        }
    }
    Ok(entries)
}

fn set_slot<T>(values: &mut [T], index: usize, value: T) -> Result<(), String> {
    let slot = values
        .get_mut(index)
        .ok_or_else(|| "variable offset is outside its storage".to_owned())?;
    *slot = value;
    Ok(())
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

struct SparseDefaults<'a, T>(&'a [T]);

impl<T> Serialize for SparseDefaults<'_, T>
where
    T: Default + PartialEq + Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let default = T::default();
        let count = self.0.iter().filter(|value| *value != &default).count();
        let mut entries = serializer.serialize_seq(Some(count))?;
        for (index, value) in self.0.iter().enumerate() {
            if value != &default {
                entries.serialize_element(&(index, value))?;
            }
        }
        entries.end()
    }
}

#[derive(Serialize)]
enum SparseVariableValuesRef<'a> {
    Integers(SparseDefaults<'a, i64>),
    Strings(SparseDefaults<'a, String>),
    IntegerPlaces(SparseDefaults<'a, PlaceDescriptor>),
    StringPlaces(SparseDefaults<'a, PlaceDescriptor>),
}

#[derive(Serialize)]
enum SparseVariableValuesEntriesRef<'a> {
    Integers(&'a [(usize, i64)]),
    Strings(&'a [(usize, String)]),
    IntegerPlaces(&'a [(usize, PlaceDescriptor)]),
    StringPlaces(&'a [(usize, PlaceDescriptor)]),
}

#[derive(Serialize, Deserialize)]
enum SparseVariableValues {
    Integers(Vec<(usize, i64)>),
    Strings(Vec<(usize, String)>),
    IntegerPlaces(Vec<(usize, PlaceDescriptor)>),
    StringPlaces(Vec<(usize, PlaceDescriptor)>),
}

impl Serialize for VariableCell {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match &self.values {
            VariableValues::Integers(values) => (
                &self.value_type,
                &self.dimensions,
                SparseVariableValuesRef::Integers(SparseDefaults(values)),
            )
                .serialize(serializer),
            VariableValues::Strings(values) => (
                &self.value_type,
                &self.dimensions,
                SparseVariableValuesRef::Strings(SparseDefaults(values)),
            )
                .serialize(serializer),
            VariableValues::IntegerPlaces(values) => (
                &self.value_type,
                &self.dimensions,
                SparseVariableValuesRef::IntegerPlaces(SparseDefaults(values)),
            )
                .serialize(serializer),
            VariableValues::StringPlaces(values) => (
                &self.value_type,
                &self.dimensions,
                SparseVariableValuesRef::StringPlaces(SparseDefaults(values)),
            )
                .serialize(serializer),
            VariableValues::SparseIntegers { entries, .. } => (
                &self.value_type,
                &self.dimensions,
                SparseVariableValuesEntriesRef::Integers(entries),
            )
                .serialize(serializer),
            VariableValues::SparseStrings { entries, .. } => (
                &self.value_type,
                &self.dimensions,
                SparseVariableValuesEntriesRef::Strings(entries),
            )
                .serialize(serializer),
            VariableValues::SparseIntegerPlaces { entries, .. } => (
                &self.value_type,
                &self.dimensions,
                SparseVariableValuesEntriesRef::IntegerPlaces(entries),
            )
                .serialize(serializer),
            VariableValues::SparseStringPlaces { entries, .. } => (
                &self.value_type,
                &self.dimensions,
                SparseVariableValuesEntriesRef::StringPlaces(entries),
            )
                .serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for VariableCell {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let (value_type, dimensions, sparse) =
            <(BytecodeType, Vec<u64>, SparseVariableValues)>::deserialize(deserializer)?;
        let length = element_count(&dimensions)
            .ok_or_else(|| D::Error::custom("snapshot variable dimensions overflow"))?;
        let values = match (value_type, sparse) {
            (BytecodeType::Integer, SparseVariableValues::Integers(entries)) => {
                validate_sparse_entries(length, &entries).map_err(D::Error::custom)?;
                VariableValues::SparseIntegers { length, entries }
            }
            (BytecodeType::String, SparseVariableValues::Strings(entries)) => {
                validate_sparse_entries(length, &entries).map_err(D::Error::custom)?;
                VariableValues::SparseStrings { length, entries }
            }
            (BytecodeType::IntegerPlace, SparseVariableValues::IntegerPlaces(entries)) => {
                validate_sparse_entries(length, &entries).map_err(D::Error::custom)?;
                VariableValues::SparseIntegerPlaces { length, entries }
            }
            (BytecodeType::StringPlace, SparseVariableValues::StringPlaces(entries)) => {
                validate_sparse_entries(length, &entries).map_err(D::Error::custom)?;
                VariableValues::SparseStringPlaces { length, entries }
            }
            _ => {
                return Err(D::Error::custom(
                    "snapshot variable values differ from their declared type",
                ));
            }
        };
        Ok(Self {
            value_type,
            dimensions,
            values,
            revision: 0,
        })
    }
}

fn try_default_vector<T: Default>(length: usize) -> Result<Vec<T>, String> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| "snapshot variable allocation failed")?;
    values.resize_with(length, T::default);
    Ok(values)
}

fn apply_sparse_entries<T>(values: &mut [T], entries: Vec<(usize, T)>) -> Result<(), &'static str> {
    validate_sparse_entries(values.len(), &entries)?;
    for (index, value) in entries {
        values[index] = value;
    }
    Ok(())
}

fn validate_sparse_entries<T>(length: usize, entries: &[(usize, T)]) -> Result<(), &'static str> {
    let mut previous = None;
    for (index, _) in entries {
        if *index >= length || previous.is_some_and(|previous| *index <= previous) {
            return Err("snapshot variable entries are not strictly ordered and in bounds");
        }
        previous = Some(*index);
    }
    Ok(())
}

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
