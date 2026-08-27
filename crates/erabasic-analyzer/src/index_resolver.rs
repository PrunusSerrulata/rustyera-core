use std::collections::BTreeMap;
use std::sync::Arc;

use erabasic_data::{LegacyEncoding, ProjectData};

#[derive(Default)]
pub(crate) struct IndexResolver {
    tables: BTreeMap<(String, usize), Arc<BTreeMap<String, i64>>>,
    // Runtime GETNUM reads only built-in NameTableKind data. Keep deferred symbolic-index
    // tables available to ordinary index analysis without exposing them to GETNUM folding.
    builtin_tables: BTreeMap<(String, usize), Arc<BTreeMap<String, i64>>>,
    rename: BTreeMap<String, i64>,
    legacy_encoding: LegacyEncoding,
}

impl IndexResolver {
    pub fn new(project: &ProjectData) -> Self {
        let mut result = Self::default();
        for (kind, table) in &project.static_data.name_tables {
            let dimension = kind.data_dimension();
            let lookup: Arc<BTreeMap<String, i64>> = Arc::new(
                table
                    .lookup
                    .iter()
                    .map(|(name, index)| (name.clone(), i64::from(*index)))
                    .collect(),
            );
            for variable in kind.data_variables() {
                let key = ((*variable).to_owned(), dimension);
                result.tables.insert(key.clone(), lookup.clone());
                result.builtin_tables.insert(key, lookup.clone());
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
                Arc::new(table.entries.clone()),
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

    pub(crate) fn resolve_builtin(
        &self,
        variable: &str,
        dimension: usize,
        name: &str,
    ) -> Option<i64> {
        self.builtin_tables
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn getnum_resolution_uses_the_builtin_table_instead_of_a_deferred_override() {
        let key = ("CFLAG".to_owned(), 0);
        let mut resolver = IndexResolver::default();
        resolver.tables.insert(
            key.clone(),
            Arc::new([("known".to_owned(), 31)].into_iter().collect()),
        );
        resolver.builtin_tables.insert(
            key,
            Arc::new([("known".to_owned(), 17)].into_iter().collect()),
        );

        assert_eq!(resolver.resolve("CFLAG", 0, "known"), Some(31));
        assert_eq!(resolver.resolve_builtin("CFLAG", 0, "known"), Some(17));
    }
}
