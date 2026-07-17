//! Reference-shaped `DataTable` schema and XML conversion helpers.

use super::{
    Cell, Column, DataRow, DataTable, DataType, HostWrite, NativeCallRequest, NativePlaceView,
    NativeReady, PlaceDescriptor, VmValue, XmlChild, XmlElement, parse_xml, xml_attribute_escape,
    xml_text_escape,
};

pub(super) fn data_table_schema_xml(key: &str, table: &DataTable) -> String {
    let table_name = encode_xml_name(key);
    let mut output = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-16\"?>\r\n\
<xs:schema id=\"NewDataSet\" xmlns=\"\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:msdata=\"urn:schemas-microsoft-com:xml-msdata\">\r\n",
    );
    output.push_str(
        "  <xs:element name=\"NewDataSet\" msdata:IsDataSet=\"true\" msdata:MainDataTable=\"",
    );
    output.push_str(&xml_attribute_escape(&table_name));
    output.push_str("\" msdata:CaseSensitive=\"");
    output.push_str(if table.case_sensitive {
        "true"
    } else {
        "false"
    });
    output.push_str("\" msdata:UseCurrentLocale=\"true\">\r\n");
    output.push_str("    <xs:complexType>\r\n      <xs:choice minOccurs=\"0\" maxOccurs=\"unbounded\">\r\n        <xs:element name=\"");
    output.push_str(&xml_attribute_escape(&table_name));
    output.push_str("\" msdata:CaseSensitive=\"");
    output.push_str(if table.case_sensitive {
        "True"
    } else {
        "False"
    });
    output.push_str("\">\r\n          <xs:complexType>\r\n            <xs:sequence>\r\n");
    for column in &table.columns {
        output.push_str("              <xs:element name=\"");
        output.push_str(&xml_attribute_escape(&encode_xml_name(&column.name)));
        output.push_str("\" type=\"");
        output.push_str(match column.value_type {
            DataType::Int8 => "xs:byte",
            DataType::Int16 => "xs:short",
            DataType::Int32 => "xs:int",
            DataType::Int64 => "xs:long",
            DataType::String => "xs:string",
        });
        if column.nullable {
            output.push_str("\" minOccurs=\"0");
        }
        output.push_str("\" />\r\n");
    }
    output.push_str(
        "            </xs:sequence>\r\n          </xs:complexType>\r\n        </xs:element>\r\n      </xs:choice>\r\n    </xs:complexType>\r\n    <xs:unique name=\"Constraint1\" msdata:PrimaryKey=\"true\">\r\n      <xs:selector xpath=\".//",
    );
    output.push_str(&xml_attribute_escape(&table_name));
    output.push_str("\" />\r\n      <xs:field xpath=\"");
    output.push_str(&xml_attribute_escape(&encode_xml_name("id")));
    output.push_str("\" />\r\n    </xs:unique>\r\n  </xs:element>\r\n</xs:schema>");
    output
}

pub(super) fn data_table_data_xml(key: &str, table: &DataTable) -> String {
    if table.rows.is_empty() {
        return "<DocumentElement />".into();
    }
    let table_name = encode_xml_name(key);
    let mut output = String::from("<DocumentElement>\r\n");
    for row in &table.rows {
        output.push_str("  <");
        output.push_str(&table_name);
        output.push_str(">\r\n");
        for (index, column) in table.columns.iter().enumerate() {
            let cell = if index == 0 {
                Cell::Integer(row.id)
            } else {
                row.cells.get(index).cloned().unwrap_or(Cell::Null)
            };
            if matches!(cell, Cell::Null) {
                continue;
            }
            let name = encode_xml_name(&column.name);
            output.push_str("    <");
            output.push_str(&name);
            output.push('>');
            match cell {
                Cell::Integer(value) => output.push_str(&value.to_string()),
                Cell::String(value) => output.push_str(&xml_text_escape(&value)),
                Cell::Null => unreachable!(),
            }
            output.push_str("</");
            output.push_str(&name);
            output.push_str(">\r\n");
        }
        output.push_str("  </");
        output.push_str(&table_name);
        output.push_str(">\r\n");
    }
    output.push_str("</DocumentElement>");
    output
}

pub(super) fn parse_data_table_schema(key: &str, xml: &str) -> Result<DataTable, String> {
    let document = parse_xml(xml)?;
    if document.root.name != "xs:schema" {
        return Err("DataTable schema root must be xs:schema".into());
    }
    let expected_table = encode_xml_name(key);
    let mut table_elements = Vec::new();
    collect_elements(&document.root, "xs:element", &mut table_elements);
    let table_element = table_elements
        .into_iter()
        .find(|element| {
            element.attribute("name") == Some(expected_table.as_str())
                && element.attribute("msdata:CaseSensitive").is_some()
        })
        .ok_or_else(|| "DataTable schema does not describe the requested table".to_owned())?;
    let case_sensitive = table_element.attribute("msdata:CaseSensitive") != Some("False");
    let mut sequences = Vec::new();
    collect_elements(table_element, "xs:sequence", &mut sequences);
    let sequence = sequences
        .first()
        .ok_or("DataTable schema has no column sequence")?;
    let mut columns = Vec::new();
    for child in &sequence.children {
        let XmlChild::Element(element) = child else {
            continue;
        };
        if element.name != "xs:element" {
            continue;
        }
        let name = decode_xml_name(
            element
                .attribute("name")
                .ok_or("DataTable column has no name")?,
        )?;
        let value_type = match element.attribute("type") {
            Some("xs:byte") => DataType::Int8,
            Some("xs:short") => DataType::Int16,
            Some("xs:int") => DataType::Int32,
            Some("xs:long") => DataType::Int64,
            Some("xs:string") => DataType::String,
            _ => return Err("DataTable schema contains an unsupported column type".into()),
        };
        columns.push(Column {
            name,
            value_type,
            nullable: element.attribute("minOccurs") == Some("0"),
        });
    }
    if columns.first()
        != Some(&Column {
            name: "id".into(),
            value_type: DataType::Int64,
            nullable: false,
        })
    {
        return Err("DataTable schema must start with a non-null Int64 id column".into());
    }
    Ok(DataTable {
        case_sensitive,
        next_id: 1,
        columns,
        rows: Vec::new(),
    })
}

pub(super) fn parse_data_table_xml(
    key: &str,
    schema: &DataTable,
    xml: &str,
) -> Result<DataTable, String> {
    let document = parse_xml(xml)?;
    if document.root.name != "DocumentElement" {
        return Err("DataTable data root must be DocumentElement".into());
    }
    let table_name = encode_xml_name(key);
    let mut table = schema.clone();
    for child in &document.root.children {
        let XmlChild::Element(row_element) = child else {
            continue;
        };
        if row_element.name != table_name {
            return Err("DataTable data contains a row for another table".into());
        }
        let mut cells = table.columns.iter().map(|_| Cell::Null).collect::<Vec<_>>();
        for cell_element in &row_element.children {
            let XmlChild::Element(cell_element) = cell_element else {
                continue;
            };
            let name = decode_xml_name(&cell_element.name)?;
            let index = table
                .column(&name)
                .ok_or_else(|| format!("DataTable data contains unknown column {name}"))?;
            if !matches!(cells[index], Cell::Null) {
                return Err(format!("DataTable row repeats column {name}"));
            }
            let text = cell_element.inner_text();
            cells[index] = match table.columns[index].value_type {
                DataType::String => Cell::String(text),
                value_type => {
                    let value = text
                        .parse::<i64>()
                        .map_err(|_| format!("DataTable column {name} is not an integer"))?;
                    cell_for_column(
                        &Column {
                            name: name.clone(),
                            value_type,
                            nullable: table.columns[index].nullable,
                        },
                        &VmValue::Integer(value),
                    )?
                }
            };
        }
        let id = match cells.first() {
            Some(Cell::Integer(value)) => *value,
            _ => return Err("DataTable row has no integer id".into()),
        };
        for (column, cell) in table.columns.iter().zip(&cells) {
            if !column.nullable && matches!(cell, Cell::Null) {
                return Err(format!(
                    "DataTable row omits non-null column {}",
                    column.name
                ));
            }
        }
        if table.rows.iter().any(|row| row.id == id) {
            return Err("DataTable data repeats a primary key".into());
        }
        cells[0] = Cell::Null;
        table.rows.push(DataRow { id, cells });
    }
    Ok(table)
}

pub(super) fn collect_elements<'a>(
    element: &'a XmlElement,
    name: &str,
    output: &mut Vec<&'a XmlElement>,
) {
    if element.name == name {
        output.push(element);
    }
    for child in &element.children {
        if let XmlChild::Element(child) = child {
            collect_elements(child, name, output);
        }
    }
}

pub(super) fn encode_xml_name(value: &str) -> String {
    let mut output = String::new();
    for (index, character) in value.chars().enumerate() {
        let valid = character == '_'
            || character.is_alphabetic()
            || index > 0 && (character.is_alphanumeric() || matches!(character, '-' | '.'));
        if valid {
            output.push(character);
        } else {
            use std::fmt::Write as _;
            let _ = write!(output, "_x{:04X}_", u32::from(character));
        }
    }
    output
}

pub(super) fn decode_xml_name(value: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut rest = value;
    while let Some(position) = rest.find("_x") {
        output.push_str(&rest[..position]);
        let escape = rest
            .get(position + 2..position + 6)
            .filter(|_| rest.get(position + 6..position + 7) == Some("_"));
        let Some(escape) = escape else {
            output.push_str("_x");
            rest = &rest[position + 2..];
            continue;
        };
        let scalar = u32::from_str_radix(escape, 16)
            .ok()
            .and_then(char::from_u32)
            .ok_or("DataTable XML name contains an invalid escape")?;
        output.push(scalar);
        rest = &rest[position + 7..];
    }
    output.push_str(rest);
    Ok(output)
}

pub(super) fn string_argument(request: &NativeCallRequest, index: usize) -> Result<&str, String> {
    match request.arguments.get(index) {
        Some(VmValue::String(value)) => Ok(value),
        _ => Err(format!("argument {} must be string", index + 1)),
    }
}

pub(super) fn optional_string(request: &NativeCallRequest, index: usize) -> Option<&str> {
    match request.arguments.get(index) {
        Some(VmValue::String(value)) => Some(value),
        _ => None,
    }
}

pub(super) fn integer_argument(request: &NativeCallRequest, index: usize) -> Result<i64, String> {
    match request.arguments.get(index) {
        Some(VmValue::Integer(value)) => Ok(*value),
        _ => Err(format!("argument {} must be integer", index + 1)),
    }
}

pub(super) fn optional_integer(request: &NativeCallRequest, index: usize) -> Option<i64> {
    match request.arguments.get(index) {
        Some(VmValue::Integer(value)) => Some(*value),
        _ => None,
    }
}

pub(super) fn argument_key(request: &NativeCallRequest, index: usize) -> Result<String, String> {
    match request.arguments.get(index) {
        Some(VmValue::Integer(value)) => Ok(value.to_string()),
        Some(VmValue::String(value)) => Ok(value.clone()),
        _ => Err(format!("argument {} must be integer or string", index + 1)),
    }
}

pub(super) fn xml_target_key(request: &NativeCallRequest, index: usize) -> Result<String, String> {
    match request.arguments.get(index) {
        Some(VmValue::Integer(value)) => Ok(value.to_string()),
        Some(VmValue::String(value)) => Ok(value.clone()),
        Some(VmValue::StringPlace(_)) => xml_target_string(request, index).map(ToOwned::to_owned),
        _ => Err(format!(
            "argument {} must identify an XML document",
            index + 1
        )),
    }
}

pub(super) fn xml_target_string(request: &NativeCallRequest, index: usize) -> Result<&str, String> {
    match request.arguments.get(index) {
        Some(VmValue::String(value)) => Ok(value),
        Some(VmValue::StringPlace(_)) => {
            let place = explicit_place(request, index)?;
            match place.values.first() {
                Some(VmValue::String(value)) => Ok(value),
                _ => Err(format!("argument {} string place is unreadable", index + 1)),
            }
        }
        _ => Err(format!("argument {} must be a string", index + 1)),
    }
}

#[allow(clippy::unnecessary_wraps)]
pub(super) fn ready_integer(value: i64) -> Result<NativeReady, String> {
    Ok(NativeReady::value(VmValue::Integer(value)))
}

pub(super) fn explicit_place(
    request: &NativeCallRequest,
    argument_index: usize,
) -> Result<&NativePlaceView, String> {
    request
        .places
        .iter()
        .find(|place| place.argument_index == argument_index)
        .ok_or_else(|| format!("argument {} must be an array place", argument_index + 1))
}

pub(super) fn implicit_place<'a>(
    request: &'a NativeCallRequest,
    name: &str,
) -> Result<&'a NativePlaceView, String> {
    request
        .implicit_places
        .get(name)
        .ok_or_else(|| format!("implicit result place {name} is unavailable"))
}

pub(super) fn indexed_target(base: &PlaceDescriptor, index: usize) -> PlaceDescriptor {
    let mut target = base.clone();
    target.indices = vec![u64::try_from(index).unwrap_or(u64::MAX)];
    target
}

pub(super) fn array_writes(
    target: &NativePlaceView,
    start: usize,
    values: impl IntoIterator<Item = VmValue>,
) -> Vec<HostWrite> {
    values
        .into_iter()
        .take(target.values.len().saturating_sub(start))
        .enumerate()
        .map(|(offset, value)| HostWrite {
            target: indexed_target(&target.target, start + offset),
            value,
        })
        .collect()
}

pub(super) fn result_count_write(
    request: &NativeCallRequest,
    count: usize,
) -> Result<Vec<HostWrite>, String> {
    let result = implicit_place(request, "RESULT")?;
    Ok(array_writes(
        result,
        0,
        [VmValue::Integer(i64::try_from(count).unwrap_or(i64::MAX))],
    ))
}

pub(super) fn parse_data_type(value: &VmValue) -> Result<DataType, String> {
    match value {
        VmValue::Integer(1) => Ok(DataType::Int8),
        VmValue::Integer(2) => Ok(DataType::Int16),
        VmValue::Integer(3) => Ok(DataType::Int32),
        VmValue::Integer(4) => Ok(DataType::Int64),
        VmValue::Integer(5) => Ok(DataType::String),
        VmValue::String(name) => match name.to_ascii_lowercase().as_str() {
            "sbyte" | "int8" => Ok(DataType::Int8),
            "short" | "int16" => Ok(DataType::Int16),
            "int" | "int32" => Ok(DataType::Int32),
            "long" | "int64" => Ok(DataType::Int64),
            "string" => Ok(DataType::String),
            _ => Err(format!("unsupported DataTable type {name}")),
        },
        _ => Err("DataTable type must be an integer code or string name".into()),
    }
}

pub(super) const fn data_type_code(value: DataType) -> i64 {
    match value {
        DataType::Int8 => 1,
        DataType::Int16 => 2,
        DataType::Int32 => 3,
        DataType::Int64 => 4,
        DataType::String => 5,
    }
}

pub(super) fn data_table_pairs(
    request: &NativeCallRequest,
    start: usize,
) -> Result<Vec<(String, VmValue)>, String> {
    if let (Some(names), Some(values)) = (
        request
            .places
            .iter()
            .find(|place| place.argument_index == start),
        request
            .places
            .iter()
            .find(|place| place.argument_index == start + 1),
    ) {
        let count = usize::try_from(integer_argument(request, start + 2)?).unwrap_or(0);
        return names
            .values
            .iter()
            .zip(&values.values)
            .take(count)
            .map(|(name, value)| match name {
                VmValue::String(name) => Ok((name.clone(), value.clone())),
                _ => Err("DataTable column-name array must contain strings".into()),
            })
            .collect();
    }
    let tail = request.arguments.get(start..).unwrap_or_default();
    if !tail.len().is_multiple_of(2) {
        return Err("DataTable row values must be column/value pairs".into());
    }
    tail.chunks_exact(2)
        .map(|pair| match &pair[0] {
            VmValue::String(name) => Ok((name.clone(), pair[1].clone())),
            _ => Err("DataTable column name must be string".into()),
        })
        .collect()
}

pub(super) fn cell_for_column(column: &Column, value: &VmValue) -> Result<Cell, String> {
    match (column.value_type, value) {
        (DataType::String, VmValue::String(value)) => Ok(Cell::String(value.clone())),
        (DataType::String, _) => Err("string DataTable column requires a string".into()),
        (_, VmValue::Integer(value)) => {
            let value = match column.value_type {
                DataType::Int8 => i64::from(
                    i8::try_from(*value).map_err(|_| "integer exceeds Int8 DataTable column")?,
                ),
                DataType::Int16 => i64::from(
                    i16::try_from(*value).map_err(|_| "integer exceeds Int16 DataTable column")?,
                ),
                DataType::Int32 => i64::from(
                    i32::try_from(*value).map_err(|_| "integer exceeds Int32 DataTable column")?,
                ),
                DataType::Int64 => *value,
                DataType::String => unreachable!(),
            };
            Ok(Cell::Integer(value))
        }
        (_, _) => Err("numeric DataTable column requires an integer".into()),
    }
}

pub(super) fn row_matches(table: &DataTable, row: &DataRow, filter: &str) -> Result<bool, String> {
    let filter = filter.trim();
    if filter.is_empty() {
        return Ok(true);
    }
    for operator in ["<>", ">=", "<=", "=", ">", "<"] {
        if let Some((left, right)) = filter.split_once(operator) {
            let column = table
                .column(left.trim().trim_matches(['[', ']']))
                .ok_or_else(|| format!("unknown DataTable filter column {}", left.trim()))?;
            let cell = if column == 0 {
                Cell::Integer(row.id)
            } else {
                row.cells[column].clone()
            };
            let right = right.trim();
            let ordering = match cell {
                Cell::Integer(value) => value.cmp(
                    &right
                        .parse::<i64>()
                        .map_err(|_| "DataTable numeric filter literal is invalid")?,
                ),
                Cell::String(value) => {
                    let literal = right.trim_matches('\'');
                    if table.case_sensitive {
                        value.as_str().cmp(literal)
                    } else {
                        value
                            .to_ascii_lowercase()
                            .cmp(&literal.to_ascii_lowercase())
                    }
                }
                Cell::Null => return Ok(false),
            };
            return Ok(match operator {
                "=" => ordering.is_eq(),
                "<>" => !ordering.is_eq(),
                ">" => ordering.is_gt(),
                "<" => ordering.is_lt(),
                ">=" => !ordering.is_lt(),
                "<=" => !ordering.is_gt(),
                _ => unreachable!(),
            });
        }
    }
    Err("unsupported DataTable filter expression".into())
}

pub(super) fn sort_rows(
    table: &DataTable,
    rows: &mut [&DataRow],
    sort: &str,
) -> Result<(), String> {
    let mut parts = sort.split_whitespace();
    let name = parts.next().ok_or("DataTable sort column is missing")?;
    let descending = parts
        .next()
        .is_some_and(|direction| direction.eq_ignore_ascii_case("DESC"));
    if parts.next().is_some() {
        return Err("only one DataTable sort column is currently supported".into());
    }
    let column = table
        .column(name.trim_matches(['[', ']']))
        .ok_or_else(|| format!("unknown DataTable sort column {name}"))?;
    rows.sort_by(|left, right| {
        let ordering = if column == 0 {
            left.id.cmp(&right.id)
        } else {
            match (&left.cells[column], &right.cells[column]) {
                (Cell::Integer(left), Cell::Integer(right)) => left.cmp(right),
                (Cell::String(left), Cell::String(right)) => left.cmp(right),
                (Cell::Null, Cell::Null) => std::cmp::Ordering::Equal,
                (Cell::Null, _) => std::cmp::Ordering::Less,
                (_, Cell::Null) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            }
        };
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    });
    Ok(())
}
