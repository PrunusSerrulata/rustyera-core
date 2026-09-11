use std::collections::{BTreeMap, BTreeSet};

use erabasic_compat::OrdinalCasing;
use erabasic_data::{NameTable, NameTableKind};

use crate::{
    CsvDiagnostic, CsvDiagnosticCode, CsvDiagnosticSeverity, CsvLoadOptions,
    input::{FileIndex, FileRoot, ascii_fold, basename},
    reader::{EnabledLine, enabled_lines},
    tables::{at_line, parse_alias_row},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum PriceSource {
    Unset,
    Csv,
    Erd,
}

/// Loading-only provenance distinguishes an explicit CSV zero from an absent price.
pub(crate) struct ItemPrices {
    pub values: Vec<i64>,
    sources: Vec<PriceSource>,
}

impl ItemPrices {
    pub fn new(length: usize) -> Self {
        Self {
            values: vec![0; length],
            sources: vec![PriceSource::Unset; length],
        }
    }

    pub fn set_csv(&mut self, index: usize, price: i64) {
        self.values[index] = price;
        self.sources[index] = PriceSource::Csv;
    }

    fn merge_erd(
        &mut self,
        index: usize,
        text: &str,
        line: &EnabledLine,
        diagnostics: &mut Vec<CsvDiagnostic>,
    ) {
        let Ok(price) = text.trim().parse::<i64>() else {
            warn(
                diagnostics,
                line,
                CsvDiagnosticCode::InvalidInteger,
                "item price is not an integer",
            );
            return;
        };
        match self.sources[index] {
            PriceSource::Unset => {
                self.values[index] = price;
                self.sources[index] = PriceSource::Erd;
            }
            PriceSource::Csv if self.values[index] != price => warn(
                diagnostics,
                line,
                CsvDiagnosticCode::DuplicateIndex,
                format!("preset ERD price at index {index} conflicts with the CSV price; CSV wins"),
            ),
            PriceSource::Csv | PriceSource::Erd => {}
        }
    }
}

fn preset_kind(stem: &str) -> Option<NameTableKind> {
    Some(match stem {
        "ABL" => NameTableKind::Abl,
        "EXP" => NameTableKind::Exp,
        "TALENT" => NameTableKind::Talent,
        "PALAM" | "UP" | "DOWN" | "JUEL" | "GOTJUEL" | "CUP" | "CDOWN" => NameTableKind::Palam,
        "TRAIN" | "TRAINNAME" => NameTableKind::Train,
        "MARK" => NameTableKind::Mark,
        "ITEM" | "ITEMSALES" | "ITEMPRICE" => NameTableKind::Item,
        "BASE" | "LOSEBASE" | "MAXBASE" | "DOWNBASE" => NameTableKind::Base,
        "SOURCE" => NameTableKind::Source,
        "EX" | "NOWEX" => NameTableKind::Ex,
        "STR" => NameTableKind::Str,
        "EQUIP" => NameTableKind::Equip,
        "TEQUIP" => NameTableKind::Tequip,
        "FLAG" => NameTableKind::Flag,
        "TFLAG" => NameTableKind::Tflag,
        "CFLAG" => NameTableKind::Cflag,
        "TCVAR" => NameTableKind::Tcvar,
        "CSTR" => NameTableKind::Cstr,
        "STAIN" => NameTableKind::Stain,
        "CDFLAG1" | "CDFLAGNAME1" => NameTableKind::Cdflag1,
        "CDFLAG2" | "CDFLAGNAME2" => NameTableKind::Cdflag2,
        "STRNAME" => NameTableKind::Strname,
        "TSTR" => NameTableKind::Tstr,
        "SAVESTR" => NameTableKind::Savestr,
        "GLOBAL" => NameTableKind::Global,
        "GLOBALS" => NameTableKind::Globals,
        "DAY" => NameTableKind::Day,
        "TIME" => NameTableKind::Time,
        "MONEY" => NameTableKind::Money,
        _ => return None,
    })
}

pub(crate) fn merge_preset_erd(
    files: &FileIndex,
    tables: &mut BTreeMap<NameTableKind, NameTable>,
    prices: &mut ItemPrices,
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
) {
    // The upstream scans all ERB descendants regardless of SearchSubdirectories.
    let mut candidates: Vec<_> = files
        .all()
        .filter_map(|file| {
            if file.root != FileRoot::Erb {
                return None;
            }
            let name = ascii_fold(basename(&file.path));
            let kind = preset_kind(name.strip_suffix(".ERD")?)?;
            // Windows reference separators participate in ordinal path ordering.
            // Compute the key once while retaining normalized paths for diagnostics.
            let sort_key = file.path.replace('/', "\\");
            Some((file, kind, sort_key))
        })
        .collect();
    let casing = OrdinalCasing::fixed_dotnet8_icu72();
    candidates.sort_by(|left, right| casing.compare(&left.2, &right.2));
    // Only canonical names participate, including STR; aliases remain lookup-only.
    // Nonempty canonical names never change during this merge, so a set suffices.
    let mut occupied: BTreeSet<String> = tables
        .values()
        .flat_map(|table| table.names.iter().flatten())
        .filter(|name| !name.is_empty())
        .cloned()
        .collect();
    for (file, kind, _) in candidates {
        let table = tables
            .get_mut(&kind)
            .expect("all name tables are allocated before loading");
        for line in enabled_lines(&file.source_path, &file.content, options, diagnostics) {
            let Some((index, name)) = parse_alias_row(&line, diagnostics) else {
                continue;
            };
            if table.names.is_empty() {
                diagnostics.push(at_line(
                    CsvDiagnosticCode::ProhibitedNameTable,
                    CsvDiagnosticSeverity::Error,
                    2,
                    &line,
                    "this name table is disabled",
                ));
                break;
            }
            let Some(index) = usize::try_from(index)
                .ok()
                .filter(|index| *index < table.names.len())
            else {
                warn(
                    diagnostics,
                    &line,
                    CsvDiagnosticCode::IndexOutOfRange,
                    "preset ERD index is outside the declared table length",
                );
                continue;
            };
            let existing = table.names[index].as_deref().unwrap_or_default();
            if existing.is_empty() {
                if !name.is_empty() && !occupied.insert(name.to_owned()) {
                    warn(
                        diagnostics,
                        &line,
                        CsvDiagnosticCode::DuplicateUserIndex,
                        format!(
                            "preset ERD name {name:?} already exists in another name-table slot"
                        ),
                    );
                    continue;
                }
                table.names[index] = Some(name.to_owned());
            } else if existing != name {
                warn(
                    diagnostics,
                    &line,
                    CsvDiagnosticCode::DuplicateIndex,
                    format!(
                        "preset ERD name at index {index} conflicts with the existing name; the existing name wins"
                    ),
                );
            }
            if kind == NameTableKind::Item
                && let Some(price) = line.text.split(',').nth(2)
            {
                prices.merge_erd(index, price, &line, diagnostics);
            }
        }
    }
}

fn warn(
    diagnostics: &mut Vec<CsvDiagnostic>,
    line: &EnabledLine,
    code: CsvDiagnosticCode,
    message: impl Into<String>,
) {
    diagnostics.push(at_line(
        code,
        CsvDiagnosticSeverity::Warning,
        1,
        line,
        message,
    ));
}

#[cfg(test)]
mod tests {
    use super::preset_kind;
    use crate::tables::TABLE_FILES;
    use erabasic_data::NameTableKind;

    #[test]
    fn all_upstream_preset_stems_map_to_existing_name_tables() {
        for (filename, kind) in TABLE_FILES {
            assert_eq!(
                preset_kind(filename.strip_suffix(".CSV").unwrap()),
                Some(kind)
            );
        }
        for (stem, kind) in [
            ("UP", NameTableKind::Palam),
            ("DOWN", NameTableKind::Palam),
            ("JUEL", NameTableKind::Palam),
            ("GOTJUEL", NameTableKind::Palam),
            ("CUP", NameTableKind::Palam),
            ("CDOWN", NameTableKind::Palam),
            ("TRAINNAME", NameTableKind::Train),
            ("ITEMSALES", NameTableKind::Item),
            ("ITEMPRICE", NameTableKind::Item),
            ("LOSEBASE", NameTableKind::Base),
            ("MAXBASE", NameTableKind::Base),
            ("DOWNBASE", NameTableKind::Base),
            ("NOWEX", NameTableKind::Ex),
            ("CDFLAGNAME1", NameTableKind::Cdflag1),
            ("CDFLAGNAME2", NameTableKind::Cdflag2),
        ] {
            assert_eq!(preset_kind(stem), Some(kind));
        }
        assert_eq!(preset_kind("CUSTOM"), None);
    }
}
