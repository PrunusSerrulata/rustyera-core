//! Text-save value conversion with legacy sparse and trailing-value semantics.

use crate::{SaveCodecError, SaveValue};

pub(super) fn scalar_string(value: &SaveValue) -> Result<String, SaveCodecError> {
    match value {
        SaveValue::Integer(value) => Ok(value.to_string()),
        SaveValue::String(value) => Ok(value.clone()),
        _ => Err(SaveCodecError::InvalidFormat(
            "text scalar has an array value".into(),
        )),
    }
}

pub(super) fn trimmed_values(value: &SaveValue) -> Result<Vec<String>, SaveCodecError> {
    let mut values = match value {
        SaveValue::Integers { values, .. } => values.iter().map(ToString::to_string).collect(),
        SaveValue::Strings { values, .. } => values.clone(),
        SaveValue::SparseIntegers { dimensions, values } => {
            let count = sparse_value_count(dimensions)?;
            let mut dense = vec![String::from("0"); count];
            for (index, value) in values {
                let index = usize::try_from(*index)
                    .map_err(|_| SaveCodecError::LimitExceeded("array elements"))?;
                let target = dense.get_mut(index).ok_or_else(|| {
                    SaveCodecError::InvalidFormat("sparse array index exceeds dimensions".into())
                })?;
                *target = value.to_string();
            }
            dense
        }
        SaveValue::SparseStrings { dimensions, values } => {
            let count = sparse_value_count(dimensions)?;
            let mut dense = vec![String::new(); count];
            for (index, value) in values {
                let index = usize::try_from(*index)
                    .map_err(|_| SaveCodecError::LimitExceeded("array elements"))?;
                let target = dense.get_mut(index).ok_or_else(|| {
                    SaveCodecError::InvalidFormat("sparse array index exceeds dimensions".into())
                })?;
                target.clone_from(value);
            }
            dense
        }
        _ => {
            return Err(SaveCodecError::InvalidFormat(
                "text array has a scalar value".into(),
            ));
        }
    };
    while values
        .last()
        .is_some_and(|value| value.is_empty() || value == "0")
    {
        values.pop();
    }
    Ok(values)
}

fn sparse_value_count(dimensions: &[u32]) -> Result<usize, SaveCodecError> {
    dimensions.iter().try_fold(1usize, |count, dimension| {
        count
            .checked_mul(*dimension as usize)
            .ok_or(SaveCodecError::LimitExceeded("array elements"))
    })
}
