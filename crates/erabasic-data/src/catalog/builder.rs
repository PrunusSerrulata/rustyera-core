//! Fixed-catalog variable schema insertion.

use std::collections::BTreeMap;

use crate::{Persistence, StorageScope, ValueType, VariableId, VariableSchema};

#[allow(clippy::too_many_arguments)]
pub(super) fn add(
    variables: &mut BTreeMap<String, VariableSchema>,
    name: &str,
    value_type: ValueType,
    storage: StorageScope,
    dimensions: &[usize],
    mutable: bool,
    persistence: Persistence,
    can_forbid: bool,
) {
    variables.insert(
        name.to_owned(),
        VariableSchema {
            id: VariableId::builtin(name),
            value_type,
            storage,
            dimensions: dimensions.to_vec(),
            mutable,
            persistence,
            can_forbid,
        },
    );
}
