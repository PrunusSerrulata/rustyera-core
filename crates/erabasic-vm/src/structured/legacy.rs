//! Frozen decoder for the pre-XML save extension, not for current native bundles.

use serde::Deserialize;

use super::{Cell, Column, DataRow, DataTable, DataType};

#[derive(Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct LegacyColumn {
    name: String,
    value_type: DataType,
    nullable: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyTable {
    case_sensitive: bool,
    next_id: i64,
    columns: Vec<LegacyColumn>,
    rows: Vec<DataRow>,
}

pub(super) fn decode_table(key: &str, schema: &str, data: &str) -> Result<DataTable, String> {
    let columns: Vec<LegacyColumn> =
        serde_json::from_str(schema).map_err(|error| error.to_string())?;
    let table: LegacyTable = serde_json::from_str(data).map_err(|error| error.to_string())?;
    if table.columns != columns {
        return Err(format!("DataTable extension {key} schema differs"));
    }
    Ok(DataTable {
        case_sensitive: table.case_sensitive,
        next_id: table.next_id,
        columns: table
            .columns
            .into_iter()
            .map(|column| Column {
                identity: 0,
                name: column.name,
                value_type: column.value_type,
                nullable: column.nullable,
                default_value: Cell::Null,
            })
            .collect(),
        rows: table.rows,
    })
}
