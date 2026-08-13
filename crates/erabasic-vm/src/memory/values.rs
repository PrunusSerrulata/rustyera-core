use super::storage_serde::{apply_sparse_entries, try_default_vector};
#[allow(clippy::wildcard_imports)]
use super::*;
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
pub(super) enum VariableValues {
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

    pub(super) fn with_default(value_type: BytecodeType, length: usize) -> Self {
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

    pub(super) fn with_lazy_default(value_type: BytecodeType, length: usize) -> Self {
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

    pub(super) const fn value_type(&self) -> BytecodeType {
        match self {
            Self::Integers(_) | Self::SparseIntegers { .. } => BytecodeType::Integer,
            Self::Strings(_) | Self::SparseStrings { .. } => BytecodeType::String,
            Self::IntegerPlaces(_) | Self::SparseIntegerPlaces { .. } => BytecodeType::IntegerPlace,
            Self::StringPlaces(_) | Self::SparseStringPlaces { .. } => BytecodeType::StringPlace,
        }
    }

    pub(super) fn len(&self) -> usize {
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

    pub(super) fn to_values_range(&self, start: usize, end: usize) -> Option<Vec<VmValue>> {
        (start <= end && end <= self.len())
            .then(|| (start..end).filter_map(|index| self.get(index)).collect())
    }

    #[inline]
    pub(super) fn get(&self, index: usize) -> Option<VmValue> {
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
    pub(super) fn set(&mut self, index: usize, value: VmValue) -> Result<(), String> {
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

    pub(super) fn fill(&mut self, value: VmValue) -> Result<(), String> {
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

    pub(super) fn fill_range(
        &mut self,
        start: usize,
        end: usize,
        value: VmValue,
    ) -> Result<(), String> {
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

    pub(super) fn to_vm_values(&self) -> Vec<VmValue> {
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

    pub(super) fn materialize(&mut self) -> Result<(), String> {
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

pub(super) fn collect_values<T>(
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

pub(super) fn collect_sparse_values<T: Default + PartialEq>(
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
