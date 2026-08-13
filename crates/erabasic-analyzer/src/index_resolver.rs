use std::collections::BTreeMap;

use erabasic_data::{LegacyEncoding, NameTableKind, ProjectData};

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
            let dimension = dimension_for_kind(*kind);
            let lookup: BTreeMap<_, _> = table
                .lookup
                .iter()
                .map(|(name, index)| (name.clone(), i64::from(*index)))
                .collect();
            for variable in data_variables_for_kind(*kind) {
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

fn data_variables_for_kind(kind: NameTableKind) -> &'static [&'static str] {
    match kind {
        NameTableKind::Abl => &["ABL"],
        NameTableKind::Exp => &["EXP"],
        NameTableKind::Talent => &["TALENT"],
        NameTableKind::Palam => &["PALAM", "UP", "DOWN", "JUEL", "GOTJUEL", "CUP", "CDOWN"],
        NameTableKind::Train => &["TRAIN"],
        NameTableKind::Mark => &["MARK"],
        // ITEM.csv names are shared by every item-indexed built-in variable.
        NameTableKind::Item => &["ITEM", "ITEMSALES", "ITEMPRICE", "ITEMNAME"],
        NameTableKind::Base => &["BASE", "MAXBASE", "LOSEBASE", "DOWNBASE"],
        NameTableKind::Source => &["SOURCE"],
        NameTableKind::Ex => &["EX", "NOWEX"],
        // STR.CSV contains initial string values. Symbolic STR indices come from
        // STRNAME.CSV in the reference implementation.
        NameTableKind::Str => &[],
        NameTableKind::Equip => &["EQUIP"],
        NameTableKind::Tequip => &["TEQUIP"],
        NameTableKind::Flag => &["FLAG"],
        NameTableKind::Tflag => &["TFLAG"],
        NameTableKind::Cflag => &["CFLAG"],
        NameTableKind::Tcvar => &["TCVAR"],
        NameTableKind::Cstr => &["CSTR"],
        NameTableKind::Stain => &["STAIN"],
        NameTableKind::Cdflag1 | NameTableKind::Cdflag2 => &["CDFLAG"],
        NameTableKind::Strname => &["STR", "STRNAME"],
        NameTableKind::Tstr => &["TSTR"],
        NameTableKind::Savestr => &["SAVESTR"],
        NameTableKind::Global => &["GLOBAL"],
        NameTableKind::Globals => &["GLOBALS"],
        NameTableKind::Day => &["DAY"],
        NameTableKind::Time => &["TIME"],
        NameTableKind::Money => &["MONEY"],
    }
}

fn dimension_for_kind(kind: NameTableKind) -> usize {
    usize::from(kind == NameTableKind::Cdflag2)
}
