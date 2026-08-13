#[allow(clippy::wildcard_imports)]
use super::*;
use serde::{de::Error as _, ser::SerializeSeq as _};
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
pub(super) enum SparseVariableValues {
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

pub(super) fn try_default_vector<T: Default>(length: usize) -> Result<Vec<T>, String> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| "snapshot variable allocation failed")?;
    values.resize_with(length, T::default);
    Ok(values)
}

pub(super) fn apply_sparse_entries<T>(
    values: &mut [T],
    entries: Vec<(usize, T)>,
) -> Result<(), &'static str> {
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
