use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use erabasic_bytecode::SymbolKey;
use erabasic_data::ExtensionData;
use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};

use crate::{HostWrite, NativeCallRequest, NativePlaceView, NativeReady, PlaceDescriptor, VmValue};

mod data_table;
mod xml;

use data_table::{
    argument_key, array_writes, cell_for_column, data_table_data_xml, data_table_pairs,
    data_table_schema_xml, data_type_code, explicit_place, implicit_place, integer_argument,
    optional_integer, optional_string, parse_data_table_schema, parse_data_table_xml,
    parse_data_type, ready_integer, result_count_write, row_matches, sort_rows, string_argument,
    xml_target_key, xml_target_string,
};
use xml::{parse_xml, xml_attribute_escape, xml_text_escape};

pub(crate) const STRUCTURED_BUNDLE_VERSION: u32 = 2;

pub(crate) fn bundle_key() -> SymbolKey {
    SymbolKey::derive("rustyera.native.bundle", b"structured-data-v2")
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct XmlSelection {
    element_path: Vec<usize>,
    attribute: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum XmlMutation {
    Set,
    AddNode,
    AddAttribute,
    RemoveNode,
    RemoveAttribute,
    Replace,
}

enum XmlTarget {
    Stored(String),
    Inline(PlaceDescriptor),
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
    fn implicit_place_names(&self) -> &'static [&'static str] {
        &["RESULT", "RESULTS"]
    }

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
        match name {
            "xml_document" => {
                let id = argument_key(request, 0)?;
                if self.xml_documents.contains_key(&id) {
                    return ready_integer(0);
                }
                let document = parse_xml(string_argument(request, 1)?)?;
                self.xml_documents.insert(id, document);
                ready_integer(1)
            }
            "xml_exist" => {
                let id = argument_key(request, 0)?;
                ready_integer(i64::from(self.xml_documents.contains_key(&id)))
            }
            "xml_release" => {
                let id = argument_key(request, 0)?;
                if self.xml_documents.remove(&id).is_some() {
                    ready_integer(1)
                } else {
                    ready_integer(0)
                }
            }
            "xml_tostr" => Ok(NativeReady::value(VmValue::String(
                self.xml_documents
                    .get(&argument_key(request, 0)?)
                    .map_or_else(String::new, XmlDocument::outer_xml),
            ))),
            "xml_get" | "xml_get_byname" => {
                let inline;
                let document = if name == "xml_get_byname"
                    || matches!(request.arguments.first(), Some(VmValue::Integer(_)))
                {
                    let id = argument_key(request, 0)?;
                    let Some(document) = self.xml_documents.get(&id) else {
                        return ready_integer(-1);
                    };
                    document
                } else {
                    inline = parse_xml(xml_target_string(request, 0)?)?;
                    &inline
                };
                let selected = document.select(string_argument(request, 1)?)?;
                let style = optional_integer(request, 3).unwrap_or(0);
                let values = selected
                    .iter()
                    .map(|selection| document.selection_value(selection, style))
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
            "xml_set" | "xml_set_byname" => {
                self.mutate_xml(name.ends_with("_byname"), request, XmlMutation::Set)
            }
            "xml_addnode" | "xml_addnode_byname" => {
                self.mutate_xml(name.ends_with("_byname"), request, XmlMutation::AddNode)
            }
            "xml_addattribute" | "xml_addattribute_byname" => self.mutate_xml(
                name.ends_with("_byname"),
                request,
                XmlMutation::AddAttribute,
            ),
            "xml_removenode" | "xml_removenode_byname" => {
                self.mutate_xml(name.ends_with("_byname"), request, XmlMutation::RemoveNode)
            }
            "xml_removeattribute" | "xml_removeattribute_byname" => self.mutate_xml(
                name.ends_with("_byname"),
                request,
                XmlMutation::RemoveAttribute,
            ),
            "xml_replace" | "xml_replace_byname" => {
                self.mutate_xml(name.ends_with("_byname"), request, XmlMutation::Replace)
            }
            _ => Err(format!(
                "XML operation {name} is outside the pinned XPath mutation subset"
            )),
        }
    }

    fn mutate_xml(
        &mut self,
        by_name: bool,
        request: &NativeCallRequest,
        mutation: XmlMutation,
    ) -> Result<NativeReady, String> {
        let target = if by_name
            || matches!(request.arguments.first(), Some(VmValue::Integer(_)))
            || mutation == XmlMutation::Replace && request.arguments.len() == 2
        {
            XmlTarget::Stored(xml_target_key(request, 0)?)
        } else {
            XmlTarget::Inline(explicit_place(request, 0)?.target.clone())
        };
        let mut candidate = match &target {
            XmlTarget::Stored(id) => {
                let Some(document) = self.xml_documents.get(id) else {
                    return ready_integer(-1);
                };
                document.clone()
            }
            XmlTarget::Inline(_) => parse_xml(xml_target_string(request, 0)?)?,
        };
        if mutation == XmlMutation::Replace && request.arguments.len() == 2 {
            let replacement = parse_xml(string_argument(request, 1)?)?;
            let XmlTarget::Stored(id) = target else {
                unreachable!("two-argument XML_REPLACE always resolves a stored document")
            };
            self.xml_documents.insert(id, replacement);
            return ready_integer(1);
        }
        let selected = candidate.select(string_argument(request, 1)?)?;
        let selected_count = selected.len();
        let set_all_index = match mutation {
            XmlMutation::Set | XmlMutation::Replace => 3,
            XmlMutation::AddNode => 4,
            XmlMutation::AddAttribute => 5,
            XmlMutation::RemoveNode | XmlMutation::RemoveAttribute => 2,
        };
        let set_all = optional_integer(request, set_all_index).is_some_and(|value| value != 0);
        let apply = if selected_count <= 1 || set_all {
            selected.clone()
        } else {
            Vec::new()
        };
        let applied = candidate.apply_mutation(mutation, request, &apply)?;
        if selected_count == 1 && !applied {
            return ready_integer(0);
        }
        let xml = candidate.outer_xml();
        let writes = match target {
            XmlTarget::Stored(id) => {
                self.xml_documents.insert(id, candidate);
                Vec::new()
            }
            XmlTarget::Inline(target) => vec![HostWrite {
                target,
                value: VmValue::String(xml),
            }],
        };
        Ok(NativeReady {
            value: Some(VmValue::Integer(
                i64::try_from(selected_count).unwrap_or(i64::MAX),
            )),
            writes,
        })
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
    ) -> Result<NativeReady, String> {
        let Ok(schema) = parse_data_table_schema(key, string_argument(request, 1)?) else {
            return ready_integer(0);
        };
        let Ok(mut table) = parse_data_table_xml(key, &schema, string_argument(request, 2)?) else {
            return ready_integer(0);
        };
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

#[cfg(test)]
mod tests;
