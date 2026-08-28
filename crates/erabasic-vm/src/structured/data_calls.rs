#[allow(clippy::wildcard_imports)]
use super::*;
use crate::ExecutionFailure;
use crate::structured::{argument_failure, resource_failure};

impl StructuredState {
    #[allow(clippy::too_many_lines)]
    pub(super) fn call_data_table(
        &mut self,
        name: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, ExecutionFailure> {
        let key = string_argument(request, 0)?.to_owned();
        match name {
            "dt_create" => {
                if self.data_tables.contains_key(&key) {
                    ready_integer(0)
                } else {
                    self.install_fresh_table(key, DataTable::new())?;
                    ready_integer(1)
                }
            }
            "dt_exist" => ready_integer(i64::from(self.data_tables.contains_key(&key))),
            "dt_release" => {
                self.remove_table(&key)?;
                ready_integer(1)
            }
            "dt_clear" => {
                let Some(table) = self.data_tables.get_mut(&key) else {
                    return ready_integer(-1);
                };
                table.rows.clear();
                ready_integer(1)
            }
            "dt_nocase" => {
                let Some(table) = self.data_tables.get_mut(&key) else {
                    return ready_integer(-1);
                };
                table.case_sensitive = integer_argument(request, 1)? == 0;
                ready_integer(1)
            }
            "dt_column_length" | "dt_row_length" => {
                let Some(table) = self.data_tables.get(&key) else {
                    return ready_integer(-1);
                };
                let length = if name == "dt_column_length" {
                    table.columns.len()
                } else {
                    table.rows.len()
                };
                ready_integer(i64::try_from(length).unwrap_or(i64::MAX))
            }
            "dt_column_exist" => {
                let Some(table) = self.data_tables.get(&key) else {
                    return ready_integer(-1);
                };
                let Some(index) = table.column(string_argument(request, 1)?) else {
                    return ready_integer(0);
                };
                ready_integer(data_type_code(table.columns[index].value_type))
            }
            "dt_column_add" => {
                let column_name = string_argument(request, 1)?.to_owned();
                let value_type = request
                    .arguments
                    .get(2)
                    .map(parse_data_type)
                    .transpose()?
                    .unwrap_or(DataType::String);
                let nullable = optional_integer(request, 3) != Some(0);
                let Some(table) = self.data_tables.get_mut(&key) else {
                    return ready_integer(-1);
                };
                if table.column(&column_name).is_some() {
                    return ready_integer(0);
                }
                if !nullable && !table.rows.is_empty() {
                    return Err(argument_failure(
                        "DT_COLUMN_ADD non-null column has no default for existing rows",
                    ));
                }
                self.append_fresh_column(
                    &key,
                    Column {
                        identity: 0,
                        name: column_name,
                        value_type,
                        nullable,
                        default_value: Cell::Null,
                    },
                )?;
                ready_integer(1)
            }
            "dt_column_remove" => {
                let name = string_argument(request, 1)?;
                let Some(table) = self.data_tables.get_mut(&key) else {
                    return ready_integer(-1);
                };
                let Some(index) = table.column(name) else {
                    return ready_integer(0);
                };
                if index == 0 {
                    return ready_integer(0);
                }
                self.remove_column(&key, index)?;
                ready_integer(1)
            }
            "dt_column_names" => {
                let Some(table) = self.data_tables.get(&key) else {
                    return ready_integer(-1);
                };
                let target = request
                    .places
                    .iter()
                    .find(|place| place.argument_index == 1)
                    .unwrap_or(implicit_place(request, "RESULTS")?);
                let writes = array_writes(
                    target,
                    0,
                    table
                        .columns
                        .iter()
                        .map(|column| VmValue::String(column.name.clone())),
                );
                Ok(NativeReady {
                    value: Some(VmValue::Integer(
                        i64::try_from(table.columns.len()).unwrap_or(i64::MAX),
                    )),
                    writes,
                })
            }
            "dt_row_add" => self.data_table_row_add(&key, request),
            "dt_row_set" => self.data_table_row_set(&key, request),
            "dt_row_remove" => self.data_table_row_remove(&key, request),
            "dt_cell_get" | "dt_cell_gets" | "dt_cell_isnull" => {
                let Some(table) = self.data_tables.get(&key) else {
                    return if name == "dt_cell_isnull" {
                        ready_integer(-1)
                    } else if name == "dt_cell_gets" {
                        Ok(NativeReady::value(VmValue::String(String::new())))
                    } else {
                        ready_integer(0)
                    };
                };
                let as_id = optional_integer(request, 3).is_some_and(|value| value != 0);
                let Some(row) = table.row(integer_argument(request, 1)?, as_id) else {
                    return if name == "dt_cell_isnull" {
                        ready_integer(-2)
                    } else if name == "dt_cell_gets" {
                        Ok(NativeReady::value(VmValue::String(String::new())))
                    } else {
                        ready_integer(0)
                    };
                };
                let Some(column) = table.column(string_argument(request, 2)?) else {
                    return if name == "dt_cell_isnull" {
                        ready_integer(-2)
                    } else if name == "dt_cell_gets" {
                        Ok(NativeReady::value(VmValue::String(String::new())))
                    } else {
                        ready_integer(0)
                    };
                };
                let cell = if column == 0 {
                    Cell::Integer(table.rows[row].id)
                } else {
                    table.rows[row].cells[column].clone()
                };
                match name {
                    "dt_cell_get" => ready_integer(match cell {
                        Cell::Integer(value) => value,
                        Cell::Null => 0,
                        Cell::String(_) => {
                            return Err(argument_failure(
                                "DT_CELL_GET cannot read a string column",
                            ));
                        }
                    }),
                    "dt_cell_gets" => Ok(NativeReady::value(VmValue::String(match cell {
                        Cell::String(value) => value,
                        Cell::Null => String::new(),
                        Cell::Integer(value) => value.to_string(),
                    }))),
                    _ => ready_integer(i64::from(matches!(cell, Cell::Null))),
                }
            }
            "dt_cell_set" => self.data_table_cell_set(&key, request),
            "dt_select" => self.data_table_select(&key, request),
            "dt_toxml" => self.data_table_to_xml(&key, request),
            "dt_fromxml" => self.data_table_from_xml(&key, request),
            _ => Err(format!("unsupported data-table native {name}").into()),
        }
    }

    fn data_table_row_add(
        &mut self,
        key: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, ExecutionFailure> {
        let Some(table) = self.data_tables.get(key) else {
            return ready_integer(-1);
        };
        let id = table.next_id;
        let next_id = table
            .next_id
            .checked_add(1)
            .ok_or_else(|| resource_failure("DT_ROW_ADD deterministic row id overflowed"))?;
        let mut row = DataRow {
            id,
            cells: table
                .columns
                .iter()
                .map(|column| column.default_value.clone())
                .collect(),
        };
        let pairs = data_table_pairs(request, 1)?;
        for (name, value) in pairs {
            let column = table.column(&name).ok_or_else(|| {
                argument_failure(format!("DT_ROW_ADD table {key} has no column {name}"))
            })?;
            if column == 0 {
                return Err(argument_failure("DT_ROW_ADD cannot edit the id column"));
            }
            row.cells[column] = cell_for_column(&table.columns[column], &value)?;
        }
        row.cells[0] = Cell::Null;
        validate_script_row_cells(table, &row.cells)?;
        let table = self
            .data_tables
            .get_mut(key)
            .expect("table remains registered while validating a row");
        table.next_id = next_id;
        table.rows.push(row);
        ready_integer(id)
    }

    fn data_table_row_set(
        &mut self,
        key: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, ExecutionFailure> {
        let Some(table) = self.data_tables.get_mut(key) else {
            return ready_integer(-1);
        };
        let id = integer_argument(request, 1)?;
        let Some(row_index) = table.rows.iter().position(|row| row.id == id) else {
            return ready_integer(-2);
        };
        let pairs = data_table_pairs(request, 2)?;
        let mut changes = Vec::with_capacity(pairs.len());
        for (name, value) in pairs {
            let column = table.column(&name).ok_or_else(|| {
                argument_failure(format!("DT_ROW_SET table {key} has no column {name}"))
            })?;
            if column == 0 {
                return Err(argument_failure("DT_ROW_SET cannot edit the id column"));
            }
            changes.push((column, cell_for_column(&table.columns[column], &value)?));
        }
        let count = changes.len();
        let mut cells = table.rows[row_index].cells.clone();
        for (column, value) in changes {
            cells[column] = value;
        }
        validate_script_row_cells(table, &cells)?;
        table.rows[row_index].cells = cells;
        ready_integer(i64::try_from(count).unwrap_or(i64::MAX))
    }

    fn data_table_row_remove(
        &mut self,
        key: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, ExecutionFailure> {
        let Some(table) = self.data_tables.get_mut(key) else {
            return ready_integer(-1);
        };
        if let Some(view) = request
            .places
            .iter()
            .find(|place| place.argument_index == 1)
        {
            let count = usize::try_from(integer_argument(request, 2)?).unwrap_or(0);
            let ids = view
                .values
                .iter()
                .take(count)
                .filter_map(|value| match value {
                    VmValue::Integer(value) => Some(*value),
                    _ => None,
                })
                .collect::<BTreeSet<_>>();
            let previous = table.rows.len();
            table.rows.retain(|row| !ids.contains(&row.id));
            return ready_integer(i64::try_from(previous - table.rows.len()).unwrap_or(i64::MAX));
        }
        let id = integer_argument(request, 1)?;
        let Some(index) = table.rows.iter().position(|row| row.id == id) else {
            return ready_integer(0);
        };
        table.rows.remove(index);
        ready_integer(1)
    }

    fn data_table_cell_set(
        &mut self,
        key: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, ExecutionFailure> {
        let Some(table) = self.data_tables.get_mut(key) else {
            return ready_integer(-1);
        };
        let as_id = optional_integer(request, 4).is_some_and(|value| value != 0);
        let Some(row) = table.row(integer_argument(request, 1)?, as_id) else {
            return ready_integer(-3);
        };
        let Some(column) = table.column(string_argument(request, 2)?) else {
            return ready_integer(-3);
        };
        if column == 0 {
            return ready_integer(0);
        }
        let value = request.arguments.get(3).map_or(Ok(Cell::Null), |value| {
            cell_for_column(&table.columns[column], value)
        });
        let value = match value {
            Ok(value) => value,
            Err(failure) if failure.is_script() => return ready_integer(-2),
            Err(failure) => return Err(failure),
        };
        if !table.columns[column].nullable && matches!(value, Cell::Null) {
            return ready_integer(-2);
        }
        table.rows[row].cells[column] = value;
        ready_integer(1)
    }

    fn data_table_select(
        &self,
        key: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, ExecutionFailure> {
        let Some(table) = self.data_tables.get(key) else {
            return ready_integer(-1);
        };
        let filter = optional_string(request, 1);
        let sort = optional_string(request, 2);
        let mut rows = Vec::new();
        for row in &table.rows {
            let matches = match filter {
                None => true,
                Some(filter) => match row_matches(table, row, filter) {
                    Ok(value) => value,
                    Err(failure) if failure.is_script() => false,
                    Err(failure) => return Err(failure),
                },
            };
            if matches {
                rows.push(row);
            }
        }
        if let Some(sort) = sort.filter(|value| !value.trim().is_empty()) {
            sort_rows(table, &mut rows, sort)?;
        }
        let values = rows.iter().map(|row| VmValue::Integer(row.id));
        let mut writes = if let Some(target) = request
            .places
            .iter()
            .find(|place| place.argument_index == 3)
        {
            array_writes(target, 0, values)
        } else {
            let target = implicit_place(request, "RESULT")?;
            let mut writes = array_writes(target, 1, values);
            writes.extend(array_writes(
                target,
                0,
                [VmValue::Integer(
                    i64::try_from(rows.len()).unwrap_or(i64::MAX),
                )],
            ));
            writes
        };
        writes.shrink_to_fit();
        Ok(NativeReady {
            value: Some(VmValue::Integer(
                i64::try_from(rows.len()).unwrap_or(i64::MAX),
            )),
            writes,
        })
    }

    fn data_table_to_xml(
        &self,
        key: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, ExecutionFailure> {
        let Some(table) = self.data_tables.get(key) else {
            return Ok(NativeReady::value(VmValue::String(String::new())));
        };
        let schema = data_table_schema_xml(key, table);
        let data = data_table_data_xml(key, table);
        let target = request
            .places
            .iter()
            .find(|place| place.argument_index == 1)
            .unwrap_or(implicit_place(request, "RESULTS")?);
        let index = usize::from(request.arguments.len() == 1);
        Ok(NativeReady {
            value: Some(VmValue::String(data)),
            writes: array_writes(target, index, [VmValue::String(schema)]),
        })
    }

    fn data_table_from_xml(
        &mut self,
        key: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, ExecutionFailure> {
        let schema = match parse_data_table_schema(key, string_argument(request, 1)?) {
            Ok(value) => value,
            Err(failure) if failure.is_script() => return ready_integer(0),
            Err(failure) => return Err(failure),
        };
        let mut table = match parse_data_table_xml(key, &schema, string_argument(request, 2)?) {
            Ok(value) => value,
            Err(failure) if failure.is_script() => return ready_integer(0),
            Err(failure) => return Err(failure),
        };
        table.next_id = table
            .rows
            .iter()
            .map(|row| row.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| resource_failure("DT_FROMXML row id overflowed"))?;
        self.install_fresh_table(key.to_owned(), table)?;
        ready_integer(1)
    }
}

// Only a row assembled from script operands may report a missing required value.
// Shared table validation stays a contract check for import and identity state.
fn validate_script_row_cells(table: &DataTable, cells: &[Cell]) -> Result<(), ExecutionFailure> {
    if cells.len() != table.columns.len() {
        return super::column_identity::validate_row_cells(table, cells);
    }
    for (column, cell) in table.columns.iter().zip(cells).skip(1) {
        super::column_identity::validate_cell_type(column.value_type, cell)?;
        if !column.nullable && matches!(cell, Cell::Null) {
            return Err(argument_failure(format!(
                "DataTable row omits non-null column {}",
                column.name
            )));
        }
    }
    Ok(())
}
