use std::collections::BTreeMap;

use erabasic_data::{LegacyEncoding, ProjectData};

#[derive(Default)]
pub(crate) struct IndexResolver {
    tables: BTreeMap<(String, usize), BTreeMap<String, i64>>,
    rename: BTreeMap<String, i64>,
    legacy_encoding: LegacyEncoding,
}

impl IndexResolver {
    pub fn new(project: &ProjectData) -> Self {
        let mut result = Self::default();
        for (kind, table) in &project.static_data.name_tables {
            let dimension = kind.data_dimension();
            let lookup: BTreeMap<_, _> = table
                .lookup
                .iter()
                .map(|(name, index)| (name.clone(), i64::from(*index)))
                .collect();
            for variable in kind.data_variables() {
                result
                    .tables
                    .insert(((*variable).to_owned(), dimension), lookup.clone());
            }
        }
        for (name, table) in &project.static_data.deferred_indices.resolved {
            let (variable, dimension) = name
                .rsplit_once('@')
                .and_then(|(variable, dimension)| {
                    dimension
                        .parse::<usize>()
                        .ok()
                        .map(|dimension| (variable, dimension - 1))
                })
                .unwrap_or((name.as_str(), 0));
            result.tables.insert(
                (variable.to_ascii_uppercase(), dimension),
                table
                    .entries
                    .iter()
                    .map(|(name, index)| {
                        (
                            name.clone(),
                            i64::try_from(*index).expect("deferred index exceeds i64"),
                        )
                    })
                    .collect(),
            );
        }
        result.rename = project
            .static_data
            .rename
            .iter()
            .filter_map(|(name, value)| value.parse().ok().map(|value| (name.clone(), value)))
            .collect();
        result.legacy_encoding = project.static_data.legacy_encoding;
        result
    }

    pub(crate) fn resolve(&self, variable: &str, dimension: usize, name: &str) -> Option<i64> {
        self.tables
            .get(&(variable.to_ascii_uppercase(), dimension))
            .and_then(|table| table.get(name))
            .copied()
    }

    pub(crate) fn resolve_rename(&self, name: &str) -> Option<i64> {
        self.rename.get(name).copied()
    }

    pub(crate) fn has_table(&self, variable: &str, dimension: usize) -> bool {
        self.tables
            .contains_key(&(variable.to_ascii_uppercase(), dimension))
    }

    pub(crate) fn legacy_encoded_len(&self, value: &str) -> usize {
        self.legacy_encoding.encoded_len(value)
    }
}
