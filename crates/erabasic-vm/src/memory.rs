use std::collections::{BTreeMap, HashSet};

use erabasic_bytecode::{
    BytecodeArtifact, BytecodeConstant, BytecodeGlobal, BytecodeStorage, BytecodeType, SymbolKey,
};
use erabasic_data::{CharacterSelection, CharacterTemplate, RuntimeDefaults};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer, de::Error as _, ser::SerializeSeq as _,
};

use crate::{GenerationId, PlaceDescriptor, VmValue};

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
    const SPARSE_DEFAULT_MINIMUM_LENGTH: usize = 64 * 1024;

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

fn set_slot<T>(values: &mut [T], index: usize, value: T) -> Result<(), String> {
    let slot = values
        .get_mut(index)
        .ok_or_else(|| "variable offset is outside its storage".to_owned())?;
    *slot = value;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VariableCell {
    pub value_type: BytecodeType,
    pub dimensions: Vec<u64>,
    values: VariableValues,
}

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
        let mut values = if matches!(
            definition.storage,
            BytecodeStorage::FunctionLocal
                | BytecodeStorage::FunctionStatic
                | BytecodeStorage::FunctionPersistent
        ) {
            VariableValues::with_lazy_default(definition.value_type, length)
        } else {
            VariableValues::with_default(definition.value_type, length)
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
        self.values.set(offset, value)
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
        self.values.set(index, value)
    }

    pub(crate) fn fill(&mut self, value: VmValue) -> Result<(), String> {
        self.values.fill(value)
    }

    pub(crate) fn fill_range(
        &mut self,
        start: usize,
        end: usize,
        value: VmValue,
    ) -> Result<(), String> {
        self.values.fill_range(start, end, value)
    }

    pub(crate) fn to_values(&self) -> Vec<VmValue> {
        self.values.to_vm_values()
    }

    pub(crate) fn integers(&self) -> Option<&[i64]> {
        match &self.values {
            VariableValues::Integers(values) => Some(values),
            _ => None,
        }
    }

    pub(crate) fn integers_mut(&mut self) -> Option<&mut [i64]> {
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

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct LegacyMemory {
    pub shared: BTreeMap<SymbolKey, VariableCell>,
    pub statics: BTreeMap<SymbolKey, VariableCell>,
    pub characters: Vec<BTreeMap<SymbolKey, VariableCell>>,
}

#[derive(Clone, Debug, Default)]
struct StaticInitializationCache(HashSet<(GenerationId, SymbolKey)>);

// This cache only avoids repeated idempotent checks. It is not VM state and
// must not affect snapshot equality or the deterministic serialized payload.
impl PartialEq for StaticInitializationCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for StaticInitializationCache {}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Memory {
    pub shared: BTreeMap<SymbolKey, VariableCell>,
    pub statics: BTreeMap<SymbolKey, VariableCell>,
    pub characters: Vec<BTreeMap<SymbolKey, VariableCell>>,
    pub legacy: BTreeMap<GenerationId, LegacyMemory>,
    #[serde(skip)]
    initialized_static_functions: StaticInitializationCache,
}

impl Memory {
    pub(crate) fn materialize_snapshot(&mut self) -> Result<(), String> {
        for cell in self
            .shared
            .values_mut()
            .chain(self.statics.values_mut())
            .chain(
                self.characters
                    .iter_mut()
                    .flat_map(|character| character.values_mut()),
            )
        {
            cell.materialize_snapshot()?;
        }
        Ok(())
    }

    pub fn title(artifact: &BytecodeArtifact) -> Self {
        let mut result = Self::empty(artifact);
        // Emuera initializes ordinary variable defaults before SYSTEM_TITLE, but
        // ResetData and the initial CSV characters are deferred until the player
        // actually selects "new game" from the built-in title flow.
        result.apply_runtime_defaults(artifact, &artifact.project_data.new_game_seed().defaults);
        result
    }

    pub fn new_game(artifact: &BytecodeArtifact) -> Self {
        let mut result = Self::title(artifact);
        for selection in &artifact.project_data.new_game_seed().initial_characters {
            match selection {
                CharacterSelection::CsvNumber(number) => {
                    let template = artifact
                        .project_data
                        .static_data
                        .characters
                        .iter()
                        .find(|template| template.csv_no == *number);
                    result.push_character(artifact, template);
                }
            }
        }
        // Calculated variables are materialized as cells so bytecode can load
        // them normally. Initialization must therefore refresh CHARANUM just as
        // the native character mutation service does after ADDCHARA.
        result.refresh_character_count(artifact);
        result
    }

    pub(crate) fn refresh_character_count(&mut self, artifact: &BytecodeArtifact) {
        self.set_named_integer(
            artifact,
            "CHARANUM",
            i64::try_from(self.characters.len()).unwrap_or(i64::MAX),
        );
    }

    pub fn empty(artifact: &BytecodeArtifact) -> Self {
        let mut result = Self::default();
        for definition in &artifact.globals {
            match definition.storage {
                BytecodeStorage::Project
                | BytecodeStorage::Constant
                | BytecodeStorage::Calculated => {
                    result
                        .shared
                        .insert(definition.key, VariableCell::new(definition));
                }
                BytecodeStorage::FunctionStatic
                | BytecodeStorage::FunctionPersistent
                | BytecodeStorage::FunctionLocal
                | BytecodeStorage::Character => {}
            }
        }
        result
    }

    pub(crate) fn ensure_function_statics<'a>(
        &mut self,
        generation: GenerationId,
        function: SymbolKey,
        definitions: impl IntoIterator<Item = &'a BytecodeGlobal>,
    ) {
        if !self
            .initialized_static_functions
            .0
            .insert((generation, function))
        {
            return;
        }
        for definition in definitions {
            if self
                .legacy
                .get(&generation)
                .is_some_and(|memory| memory.statics.contains_key(&definition.key))
            {
                continue;
            }
            self.statics
                .entry(definition.key)
                .or_insert_with(|| VariableCell::new(definition));
        }
    }

    pub fn push_character(
        &mut self,
        artifact: &BytecodeArtifact,
        template: Option<&CharacterTemplate>,
    ) {
        let mut character: BTreeMap<_, _> = artifact
            .globals
            .iter()
            .filter(|definition| definition.storage == BytecodeStorage::Character)
            .map(|definition| (definition.key, VariableCell::new(definition)))
            .collect();
        if let Some(template) = template {
            initialize_character(artifact, &mut character, template);
        }
        self.characters.push(character);
    }

    fn apply_runtime_defaults(&mut self, artifact: &BytecodeArtifact, defaults: &RuntimeDefaults) {
        self.set_named_values(artifact, "ITEMPRICE", &defaults.item_prices);
        self.set_named_optional_strings(artifact, "STR", &defaults.str_values);
        self.set_named_values(artifact, "PALAMLV", &defaults.palam_levels);
        self.set_named_values(artifact, "EXPLV", &defaults.exp_levels);
        let static_data = &artifact.project_data.static_data;
        let game_base = &static_data.game_base;
        for (name, value) in [
            ("ASSI", defaults.assi_0),
            ("TARGET", defaults.target_0),
            ("PBAND", defaults.pband_0),
            ("EJAC", defaults.ejac_0),
            ("NOITEM", defaults.no_item_0),
            ("RELATION", defaults.relation_default),
            ("LASTLOAD_VERSION", defaults.last_load_version),
            ("LASTLOAD_NO", defaults.last_load_no),
            ("GAMEBASE_GAMECODE", game_base.unique_code),
            ("GAMEBASE_VERSION", game_base.version),
            ("GAMEBASE_ALLOWVERSION", game_base.compatible_min_version),
            ("GAMEBASE_DEFAULTCHARA", game_base.default_character),
            ("GAMEBASE_NOITEM", game_base.no_item),
            ("__INT_MAX__", i64::MAX),
            ("__INT_MIN__", i64::MIN),
        ] {
            self.set_named_integer(artifact, name, value);
        }
        self.set_named_string(artifact, "LASTLOAD_TEXT", &defaults.last_load_text);
        for (name, value) in [
            ("GAMEBASE_AUTHER", game_base.author.as_str()),
            ("GAMEBASE_AUTHOR", game_base.author.as_str()),
            ("GAMEBASE_INFO", game_base.info.as_str()),
            ("GAMEBASE_YEAR", game_base.year.as_str()),
            ("GAMEBASE_TITLE", game_base.title.as_str()),
            ("GAMEBASE_URL", game_base.update_url.as_str()),
            ("GAMEBASE_VERSIONNAME", game_base.version_name.as_str()),
            (
                "WINDOW_TITLE",
                game_base.window_title.as_deref().unwrap_or_default(),
            ),
            ("MONEYLABEL", static_data.replace.money_label.as_str()),
            ("DRAWLINESTR", static_data.replace.draw_line_string.as_str()),
            ("EMUERA_VERSION", "1.824.0.0"),
        ] {
            self.set_named_string(artifact, name, value);
        }
    }

    pub(crate) fn set_last_load(
        &mut self,
        artifact: &BytecodeArtifact,
        version: i64,
        slot: i64,
        text: &str,
    ) {
        self.set_named_integer(artifact, "LASTLOAD_VERSION", version);
        self.set_named_integer(artifact, "LASTLOAD_NO", slot);
        self.set_named_string(artifact, "LASTLOAD_TEXT", text);
    }

    fn set_named_integer(&mut self, artifact: &BytecodeArtifact, name: &str, value: i64) {
        if let Some(definition) = find_definition(artifact, name)
            && let Some(cell) = self.shared.get_mut(&definition.key)
        {
            let _ = cell.set(0, VmValue::Integer(value));
        }
    }

    fn set_named_string(&mut self, artifact: &BytecodeArtifact, name: &str, value: &str) {
        if let Some(definition) = find_definition(artifact, name)
            && let Some(cell) = self.shared.get_mut(&definition.key)
        {
            let _ = cell.set(0, VmValue::String(value.into()));
        }
    }

    fn set_named_values(&mut self, artifact: &BytecodeArtifact, name: &str, values: &[i64]) {
        if let Some(definition) = find_definition(artifact, name)
            && let Some(cell) = self.shared.get_mut(&definition.key)
        {
            for (index, value) in values.iter().copied().take(cell.len()).enumerate() {
                let _ = cell.set(index, VmValue::Integer(value));
            }
        }
    }

    fn set_named_optional_strings(
        &mut self,
        artifact: &BytecodeArtifact,
        name: &str,
        values: &[Option<String>],
    ) {
        if let Some(definition) = find_definition(artifact, name)
            && let Some(cell) = self.shared.get_mut(&definition.key)
        {
            for (index, value) in values.iter().take(cell.len()).enumerate() {
                let _ = cell.set(index, VmValue::String(value.clone().unwrap_or_default()));
            }
        }
    }

    pub fn target_character(&self, artifact: &BytecodeArtifact, generation: GenerationId) -> usize {
        self.target_character_from_definition(find_definition(artifact, "TARGET"), generation)
    }

    #[inline]
    pub(crate) fn target_character_from_definition(
        &self,
        definition: Option<&BytecodeGlobal>,
        generation: GenerationId,
    ) -> usize {
        let Some(definition) = definition else {
            return 0;
        };
        let cell = if self.legacy.is_empty() {
            self.shared.get(&definition.key)
        } else {
            self.legacy
                .get(&generation)
                .and_then(|memory| memory.shared.get(&definition.key))
                .or_else(|| self.shared.get(&definition.key))
        };
        match cell.and_then(VariableCell::first) {
            Some(VmValue::Integer(value)) => usize::try_from(value).unwrap_or(0),
            _ => 0,
        }
    }

    #[inline]
    pub fn cell(
        &self,
        generation: GenerationId,
        definition: &BytecodeGlobal,
        character: usize,
    ) -> Option<&VariableCell> {
        if self.legacy.is_empty() {
            return match definition.storage {
                BytecodeStorage::Project
                | BytecodeStorage::Constant
                | BytecodeStorage::Calculated => self.shared.get(&definition.key),
                BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                    self.statics.get(&definition.key)
                }
                BytecodeStorage::Character => self
                    .characters
                    .get(character)
                    .and_then(|values| values.get(&definition.key)),
                BytecodeStorage::FunctionLocal => None,
            };
        }
        let legacy = self.legacy.get(&generation);
        match definition.storage {
            BytecodeStorage::Project | BytecodeStorage::Constant | BytecodeStorage::Calculated => {
                legacy
                    .and_then(|memory| memory.shared.get(&definition.key))
                    .or_else(|| self.shared.get(&definition.key))
            }
            BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => legacy
                .and_then(|memory| memory.statics.get(&definition.key))
                .or_else(|| self.statics.get(&definition.key)),
            BytecodeStorage::Character => legacy
                .and_then(|memory| memory.characters.get(character))
                .and_then(|values| values.get(&definition.key))
                .or_else(|| {
                    self.characters
                        .get(character)
                        .and_then(|values| values.get(&definition.key))
                }),
            BytecodeStorage::FunctionLocal => None,
        }
    }

    #[inline]
    pub fn cell_mut(
        &mut self,
        generation: GenerationId,
        definition: &BytecodeGlobal,
        character: usize,
    ) -> Option<&mut VariableCell> {
        if self.legacy.is_empty() {
            return match definition.storage {
                BytecodeStorage::Project
                | BytecodeStorage::Constant
                | BytecodeStorage::Calculated => self.shared.get_mut(&definition.key),
                BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                    self.statics.get_mut(&definition.key)
                }
                BytecodeStorage::Character => self
                    .characters
                    .get_mut(character)
                    .and_then(|values| values.get_mut(&definition.key)),
                BytecodeStorage::FunctionLocal => None,
            };
        }
        let use_legacy =
            self.legacy
                .get(&generation)
                .is_some_and(|memory| match definition.storage {
                    BytecodeStorage::Project
                    | BytecodeStorage::Constant
                    | BytecodeStorage::Calculated => memory.shared.contains_key(&definition.key),
                    BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                        memory.statics.contains_key(&definition.key)
                    }
                    BytecodeStorage::Character => memory
                        .characters
                        .get(character)
                        .is_some_and(|values| values.contains_key(&definition.key)),
                    BytecodeStorage::FunctionLocal => false,
                });
        if use_legacy {
            let memory = self.legacy.get_mut(&generation)?;
            return match definition.storage {
                BytecodeStorage::Project
                | BytecodeStorage::Constant
                | BytecodeStorage::Calculated => memory.shared.get_mut(&definition.key),
                BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                    memory.statics.get_mut(&definition.key)
                }
                BytecodeStorage::Character => memory
                    .characters
                    .get_mut(character)
                    .and_then(|values| values.get_mut(&definition.key)),
                BytecodeStorage::FunctionLocal => None,
            };
        }
        match definition.storage {
            BytecodeStorage::Project | BytecodeStorage::Constant | BytecodeStorage::Calculated => {
                self.shared.get_mut(&definition.key)
            }
            BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                self.statics.get_mut(&definition.key)
            }
            BytecodeStorage::Character => self
                .characters
                .get_mut(character)
                .and_then(|values| values.get_mut(&definition.key)),
            BytecodeStorage::FunctionLocal => None,
        }
    }

    pub fn migrate(
        &mut self,
        old_generation: GenerationId,
        old: &BytecodeArtifact,
        target: &BytecodeArtifact,
    ) {
        let target_definitions: BTreeMap<_, _> = target
            .globals
            .iter()
            .map(|definition| (definition.key, definition))
            .collect();
        let mut legacy = LegacyMemory {
            characters: vec![BTreeMap::new(); self.characters.len()],
            ..LegacyMemory::default()
        };
        for definition in &old.globals {
            if definition.storage == BytecodeStorage::FunctionLocal {
                continue;
            }
            let changed = target_definitions
                .get(&definition.key)
                .is_none_or(|target| target.dimensions != definition.dimensions);
            if !changed {
                continue;
            }
            match definition.storage {
                BytecodeStorage::Project
                | BytecodeStorage::Constant
                | BytecodeStorage::Calculated => {
                    if let Some(cell) = self.shared.get(&definition.key) {
                        legacy.shared.insert(definition.key, cell.clone());
                    }
                }
                BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                    legacy.statics.insert(
                        definition.key,
                        self.statics
                            .get(&definition.key)
                            .map_or_else(|| VariableCell::new(definition), Clone::clone),
                    );
                }
                BytecodeStorage::Character => {
                    for (index, character) in self.characters.iter().enumerate() {
                        if let Some(cell) = character.get(&definition.key) {
                            legacy.characters[index].insert(definition.key, cell.clone());
                        }
                    }
                }
                BytecodeStorage::FunctionLocal => {}
            }
        }
        for definition in &target.globals {
            let old_definition = old.globals.iter().find(|old| old.key == definition.key);
            let changed = old_definition.is_none_or(|old| old.dimensions != definition.dimensions);
            if !changed || definition.storage == BytecodeStorage::FunctionLocal {
                continue;
            }
            match definition.storage {
                BytecodeStorage::Project
                | BytecodeStorage::Constant
                | BytecodeStorage::Calculated => {
                    let cell = self.shared.get(&definition.key).map_or_else(
                        || VariableCell::new(definition),
                        |cell| cell.migrate(definition),
                    );
                    self.shared.insert(definition.key, cell);
                }
                BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                    if let Some(cell) = self.statics.get(&definition.key) {
                        self.statics
                            .insert(definition.key, cell.migrate(definition));
                    }
                }
                BytecodeStorage::Character => {
                    for character in &mut self.characters {
                        let cell = character.get(&definition.key).map_or_else(
                            || VariableCell::new(definition),
                            |cell| cell.migrate(definition),
                        );
                        character.insert(definition.key, cell);
                    }
                }
                BytecodeStorage::FunctionLocal => {}
            }
        }
        if !legacy.shared.is_empty()
            || !legacy.statics.is_empty()
            || legacy.characters.iter().any(|values| !values.is_empty())
        {
            self.legacy.insert(old_generation, legacy);
        }
    }

    pub fn reclaim_generation(&mut self, generation: GenerationId) {
        self.legacy.remove(&generation);
        self.initialized_static_functions
            .0
            .retain(|(cached, _)| *cached != generation);
    }
}

fn find_definition<'a>(artifact: &'a BytecodeArtifact, name: &str) -> Option<&'a BytecodeGlobal> {
    artifact
        .globals
        .iter()
        .find(|definition| definition.name.eq_ignore_ascii_case(name))
}

fn initialize_character(
    artifact: &BytecodeArtifact,
    cells: &mut BTreeMap<SymbolKey, VariableCell>,
    template: &CharacterTemplate,
) {
    for (name, value) in [
        ("NO", VmValue::Integer(template.no)),
        ("NAME", VmValue::String(template.name.clone())),
        ("CALLNAME", VmValue::String(template.call_name.clone())),
        ("NICKNAME", VmValue::String(template.nick_name.clone())),
        ("MASTERNAME", VmValue::String(template.master_name.clone())),
    ] {
        if let Some(definition) = find_definition(artifact, name)
            && let Some(cell) = cells.get_mut(&definition.key)
        {
            let _ = cell.set(0, value);
        }
    }
    for (name, values) in [
        ("MAXBASE", &template.max_base),
        ("BASE", &template.max_base),
        ("MARK", &template.mark),
        ("EXP", &template.exp),
        ("ABL", &template.abl),
        ("TALENT", &template.talent),
        ("RELATION", &template.relation),
        ("CFLAG", &template.cflag),
        ("EQUIP", &template.equip),
        ("JUEL", &template.juel),
    ] {
        if let Some(definition) = find_definition(artifact, name)
            && let Some(cell) = cells.get_mut(&definition.key)
        {
            for (index, value) in values {
                if *index < cell.len() {
                    let _ = cell.set(*index, VmValue::Integer(*value));
                }
            }
        }
    }
    if let Some(definition) = find_definition(artifact, "CSTR")
        && let Some(cell) = cells.get_mut(&definition.key)
    {
        for (index, value) in &template.cstr {
            if *index < cell.len() {
                let _ = cell.set(*index, VmValue::String(value.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
    fn large_function_cells_keep_default_storage_sparse_during_point_updates() {
        let mut integer_definition = global(BytecodeType::Integer, vec![1_000_000]);
        integer_definition.storage = BytecodeStorage::FunctionPersistent;
        integer_definition.owner = Some(SymbolKey::derive("memory.test", b"function"));
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

        let mut string_definition = global(BytecodeType::String, vec![1_000_000]);
        string_definition.storage = BytecodeStorage::FunctionPersistent;
        string_definition.owner = Some(SymbolKey::derive("memory.test", b"function"));
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
    fn snapshot_cells_use_sparse_round_trippable_storage() {
        let mut integer = VariableCell::new(&global(BytecodeType::Integer, vec![1_000_000]));
        integer.set(999_999, VmValue::Integer(42)).unwrap();
        let encoded = rmp_serde::to_vec(&integer).unwrap();
        assert!(encoded.len() < 128);
        let mut decoded = rmp_serde::from_slice::<VariableCell>(&encoded).unwrap();
        decoded.materialize_snapshot().unwrap();
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
}
