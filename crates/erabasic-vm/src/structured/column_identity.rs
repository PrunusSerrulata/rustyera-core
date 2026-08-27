//! Column identities survive calls and snapshots, but never name-based replacement.

use std::collections::BTreeSet;

use super::{Cell, Column, DataTable, DataType, StructuredState};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ColumnIdentityStamp {
    revision: u64,
    next_identity: u64,
}

impl StructuredState {
    pub(crate) const fn column_identity_stamp(&self) -> ColumnIdentityStamp {
        ColumnIdentityStamp {
            revision: self.column_identity_revision,
            next_identity: self.next_column_identity,
        }
    }

    fn planned_identities(&self, count: usize) -> Result<(std::ops::Range<u64>, u64), String> {
        let count = u64::try_from(count).map_err(|_| "DataTable column count overflowed")?;
        let end = self
            .next_column_identity
            .checked_add(count)
            .ok_or("DataTable column identity overflowed")?;
        let revision = self
            .column_identity_revision
            .checked_add(1)
            .ok_or("DataTable column identity revision overflowed")?;
        if end == u64::MAX || revision == u64::MAX {
            return Err("DataTable column identity allocator exhausted".into());
        }
        Ok((self.next_column_identity..end, revision))
    }

    pub(super) fn install_fresh_table(
        &mut self,
        key: String,
        mut table: DataTable,
    ) -> Result<(), String> {
        validate_table(&table)?;
        let (identities, revision) = self.planned_identities(table.columns.len())?;
        let next_identity = identities.end;
        for (column, identity) in table.columns.iter_mut().zip(identities) {
            column.identity = identity;
        }
        self.data_tables.insert(key, table);
        self.next_column_identity = next_identity;
        self.column_identity_revision = revision;
        Ok(())
    }

    pub(super) fn append_fresh_column(
        &mut self,
        key: &str,
        mut column: Column,
    ) -> Result<(), String> {
        validate_default(&column)?;
        let (identities, revision) = self.planned_identities(1)?;
        let table = self
            .data_tables
            .get_mut(key)
            .ok_or("DataTable disappeared during column creation")?;
        column.identity = identities.start;
        for row in &mut table.rows {
            row.cells.push(column.default_value.clone());
        }
        table.columns.push(column);
        self.next_column_identity = identities.end;
        self.column_identity_revision = revision;
        Ok(())
    }

    pub(super) fn remove_table(&mut self, key: &str) -> Result<(), String> {
        if self.data_tables.contains_key(key) {
            let (_, revision) = self.planned_identities(0)?;
            self.data_tables.remove(key);
            self.column_identity_revision = revision;
        }
        Ok(())
    }

    pub(super) fn remove_column(&mut self, key: &str, index: usize) -> Result<(), String> {
        let (_, revision) = self.planned_identities(0)?;
        let table = self
            .data_tables
            .get_mut(key)
            .ok_or("DataTable disappeared during column removal")?;
        if index == 0 || index >= table.columns.len() {
            return Err("DataTable column removal index is invalid".into());
        }
        table.columns.remove(index);
        for row in &mut table.rows {
            row.cells.remove(index);
        }
        self.column_identity_revision = revision;
        Ok(())
    }

    pub(super) fn find_column_identity(&self, identity: u64) -> Option<&Column> {
        self.data_tables
            .values()
            .flat_map(|table| &table.columns)
            .find(|column| column.identity == identity)
    }

    pub(super) fn find_column_identity_mut(&mut self, identity: u64) -> Option<&mut Column> {
        self.data_tables
            .values_mut()
            .flat_map(|table| &mut table.columns)
            .find(|column| column.identity == identity)
    }

    pub(super) fn validate_identity_state(&self) -> Result<(), String> {
        if self.next_column_identity == 0
            || self.next_column_identity == u64::MAX
            || self.column_identity_revision == u64::MAX
        {
            return Err("DataTable column identity allocator is invalid or exhausted".into());
        }
        let mut identities = BTreeSet::new();
        for table in self.data_tables.values() {
            validate_table(table)?;
            for column in &table.columns {
                if column.identity == 0
                    || column.identity >= self.next_column_identity
                    || !identities.insert(column.identity)
                {
                    return Err(
                        "DataTable column identity is zero, duplicated, or outside the allocator"
                            .into(),
                    );
                }
            }
        }
        if (!identities.is_empty() || self.next_column_identity != 1)
            && self.column_identity_revision == 0
        {
            return Err("DataTable column identities have no revision history".into());
        }
        Ok(())
    }
}

pub(super) fn validate_default(column: &Column) -> Result<(), String> {
    validate_cell_type(column.value_type, &column.default_value)
        .map_err(|error| format!("DataTable column {} default: {error}", column.name))
}

pub(super) fn validate_cell_type(value_type: DataType, cell: &Cell) -> Result<(), String> {
    match (value_type, cell) {
        (_, Cell::Null)
        | (DataType::String, Cell::String(_))
        | (DataType::Int64, Cell::Integer(_)) => Ok(()),
        (DataType::Int8, Cell::Integer(value)) if i8::try_from(*value).is_ok() => Ok(()),
        (DataType::Int16, Cell::Integer(value)) if i16::try_from(*value).is_ok() => Ok(()),
        (DataType::Int32, Cell::Integer(value)) if i32::try_from(*value).is_ok() => Ok(()),
        _ => Err("cell type or integer range differs from its column".into()),
    }
}

pub(super) fn validate_row_cells(table: &DataTable, cells: &[Cell]) -> Result<(), String> {
    if cells.len() != table.columns.len() {
        return Err("DataTable row width differs from its schema".into());
    }
    // The structural id cell is stored separately in DataRow::id.
    for (column, cell) in table.columns.iter().zip(cells).skip(1) {
        validate_cell_type(column.value_type, cell)?;
        if !column.nullable && matches!(cell, Cell::Null) {
            return Err(format!(
                "DataTable row omits non-null column {}",
                column.name
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_table(table: &DataTable) -> Result<(), String> {
    if !table.columns.first().is_some_and(|column| {
        column.name == "id" && column.value_type == DataType::Int64 && !column.nullable
    }) {
        return Err("DataTable schema must start with a non-null Int64 id column".into());
    }
    let mut names = BTreeSet::new();
    for column in &table.columns {
        // Existing NOCASE changes lookup policy without renaming stored columns.
        if !names.insert(&column.name) {
            return Err("DataTable schema repeats a column name".into());
        }
        validate_default(column)?;
    }
    if table.next_id <= table.rows.iter().map(|row| row.id).max().unwrap_or(0) {
        return Err("DataTable next row identity does not follow its rows".into());
    }
    let mut ids = BTreeSet::new();
    for row in &table.rows {
        validate_row_cells(table, &row.cells)?;
        if !matches!(row.cells.first(), Some(Cell::Null)) || !ids.insert(row.id) {
            return Err("DataTable row has an invalid or repeated structural id".into());
        }
    }
    Ok(())
}
