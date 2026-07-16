use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use erabasic_bytecode::SymbolKey;
use erabasic_data::ExtensionData;
use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};

use crate::{HostWrite, NativeCallRequest, NativePlaceView, NativeReady, PlaceDescriptor, VmValue};

pub(crate) const STRUCTURED_BUNDLE_VERSION: u32 = 1;

pub(crate) fn bundle_key() -> SymbolKey {
    SymbolKey::derive("rustyera.native.bundle", b"structured-data-v1")
}

pub(crate) fn is_structured_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("map_") || name.starts_with("xml_") || name.starts_with("dt_")
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct StructuredState {
    maps: BTreeMap<String, OrderedMap>,
    xml_documents: BTreeMap<String, XmlDocument>,
    data_tables: BTreeMap<String, DataTable>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StructuredScope {
    Ordinary,
    Global,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StructuredExtension {
    Map {
        key: String,
        entries: Vec<(String, String)>,
    },
    Xml {
        key: String,
        document: String,
    },
    DataTable {
        key: String,
        schema: String,
        data: String,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct OrderedMap {
    entries: Vec<(String, String)>,
}

impl OrderedMap {
    fn position(&self, key: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|(candidate, _)| candidate == key)
    }

    fn set(&mut self, key: String, value: String) {
        if let Some(index) = self.position(&key) {
            self.entries[index].1 = value;
        } else {
            self.entries.push((key, value));
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct XmlDocument {
    root: XmlElement,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct XmlElement {
    name: String,
    attributes: Vec<(String, String)>,
    children: Vec<XmlChild>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum XmlChild {
    Element(XmlElement),
    Text(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum DataType {
    Int8,
    Int16,
    Int32,
    Int64,
    String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum Cell {
    Null,
    Integer(i64),
    String(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Column {
    name: String,
    value_type: DataType,
    nullable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct DataRow {
    id: i64,
    cells: Vec<Cell>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct DataTable {
    case_sensitive: bool,
    next_id: i64,
    columns: Vec<Column>,
    rows: Vec<DataRow>,
}

impl DataTable {
    fn new() -> Self {
        Self {
            case_sensitive: true,
            next_id: 1,
            columns: vec![Column {
                name: "id".into(),
                value_type: DataType::Int64,
                nullable: false,
            }],
            rows: Vec::new(),
        }
    }

    fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|column| {
            if self.case_sensitive {
                column.name == name
            } else {
                column.name.eq_ignore_ascii_case(name)
            }
        })
    }

    fn row(&self, index: i64, as_id: bool) -> Option<usize> {
        if as_id {
            self.rows.iter().position(|row| row.id == index)
        } else {
            usize::try_from(index)
                .ok()
                .filter(|index| *index < self.rows.len())
        }
    }
}

#[derive(Clone)]
pub(crate) struct StructuredNative {
    name: String,
    state: Arc<Mutex<StructuredState>>,
}

impl StructuredNative {
    pub(crate) fn new(name: impl Into<String>, state: Arc<Mutex<StructuredState>>) -> Self {
        Self {
            name: name.into(),
            state,
        }
    }
}

impl crate::NativeService for StructuredNative {
    fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "structured native state lock is poisoned".to_owned())?;
        state.call(&self.name, &request)
    }

    // The registry serializes the shared bundle once under a stable bundle key.
    fn snapshot(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(Some(Vec::new()))
    }
}

impl StructuredState {
    pub(crate) fn encode(&self) -> Result<Vec<u8>, String> {
        let payload = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        let mut output = STRUCTURED_BUNDLE_VERSION.to_le_bytes().to_vec();
        output.extend(payload);
        Ok(output)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, String> {
        let version = bytes
            .get(..4)
            .and_then(|value| value.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| "structured native bundle is truncated".to_owned())?;
        if version != STRUCTURED_BUNDLE_VERSION {
            return Err(format!(
                "unsupported structured native bundle version {version}"
            ));
        }
        serde_json::from_slice(&bytes[4..]).map_err(|error| error.to_string())
    }

    pub(crate) fn clear_for_transaction(
        &mut self,
        extensions: &ExtensionData,
        transaction: &crate::VmRuntimeStateTransaction,
    ) {
        match transaction {
            crate::VmRuntimeStateTransaction::ResetNewGame => *self = Self::default(),
            crate::VmRuntimeStateTransaction::ResetGameData
            | crate::VmRuntimeStateTransaction::RestoreOrdinary(_) => {
                self.clear_declared(
                    &extensions.save_maps,
                    &extensions.save_xmls,
                    &extensions.save_data_tables,
                );
            }
            crate::VmRuntimeStateTransaction::ResetGlobalData => {
                self.clear_declared(
                    &extensions.global_maps,
                    &extensions.global_xmls,
                    &extensions.global_data_tables,
                );
                self.clear_declared(
                    &extensions.static_maps,
                    &extensions.static_xmls,
                    &extensions.static_data_tables,
                );
            }
            crate::VmRuntimeStateTransaction::OverlayGlobal(_) => {
                self.clear_declared(
                    &extensions.global_maps,
                    &extensions.global_xmls,
                    &extensions.global_data_tables,
                );
            }
            crate::VmRuntimeStateTransaction::AppendCharacters(_)
            | crate::VmRuntimeStateTransaction::SetLastLoad { .. }
            | crate::VmRuntimeStateTransaction::Mutate { .. } => {}
        }
    }

    fn clear_declared(
        &mut self,
        maps: &BTreeSet<String>,
        xmls: &BTreeSet<String>,
        tables: &BTreeSet<String>,
    ) {
        for key in maps {
            if let Some(map) = self.maps.get_mut(key) {
                map.entries.clear();
            }
        }
        for key in xmls {
            self.xml_documents.remove(key);
        }
        for key in tables {
            if let Some(table) = self.data_tables.get_mut(key) {
                table.rows.clear();
            }
        }
    }

    pub(crate) fn export_extensions(
        &self,
        declarations: &ExtensionData,
        scope: StructuredScope,
    ) -> Result<Vec<StructuredExtension>, String> {
        let (maps, xmls, tables) = match scope {
            StructuredScope::Ordinary => (
                &declarations.save_maps,
                &declarations.save_xmls,
                &declarations.save_data_tables,
            ),
            StructuredScope::Global => (
                &declarations.global_maps,
                &declarations.global_xmls,
                &declarations.global_data_tables,
            ),
        };
        let mut output = Vec::new();
        for key in maps {
            if let Some(map) = self.maps.get(key) {
                output.push(StructuredExtension::Map {
                    key: key.clone(),
                    entries: map.entries.clone(),
                });
            }
        }
        for key in xmls {
            if let Some(document) = self.xml_documents.get(key) {
                output.push(StructuredExtension::Xml {
                    key: key.clone(),
                    document: document.outer_xml(),
                });
            }
        }
        for key in tables {
            if let Some(table) = self.data_tables.get(key) {
                output.push(StructuredExtension::DataTable {
                    key: key.clone(),
                    schema: serde_json::to_string(&table.columns)
                        .map_err(|error| error.to_string())?,
                    data: serde_json::to_string(table).map_err(|error| error.to_string())?,
                });
            }
        }
        Ok(output)
    }

    pub(crate) fn import_extensions(
        &mut self,
        declarations: &ExtensionData,
        scope: StructuredScope,
        values: &[StructuredExtension],
    ) -> Result<BTreeSet<(u8, String)>, String> {
        let (maps, xmls, tables) = match scope {
            StructuredScope::Ordinary => (
                &declarations.save_maps,
                &declarations.save_xmls,
                &declarations.save_data_tables,
            ),
            StructuredScope::Global => (
                &declarations.global_maps,
                &declarations.global_xmls,
                &declarations.global_data_tables,
            ),
        };
        let mut imported = BTreeSet::new();
        for value in values {
            match value {
                StructuredExtension::Map { key, entries } if maps.contains(key) => {
                    let mut map = OrderedMap::default();
                    for (entry_key, value) in entries {
                        map.set(entry_key.clone(), value.clone());
                    }
                    self.maps.insert(key.clone(), map);
                    imported.insert((0x20, key.clone()));
                }
                StructuredExtension::Xml { key, document } if xmls.contains(key) => {
                    self.xml_documents.insert(key.clone(), parse_xml(document)?);
                    imported.insert((0x21, key.clone()));
                }
                StructuredExtension::DataTable { key, schema, data } if tables.contains(key) => {
                    let columns: Vec<Column> =
                        serde_json::from_str(schema).map_err(|error| error.to_string())?;
                    let mut table: DataTable =
                        serde_json::from_str(data).map_err(|error| error.to_string())?;
                    if table.columns != columns {
                        return Err(format!("DataTable extension {key} schema differs"));
                    }
                    table.next_id = table
                        .rows
                        .iter()
                        .map(|row| row.id)
                        .max()
                        .unwrap_or(0)
                        .checked_add(1)
                        .ok_or_else(|| format!("DataTable extension {key} row id overflowed"))?;
                    self.data_tables.insert(key.clone(), table);
                    imported.insert((0x22, key.clone()));
                }
                _ => {}
            }
        }
        Ok(imported)
    }

    #[allow(clippy::too_many_lines)]
    fn call(&mut self, name: &str, request: &NativeCallRequest) -> Result<NativeReady, String> {
        let name = name.to_ascii_lowercase();
        if name.starts_with("map_") {
            return self.call_map(&name, request);
        }
        if name.starts_with("xml_") {
            return self.call_xml(&name, request);
        }
        if name.starts_with("dt_") {
            return self.call_data_table(&name, request);
        }
        Err(format!("unknown structured native service {name}"))
    }

    #[allow(clippy::too_many_lines)]
    fn call_map(&mut self, name: &str, request: &NativeCallRequest) -> Result<NativeReady, String> {
        let map_name = string_argument(request, 0)?.to_owned();
        match name {
            "map_create" => {
                if let std::collections::btree_map::Entry::Vacant(entry) = self.maps.entry(map_name)
                {
                    entry.insert(OrderedMap::default());
                    ready_integer(1)
                } else {
                    ready_integer(0)
                }
            }
            "map_exist" => ready_integer(i64::from(self.maps.contains_key(&map_name))),
            "map_release" => {
                self.maps.remove(&map_name);
                ready_integer(1)
            }
            "map_get" => {
                let key = string_argument(request, 1)?;
                let value = self
                    .maps
                    .get(&map_name)
                    .and_then(|map| map.position(key).map(|index| map.entries[index].1.clone()))
                    .unwrap_or_default();
                Ok(NativeReady::value(VmValue::String(value)))
            }
            "map_has" => {
                let Some(map) = self.maps.get(&map_name) else {
                    return ready_integer(-1);
                };
                ready_integer(i64::from(
                    map.position(string_argument(request, 1)?).is_some(),
                ))
            }
            "map_set" => {
                let key = string_argument(request, 1)?.to_owned();
                let value = string_argument(request, 2)?.to_owned();
                let Some(map) = self.maps.get_mut(&map_name) else {
                    return ready_integer(-1);
                };
                map.set(key, value);
                ready_integer(1)
            }
            "map_remove" => {
                let key = string_argument(request, 1)?;
                let Some(map) = self.maps.get_mut(&map_name) else {
                    return ready_integer(-1);
                };
                if let Some(index) = map.position(key) {
                    map.entries.remove(index);
                }
                ready_integer(1)
            }
            "map_clear" => {
                let Some(map) = self.maps.get_mut(&map_name) else {
                    return ready_integer(-1);
                };
                map.entries.clear();
                ready_integer(1)
            }
            "map_size" => self.maps.get(&map_name).map_or_else(
                || ready_integer(-1),
                |map| ready_integer(i64::try_from(map.entries.len()).unwrap_or(i64::MAX)),
            ),
            "map_getkeys" => {
                let Some(map) = self.maps.get(&map_name) else {
                    return Ok(NativeReady::value(VmValue::String(String::new())));
                };
                let keys = map
                    .entries
                    .iter()
                    .map(|(key, _)| key.clone())
                    .collect::<Vec<_>>();
                if request.arguments.len() == 1 {
                    return Ok(NativeReady::value(VmValue::String(keys.join(","))));
                }
                let enabled_index = if request.arguments.len() == 2 { 1 } else { 2 };
                if integer_argument(request, enabled_index)? == 0 {
                    return Ok(NativeReady::value(VmValue::String(String::new())));
                }
                let mut writes = result_count_write(request, keys.len())?;
                let target = if request.arguments.len() == 2 {
                    implicit_place(request, "RESULTS")?
                } else {
                    explicit_place(request, 1)?
                };
                writes.extend(array_writes(
                    target,
                    0,
                    keys.iter().cloned().map(VmValue::String),
                ));
                let value = if request.arguments.len() == 2 {
                    keys.first().cloned().unwrap_or_default()
                } else {
                    String::new()
                };
                Ok(NativeReady {
                    value: Some(VmValue::String(value)),
                    writes,
                })
            }
            "map_toxml" => {
                let value = self.maps.get(&map_name).map_or_else(String::new, |map| {
                    let mut xml = String::from("<map>");
                    for (key, value) in &map.entries {
                        xml.push_str("<p><k>");
                        xml.push_str(key);
                        xml.push_str("</k><v>");
                        xml.push_str(value);
                        xml.push_str("</v></p>");
                    }
                    xml.push_str("</map>");
                    xml
                });
                Ok(NativeReady::value(VmValue::String(value)))
            }
            "map_fromxml" => {
                let xml = string_argument(request, 1)?;
                let Some(_) = self.maps.get(&map_name) else {
                    return ready_integer(0);
                };
                let document = parse_xml(xml)?;
                if document.root.name != "map" {
                    return ready_integer(1);
                }
                let mut incoming = Vec::new();
                for child in document.root.elements_named("p") {
                    let keys = child.elements_named("k");
                    let values = child.elements_named("v");
                    if keys.len() == 1 && values.len() == 1 {
                        incoming.push((keys[0].inner_text(), values[0].inner_xml()));
                    }
                }
                let map = self.maps.get_mut(&map_name).expect("checked above");
                for (key, value) in incoming {
                    map.set(key, value);
                }
                ready_integer(1)
            }
            _ => Err(format!("unsupported map native {name}")),
        }
    }

    fn call_xml(&mut self, name: &str, request: &NativeCallRequest) -> Result<NativeReady, String> {
        let id = argument_key(request, 0)?;
        match name {
            "xml_document" => {
                if self.xml_documents.contains_key(&id) {
                    return ready_integer(0);
                }
                let document = parse_xml(string_argument(request, 1)?)?;
                self.xml_documents.insert(id, document);
                ready_integer(1)
            }
            "xml_exist" => ready_integer(i64::from(self.xml_documents.contains_key(&id))),
            "xml_release" => {
                if self.xml_documents.remove(&id).is_some() {
                    ready_integer(1)
                } else {
                    ready_integer(0)
                }
            }
            "xml_tostr" => Ok(NativeReady::value(VmValue::String(
                self.xml_documents
                    .get(&id)
                    .map_or_else(String::new, XmlDocument::outer_xml),
            ))),
            "xml_get" | "xml_get_byname" => {
                let Some(document) = self.xml_documents.get(&id) else {
                    return ready_integer(-1);
                };
                let selected = document.select(string_argument(request, 1)?)?;
                let style = optional_integer(request, 3).unwrap_or(0);
                let values = selected
                    .iter()
                    .map(|element| match style {
                        1 => element.inner_text(),
                        2 => element.inner_xml(),
                        3 => element.outer_xml(),
                        4 => element.name.clone(),
                        _ => String::new(),
                    })
                    .collect::<Vec<_>>();
                let mut writes = Vec::new();
                if request.arguments.len() >= 3 {
                    let target = if matches!(request.arguments.get(2), Some(VmValue::Integer(value)) if *value != 0)
                    {
                        Some(implicit_place(request, "RESULTS")?)
                    } else {
                        request
                            .places
                            .iter()
                            .find(|place| place.argument_index == 2)
                    };
                    if let Some(target) = target {
                        writes.extend(array_writes(
                            target,
                            0,
                            values.into_iter().map(VmValue::String),
                        ));
                    }
                }
                Ok(NativeReady {
                    value: Some(VmValue::Integer(
                        i64::try_from(selected.len()).unwrap_or(i64::MAX),
                    )),
                    writes,
                })
            }
            _ => Err(format!(
                "XML operation {name} is outside the pinned XPath mutation subset"
            )),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn call_data_table(
        &mut self,
        name: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, String> {
        let key = string_argument(request, 0)?.to_owned();
        match name {
            "dt_create" => {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    self.data_tables.entry(key)
                {
                    entry.insert(DataTable::new());
                    ready_integer(1)
                } else {
                    ready_integer(0)
                }
            }
            "dt_exist" => ready_integer(i64::from(self.data_tables.contains_key(&key))),
            "dt_release" => {
                self.data_tables.remove(&key);
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
                table.columns.push(Column {
                    name: column_name,
                    value_type,
                    nullable,
                });
                for row in &mut table.rows {
                    row.cells.push(Cell::Null);
                }
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
                table.columns.remove(index);
                for row in &mut table.rows {
                    row.cells.remove(index);
                }
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
                            return Err("DT_CELL_GET cannot read a string column".into());
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
            _ => Err(format!("unsupported data-table native {name}")),
        }
    }

    fn data_table_row_add(
        &mut self,
        key: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, String> {
        let Some(table) = self.data_tables.get_mut(key) else {
            return ready_integer(-1);
        };
        let id = table.next_id;
        table.next_id = table
            .next_id
            .checked_add(1)
            .ok_or_else(|| "DT_ROW_ADD deterministic row id overflowed".to_owned())?;
        let mut row = DataRow {
            id,
            cells: table.columns.iter().map(|_| Cell::Null).collect(),
        };
        let pairs = data_table_pairs(request, 1)?;
        for (name, value) in pairs {
            let column = table
                .column(&name)
                .ok_or_else(|| format!("DT_ROW_ADD table {key} has no column {name}"))?;
            if column == 0 {
                return Err("DT_ROW_ADD cannot edit the id column".into());
            }
            row.cells[column] = cell_for_column(&table.columns[column], &value)?;
        }
        table.rows.push(row);
        ready_integer(id)
    }

    fn data_table_row_set(
        &mut self,
        key: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, String> {
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
            let column = table
                .column(&name)
                .ok_or_else(|| format!("DT_ROW_SET table {key} has no column {name}"))?;
            if column == 0 {
                return Err("DT_ROW_SET cannot edit the id column".into());
            }
            changes.push((column, cell_for_column(&table.columns[column], &value)?));
        }
        let count = changes.len();
        for (column, value) in changes {
            table.rows[row_index].cells[column] = value;
        }
        ready_integer(i64::try_from(count).unwrap_or(i64::MAX))
    }

    fn data_table_row_remove(
        &mut self,
        key: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, String> {
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
    ) -> Result<NativeReady, String> {
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
        let Ok(value) = value else {
            return ready_integer(-2);
        };
        table.rows[row].cells[column] = value;
        ready_integer(1)
    }

    fn data_table_select(
        &self,
        key: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, String> {
        let Some(table) = self.data_tables.get(key) else {
            return ready_integer(-1);
        };
        let filter = optional_string(request, 1);
        let sort = optional_string(request, 2);
        let mut rows = table
            .rows
            .iter()
            .filter(|row| {
                filter.is_none_or(|filter| row_matches(table, row, filter).unwrap_or(false))
            })
            .collect::<Vec<_>>();
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
    ) -> Result<NativeReady, String> {
        let Some(table) = self.data_tables.get(key) else {
            return Ok(NativeReady::value(VmValue::String(String::new())));
        };
        let schema = serde_json::to_string(&table.columns).map_err(|error| error.to_string())?;
        let data = serde_json::to_string(&table).map_err(|error| error.to_string())?;
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
    ) -> Result<NativeReady, String> {
        let schema: Vec<Column> = match serde_json::from_str(string_argument(request, 1)?) {
            Ok(schema) => schema,
            Err(_) => return ready_integer(0),
        };
        let mut table: DataTable = match serde_json::from_str(string_argument(request, 2)?) {
            Ok(table) => table,
            Err(_) => return ready_integer(0),
        };
        if table.columns != schema
            || table
                .columns
                .first()
                .is_none_or(|column| column.name != "id")
        {
            return ready_integer(0);
        }
        table.next_id = table
            .rows
            .iter()
            .map(|row| row.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| "DT_FROMXML row id overflowed".to_owned())?;
        self.data_tables.insert(key.to_owned(), table);
        ready_integer(1)
    }
}

fn string_argument(request: &NativeCallRequest, index: usize) -> Result<&str, String> {
    match request.arguments.get(index) {
        Some(VmValue::String(value)) => Ok(value),
        _ => Err(format!("argument {} must be string", index + 1)),
    }
}

fn optional_string(request: &NativeCallRequest, index: usize) -> Option<&str> {
    match request.arguments.get(index) {
        Some(VmValue::String(value)) => Some(value),
        _ => None,
    }
}

fn integer_argument(request: &NativeCallRequest, index: usize) -> Result<i64, String> {
    match request.arguments.get(index) {
        Some(VmValue::Integer(value)) => Ok(*value),
        _ => Err(format!("argument {} must be integer", index + 1)),
    }
}

fn optional_integer(request: &NativeCallRequest, index: usize) -> Option<i64> {
    match request.arguments.get(index) {
        Some(VmValue::Integer(value)) => Some(*value),
        _ => None,
    }
}

fn argument_key(request: &NativeCallRequest, index: usize) -> Result<String, String> {
    match request.arguments.get(index) {
        Some(VmValue::Integer(value)) => Ok(value.to_string()),
        Some(VmValue::String(value)) => Ok(value.clone()),
        _ => Err(format!("argument {} must be integer or string", index + 1)),
    }
}

#[allow(clippy::unnecessary_wraps)]
fn ready_integer(value: i64) -> Result<NativeReady, String> {
    Ok(NativeReady::value(VmValue::Integer(value)))
}

fn explicit_place(
    request: &NativeCallRequest,
    argument_index: usize,
) -> Result<&NativePlaceView, String> {
    request
        .places
        .iter()
        .find(|place| place.argument_index == argument_index)
        .ok_or_else(|| format!("argument {} must be an array place", argument_index + 1))
}

fn implicit_place<'a>(
    request: &'a NativeCallRequest,
    name: &str,
) -> Result<&'a NativePlaceView, String> {
    request
        .implicit_places
        .get(name)
        .ok_or_else(|| format!("implicit result place {name} is unavailable"))
}

fn indexed_target(base: &PlaceDescriptor, index: usize) -> PlaceDescriptor {
    let mut target = base.clone();
    target.indices = vec![u64::try_from(index).unwrap_or(u64::MAX)];
    target
}

fn array_writes(
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

fn result_count_write(request: &NativeCallRequest, count: usize) -> Result<Vec<HostWrite>, String> {
    let result = implicit_place(request, "RESULT")?;
    Ok(array_writes(
        result,
        0,
        [VmValue::Integer(i64::try_from(count).unwrap_or(i64::MAX))],
    ))
}

fn parse_data_type(value: &VmValue) -> Result<DataType, String> {
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

const fn data_type_code(value: DataType) -> i64 {
    match value {
        DataType::Int8 => 1,
        DataType::Int16 => 2,
        DataType::Int32 => 3,
        DataType::Int64 => 4,
        DataType::String => 5,
    }
}

fn data_table_pairs(
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

fn cell_for_column(column: &Column, value: &VmValue) -> Result<Cell, String> {
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

fn row_matches(table: &DataTable, row: &DataRow, filter: &str) -> Result<bool, String> {
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

fn sort_rows(table: &DataTable, rows: &mut [&DataRow], sort: &str) -> Result<(), String> {
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

fn parse_xml(input: &str) -> Result<XmlDocument, String> {
    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(false);
    let mut stack = Vec::<XmlElement>::new();
    let mut root = None;
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(start) => {
                let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
                let attributes = start
                    .attributes()
                    .map(|attribute| {
                        let attribute = attribute.map_err(|error| error.to_string())?;
                        Ok((
                            String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                            attribute
                                .decode_and_unescape_value(reader.decoder())
                                .map_err(|error| error.to_string())?
                                .into_owned(),
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                stack.push(XmlElement {
                    name,
                    attributes,
                    children: Vec::new(),
                });
            }
            Event::Empty(start) => {
                let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
                let element = XmlElement {
                    name,
                    attributes: start
                        .attributes()
                        .map(|attribute| {
                            let attribute = attribute.map_err(|error| error.to_string())?;
                            Ok((
                                String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                                attribute
                                    .decode_and_unescape_value(reader.decoder())
                                    .map_err(|error| error.to_string())?
                                    .into_owned(),
                            ))
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                    children: Vec::new(),
                };
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(XmlChild::Element(element));
                } else if root.replace(element).is_some() {
                    return Err("XML contains more than one root element".into());
                }
            }
            Event::Text(text) => {
                let value = text.decode().map_err(|error| error.to_string())?;
                let value = quick_xml::escape::unescape(&value)
                    .map_err(|error| error.to_string())?
                    .into_owned();
                let parent = stack
                    .last_mut()
                    .ok_or("XML text appears outside the root element")?;
                parent.children.push(XmlChild::Text(value));
            }
            Event::CData(text) => {
                let value = text
                    .decode()
                    .map_err(|error| error.to_string())?
                    .into_owned();
                let parent = stack
                    .last_mut()
                    .ok_or("XML CDATA appears outside the root element")?;
                parent.children.push(XmlChild::Text(value));
            }
            Event::End(_) => {
                let element = stack.pop().ok_or("XML contains an unmatched close tag")?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(XmlChild::Element(element));
                } else if root.replace(element).is_some() {
                    return Err("XML contains more than one root element".into());
                }
            }
            Event::Decl(_)
            | Event::Comment(_)
            | Event::PI(_)
            | Event::DocType(_)
            | Event::GeneralRef(_) => {}
            Event::Eof => break,
        }
    }
    if !stack.is_empty() {
        return Err("XML document ended before all elements were closed".into());
    }
    Ok(XmlDocument {
        root: root.ok_or("XML document has no root element")?,
    })
}

impl XmlDocument {
    fn outer_xml(&self) -> String {
        self.root.outer_xml()
    }

    fn select(&self, path: &str) -> Result<Vec<&XmlElement>, String> {
        let path = path.trim();
        if let Some(name) = path.strip_prefix("//") {
            if name.is_empty() || name.contains(['/', '[', '@']) {
                return Err("unsupported XPath descendant expression".into());
            }
            let mut output = Vec::new();
            self.root.descendants_named(name, &mut output);
            return Ok(output);
        }
        let absolute = path.starts_with('/');
        let parts = path
            .trim_start_matches("./")
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.iter().any(|part| part.contains(['[', '@', ':'])) {
            return Err("unsupported XPath predicate, attribute, or namespace expression".into());
        }
        let mut current = vec![&self.root];
        let mut offset = 0;
        if absolute && parts.first().is_some_and(|part| *part == self.root.name) {
            offset = 1;
        }
        for part in &parts[offset..] {
            let mut next = Vec::new();
            for element in current {
                next.extend(element.elements_named(part));
            }
            current = next;
        }
        Ok(current)
    }
}

impl XmlElement {
    fn elements_named(&self, name: &str) -> Vec<&Self> {
        self.children
            .iter()
            .filter_map(|child| match child {
                XmlChild::Element(element) if name == "*" || element.name == name => Some(element),
                XmlChild::Element(_) | XmlChild::Text(_) => None,
            })
            .collect()
    }

    fn descendants_named<'a>(&'a self, name: &str, output: &mut Vec<&'a Self>) {
        if self.name == name || name == "*" {
            output.push(self);
        }
        for child in &self.children {
            if let XmlChild::Element(element) = child {
                element.descendants_named(name, output);
            }
        }
    }

    fn inner_text(&self) -> String {
        let mut output = String::new();
        for child in &self.children {
            match child {
                XmlChild::Text(value) => output.push_str(value),
                XmlChild::Element(element) => output.push_str(&element.inner_text()),
            }
        }
        output
    }

    fn inner_xml(&self) -> String {
        let mut output = String::new();
        for child in &self.children {
            match child {
                XmlChild::Text(value) => output.push_str(&xml_escape(value)),
                XmlChild::Element(element) => output.push_str(&element.outer_xml()),
            }
        }
        output
    }

    fn outer_xml(&self) -> String {
        let mut output = String::new();
        output.push('<');
        output.push_str(&self.name);
        for (name, value) in &self.attributes {
            output.push(' ');
            output.push_str(name);
            output.push_str("=\"");
            output.push_str(&xml_escape(value));
            output.push('"');
        }
        if self.children.is_empty() {
            output.push_str(" />");
        } else {
            output.push('>');
            output.push_str(&self.inner_xml());
            output.push_str("</");
            output.push_str(&self.name);
            output.push('>');
        }
        output
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_map_overwrite_keeps_insertion_position() {
        let mut map = OrderedMap::default();
        map.set("b".into(), "1".into());
        map.set("a".into(), "2".into());
        map.set("b".into(), "3".into());
        assert_eq!(
            map.entries,
            vec![("b".into(), "3".into()), ("a".into(), "2".into())]
        );
    }

    #[test]
    fn xml_subset_preserves_mixed_content_and_selects_paths() {
        let document = parse_xml("<root a='x'>A<p><k>one</k></p>B</root>").unwrap();
        assert_eq!(document.root.inner_text(), "AoneB");
        assert_eq!(document.select("/root/p/k").unwrap()[0].inner_text(), "one");
        assert_eq!(parse_xml(&document.outer_xml()).unwrap(), document);
    }

    #[test]
    fn deterministic_table_ids_are_monotonic() {
        let table = DataTable::new();
        assert_eq!(table.next_id, 1);
        assert_eq!(table.columns[0].name, "id");
    }

    #[test]
    fn extension_scopes_clear_and_import_without_touching_other_scopes() {
        let declarations = ExtensionData {
            save_maps: BTreeSet::from(["save".into()]),
            global_maps: BTreeSet::from(["global".into()]),
            static_maps: BTreeSet::from(["static".into()]),
            ..ExtensionData::default()
        };
        let mut state = StructuredState::default();
        for key in ["save", "global", "static"] {
            let mut map = OrderedMap::default();
            map.set("key".into(), key.into());
            state.maps.insert(key.into(), map);
        }

        state.clear_for_transaction(
            &declarations,
            &crate::VmRuntimeStateTransaction::ResetGameData,
        );
        assert!(state.maps["save"].entries.is_empty());
        assert!(!state.maps["global"].entries.is_empty());
        assert!(!state.maps["static"].entries.is_empty());

        state.clear_for_transaction(
            &declarations,
            &crate::VmRuntimeStateTransaction::ResetGlobalData,
        );
        assert!(state.maps["global"].entries.is_empty());
        assert!(state.maps["static"].entries.is_empty());

        let imported = state
            .import_extensions(
                &declarations,
                StructuredScope::Ordinary,
                &[
                    StructuredExtension::Map {
                        key: "save".into(),
                        entries: vec![("a".into(), "1".into())],
                    },
                    StructuredExtension::Map {
                        key: "undeclared".into(),
                        entries: vec![("b".into(), "2".into())],
                    },
                ],
            )
            .unwrap();
        assert_eq!(imported, BTreeSet::from([(0x20, "save".into())]));
        assert_eq!(state.maps["save"].entries, vec![("a".into(), "1".into())]);
        assert!(!state.maps.contains_key("undeclared"));
    }
}
