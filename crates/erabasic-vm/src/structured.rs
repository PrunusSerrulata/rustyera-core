use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use erabasic_bytecode::SymbolKey;
use erabasic_data::ExtensionData;
use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};

use crate::{HostWrite, NativeCallRequest, NativePlaceView, NativeReady, PlaceDescriptor, VmValue};

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

fn data_table_schema_xml(key: &str, table: &DataTable) -> String {
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

fn data_table_data_xml(key: &str, table: &DataTable) -> String {
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

fn parse_data_table_schema(key: &str, xml: &str) -> Result<DataTable, String> {
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

fn parse_data_table_xml(key: &str, schema: &DataTable, xml: &str) -> Result<DataTable, String> {
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

fn collect_elements<'a>(element: &'a XmlElement, name: &str, output: &mut Vec<&'a XmlElement>) {
    if element.name == name {
        output.push(element);
    }
    for child in &element.children {
        if let XmlChild::Element(child) = child {
            collect_elements(child, name, output);
        }
    }
}

fn encode_xml_name(value: &str) -> String {
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

fn decode_xml_name(value: &str) -> Result<String, String> {
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

fn xml_target_key(request: &NativeCallRequest, index: usize) -> Result<String, String> {
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

fn xml_target_string(request: &NativeCallRequest, index: usize) -> Result<&str, String> {
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

#[allow(clippy::too_many_lines)]
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
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(XmlChild::Text(value));
                } else if !value.trim().is_empty() {
                    return Err("XML text appears outside the root element".into());
                }
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
            Event::GeneralRef(reference) => {
                let reference = reference.decode().map_err(|error| error.to_string())?;
                let value = if let Some(number) = reference.strip_prefix("#x") {
                    u32::from_str_radix(number, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .map(|value| value.to_string())
                } else if let Some(number) = reference.strip_prefix('#') {
                    number
                        .parse::<u32>()
                        .ok()
                        .and_then(char::from_u32)
                        .map(|value| value.to_string())
                } else {
                    resolve_predefined_entity(&reference).map(ToOwned::to_owned)
                }
                .ok_or_else(|| format!("XML contains unknown entity &{reference};"))?;
                let parent = stack
                    .last_mut()
                    .ok_or("XML entity appears outside the root element")?;
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
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) | Event::DocType(_) => {}
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

    fn select(&self, path: &str) -> Result<Vec<XmlSelection>, String> {
        let path = path.trim();
        if path.is_empty() || path == "." {
            return Ok(vec![XmlSelection {
                element_path: Vec::new(),
                attribute: None,
            }]);
        }
        if path.contains(['|', ':']) {
            return Err(
                "native.xpath.unsupported: namespace and union expressions are unsupported".into(),
            );
        }
        let absolute = path.starts_with('/') && !path.starts_with("//");
        let marked = path.replace("//", "/__DESCENDANT__/");
        let mut descendant = path.starts_with("//");
        let mut steps = Vec::new();
        for part in marked
            .trim_start_matches("./")
            .split('/')
            .filter(|part| !part.is_empty())
        {
            if part == "__DESCENDANT__" {
                descendant = true;
                continue;
            }
            steps.push((descendant, parse_xpath_step(part)?));
            descendant = false;
        }
        if steps.is_empty() {
            return Ok(Vec::new());
        }

        let mut current = vec![Vec::<usize>::new()];
        let mut offset = 0;
        if absolute
            && matches!(&steps[0].1.test, XPathTest::Element(name) if name == "*" || name == &self.root.name)
            && !steps[0].0
        {
            if !predicate_matches(&self.root, steps[0].1.predicate.as_ref()) {
                return Ok(Vec::new());
            }
            offset = 1;
        }
        for (descendant, step) in &steps[offset..] {
            if let XPathTest::Attribute(name) = &step.test {
                if *descendant || step.predicate.is_some() {
                    return Err(
                        "native.xpath.unsupported: attribute axes cannot have predicates".into(),
                    );
                }
                let mut output = Vec::new();
                for path in current {
                    let element = self.element(&path)?;
                    for (index, (candidate, _)) in element.attributes.iter().enumerate() {
                        if name == "*" || candidate == name {
                            output.push(XmlSelection {
                                element_path: path.clone(),
                                attribute: Some(index),
                            });
                        }
                    }
                }
                return Ok(output);
            }
            let XPathTest::Element(name) = &step.test else {
                unreachable!()
            };
            let mut next = Vec::new();
            for path in current {
                let mut candidates = Vec::new();
                if *descendant {
                    self.descendant_paths(&path, name, &mut candidates)?;
                } else {
                    let element = self.element(&path)?;
                    for (index, child) in element.children.iter().enumerate() {
                        if let XmlChild::Element(child) = child
                            && (name == "*" || child.name == *name)
                        {
                            let mut child_path = path.clone();
                            child_path.push(index);
                            candidates.push(child_path);
                        }
                    }
                }
                apply_xpath_predicate(self, &mut candidates, step.predicate.as_ref());
                next.extend(candidates);
            }
            current = next;
        }
        Ok(current
            .into_iter()
            .map(|element_path| XmlSelection {
                element_path,
                attribute: None,
            })
            .collect())
    }

    fn selection_value(&self, selection: &XmlSelection, style: i64) -> String {
        let Ok(element) = self.element(&selection.element_path) else {
            return String::new();
        };
        if let Some(attribute) = selection.attribute {
            let Some((name, value)) = element.attributes.get(attribute) else {
                return String::new();
            };
            return match style {
                3 => format!("{name}=\"{}\"", xml_attribute_escape(value)),
                4 => name.clone(),
                _ => value.clone(),
            };
        }
        match style {
            1 => element.inner_text(),
            2 => element.inner_xml(),
            3 => element.outer_xml(),
            4 => element.name.clone(),
            _ => String::new(),
        }
    }

    fn element(&self, path: &[usize]) -> Result<&XmlElement, String> {
        let mut element = &self.root;
        for index in path {
            element = match element.children.get(*index) {
                Some(XmlChild::Element(child)) => child,
                _ => return Err("XML selection path became invalid".into()),
            };
        }
        Ok(element)
    }

    fn element_mut(&mut self, path: &[usize]) -> Result<&mut XmlElement, String> {
        let mut element = &mut self.root;
        for index in path {
            element = match element.children.get_mut(*index) {
                Some(XmlChild::Element(child)) => child,
                _ => return Err("XML selection path became invalid".into()),
            };
        }
        Ok(element)
    }

    fn descendant_paths(
        &self,
        start: &[usize],
        name: &str,
        output: &mut Vec<Vec<usize>>,
    ) -> Result<(), String> {
        let element = self.element(start)?;
        if (name == "*" || element.name == name) && !start.is_empty() {
            output.push(start.to_vec());
        }
        for (index, child) in element.children.iter().enumerate() {
            if matches!(child, XmlChild::Element(_)) {
                let mut path = start.to_vec();
                path.push(index);
                self.descendant_paths(&path, name, output)?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn apply_mutation(
        &mut self,
        mutation: XmlMutation,
        request: &NativeCallRequest,
        selections: &[XmlSelection],
    ) -> Result<bool, String> {
        let mut applied = true;
        match mutation {
            XmlMutation::Set => {
                let value = string_argument(request, 2)?.to_owned();
                let style = optional_integer(request, 4)
                    .filter(|value| (0..=2).contains(value))
                    .unwrap_or(0);
                for selection in selections {
                    let element = self.element_mut(&selection.element_path)?;
                    if let Some(attribute) = selection.attribute {
                        if let Some((_, target)) = element.attributes.get_mut(attribute) {
                            target.clone_from(&value);
                        }
                    } else if style == 1 {
                        element.children = vec![XmlChild::Text(value.clone())];
                    } else if style == 2 {
                        element.children = parse_xml_fragment(&value)?;
                    } else {
                        // XmlElement.Value cannot be assigned in System.Xml.
                        return Err("XML_SET style 0 requires an attribute or text node".into());
                    }
                }
            }
            XmlMutation::AddNode => {
                let child = parse_xml(string_argument(request, 2)?)?.root;
                let method = optional_integer(request, 3)
                    .filter(|value| (0..=2).contains(value))
                    .unwrap_or(0);
                for selection in sorted_selections(selections, method != 0) {
                    if selection.attribute.is_some() {
                        applied = false;
                    } else if method == 0 {
                        self.element_mut(&selection.element_path)?
                            .children
                            .push(XmlChild::Element(child.clone()));
                    } else {
                        applied &= insert_sibling(
                            self,
                            &selection.element_path,
                            child.clone(),
                            method == 2,
                        )?;
                    }
                }
            }
            XmlMutation::AddAttribute => {
                let name = string_argument(request, 2)?.to_owned();
                if name.is_empty() || name.contains(['<', '>', '=', '/', ':']) {
                    return Err("XML attribute name is invalid".into());
                }
                let value = optional_string(request, 3).unwrap_or_default().to_owned();
                let method = optional_integer(request, 4)
                    .filter(|value| (0..=2).contains(value))
                    .unwrap_or(0);
                for selection in selections {
                    let element = self.element_mut(&selection.element_path)?;
                    if method == 0 {
                        if selection.attribute.is_none() {
                            element.attributes.push((name.clone(), value.clone()));
                        } else {
                            applied = false;
                        }
                    } else {
                        let Some(index) = selection.attribute else {
                            applied = false;
                            continue;
                        };
                        let insert = index + usize::from(method == 2);
                        element
                            .attributes
                            .insert(insert, (name.clone(), value.clone()));
                    }
                }
            }
            XmlMutation::RemoveNode => {
                for selection in sorted_selections(selections, true) {
                    if selection.attribute.is_some() {
                        applied = false;
                    } else {
                        applied &= remove_element(self, &selection.element_path)?;
                    }
                }
            }
            XmlMutation::RemoveAttribute => {
                let mut selections = selections.to_vec();
                selections.sort_by(|left, right| {
                    right
                        .element_path
                        .cmp(&left.element_path)
                        .then_with(|| right.attribute.cmp(&left.attribute))
                });
                for selection in selections {
                    if let Some(index) = selection.attribute {
                        let element = self.element_mut(&selection.element_path)?;
                        if index < element.attributes.len() {
                            element.attributes.remove(index);
                        }
                    } else {
                        applied = false;
                    }
                }
            }
            XmlMutation::Replace => {
                let replacement = parse_xml(string_argument(request, 2)?)?.root;
                for selection in sorted_selections(selections, true) {
                    if selection.attribute.is_some() {
                        applied = false;
                    } else {
                        applied &=
                            replace_element(self, &selection.element_path, replacement.clone())?;
                    }
                }
            }
        }
        Ok(applied)
    }
}

#[derive(Clone, Debug)]
struct XPathStep {
    test: XPathTest,
    predicate: Option<XPathPredicate>,
}

#[derive(Clone, Debug)]
enum XPathTest {
    Element(String),
    Attribute(String),
}

#[derive(Clone, Debug)]
enum XPathPredicate {
    Position(usize),
    Last,
    AttributeExists(String),
    AttributeEquals(String, String),
    TextEquals(String),
    ChildEquals(String, String),
}

fn parse_xpath_step(value: &str) -> Result<XPathStep, String> {
    let (test, predicate) = if let Some(open) = value.find('[') {
        if !value.ends_with(']') {
            return Err("native.xpath.unsupported: malformed predicate".into());
        }
        (&value[..open], Some(&value[open + 1..value.len() - 1]))
    } else {
        (value, None)
    };
    if test.is_empty() || test.contains(['(', ')']) {
        return Err("native.xpath.unsupported: unsupported node test".into());
    }
    let test = test.strip_prefix('@').map_or_else(
        || XPathTest::Element(test.to_owned()),
        |name| XPathTest::Attribute(name.to_owned()),
    );
    let predicate = predicate.map(parse_xpath_predicate).transpose()?;
    Ok(XPathStep { test, predicate })
}

fn parse_xpath_predicate(value: &str) -> Result<XPathPredicate, String> {
    let value = value.trim();
    if value == "last()" {
        return Ok(XPathPredicate::Last);
    }
    if let Ok(position) = value.parse::<usize>() {
        return Ok(XPathPredicate::Position(position));
    }
    if let Some(attribute) = value.strip_prefix('@') {
        if let Some((name, literal)) = attribute.split_once('=') {
            return Ok(XPathPredicate::AttributeEquals(
                name.trim().to_owned(),
                xpath_literal(literal)?,
            ));
        }
        return Ok(XPathPredicate::AttributeExists(attribute.trim().to_owned()));
    }
    if let Some(literal) = value.strip_prefix("text()=") {
        return Ok(XPathPredicate::TextEquals(xpath_literal(literal)?));
    }
    if let Some((child, literal)) = value.split_once('=')
        && !child.trim().is_empty()
    {
        return Ok(XPathPredicate::ChildEquals(
            child.trim().to_owned(),
            xpath_literal(literal)?,
        ));
    }
    Err("native.xpath.unsupported: predicate is outside the fixed XPath subset".into())
}

fn xpath_literal(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('\'') && value.ends_with('\''))
            || (value.starts_with('"') && value.ends_with('"')))
    {
        Ok(value[1..value.len() - 1].to_owned())
    } else {
        Err("native.xpath.unsupported: predicate literal must be quoted".into())
    }
}

fn apply_xpath_predicate(
    document: &XmlDocument,
    candidates: &mut Vec<Vec<usize>>,
    predicate: Option<&XPathPredicate>,
) {
    match predicate {
        None => {}
        Some(XPathPredicate::Position(position)) => {
            let selected = position
                .checked_sub(1)
                .and_then(|index| candidates.get(index))
                .cloned();
            candidates.clear();
            candidates.extend(selected);
        }
        Some(XPathPredicate::Last) => {
            let selected = candidates.last().cloned();
            candidates.clear();
            candidates.extend(selected);
        }
        Some(predicate) => candidates.retain(|path| {
            document
                .element(path)
                .is_ok_and(|element| predicate_matches(element, Some(predicate)))
        }),
    }
}

fn predicate_matches(element: &XmlElement, predicate: Option<&XPathPredicate>) -> bool {
    match predicate {
        None | Some(XPathPredicate::Position(1) | XPathPredicate::Last) => true,
        Some(XPathPredicate::Position(_)) => false,
        Some(XPathPredicate::AttributeExists(name)) => element
            .attributes
            .iter()
            .any(|(candidate, _)| candidate == name),
        Some(XPathPredicate::AttributeEquals(name, value)) => element
            .attributes
            .iter()
            .any(|(candidate, candidate_value)| candidate == name && candidate_value == value),
        Some(XPathPredicate::TextEquals(value)) => element.inner_text() == *value,
        Some(XPathPredicate::ChildEquals(name, value)) => element
            .elements_named(name)
            .iter()
            .any(|child| child.inner_text() == *value),
    }
}

fn sorted_selections(selections: &[XmlSelection], reverse: bool) -> Vec<XmlSelection> {
    let mut result = selections.to_vec();
    if reverse {
        result.sort_by(|left, right| right.element_path.cmp(&left.element_path));
    }
    result
}

fn insert_sibling(
    document: &mut XmlDocument,
    path: &[usize],
    child: XmlElement,
    after: bool,
) -> Result<bool, String> {
    let Some((index, parent)) = path.split_last() else {
        return Ok(false);
    };
    let parent = document.element_mut(parent)?;
    parent
        .children
        .insert(*index + usize::from(after), XmlChild::Element(child));
    Ok(true)
}

fn remove_element(document: &mut XmlDocument, path: &[usize]) -> Result<bool, String> {
    let Some((index, parent)) = path.split_last() else {
        return Ok(false);
    };
    let parent = document.element_mut(parent)?;
    if *index < parent.children.len() {
        parent.children.remove(*index);
    }
    Ok(true)
}

fn replace_element(
    document: &mut XmlDocument,
    path: &[usize],
    replacement: XmlElement,
) -> Result<bool, String> {
    let Some((index, parent)) = path.split_last() else {
        return Ok(false);
    };
    let parent = document.element_mut(parent)?;
    let Some(slot) = parent.children.get_mut(*index) else {
        return Err("XML replacement path became invalid".into());
    };
    *slot = XmlChild::Element(replacement);
    Ok(true)
}

fn parse_xml_fragment(value: &str) -> Result<Vec<XmlChild>, String> {
    Ok(parse_xml(&format!(
        "<__rustyera_fragment>{value}</__rustyera_fragment>"
    ))?
    .root
    .children)
}

impl XmlElement {
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find_map(|(candidate, value)| (candidate == name).then_some(value.as_str()))
    }

    fn elements_named(&self, name: &str) -> Vec<&Self> {
        self.children
            .iter()
            .filter_map(|child| match child {
                XmlChild::Element(element) if name == "*" || element.name == name => Some(element),
                XmlChild::Element(_) | XmlChild::Text(_) => None,
            })
            .collect()
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
                XmlChild::Text(value) => output.push_str(&xml_text_escape(value)),
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
            output.push_str(&xml_attribute_escape(value));
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

fn xml_text_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn xml_attribute_escape(value: &str) -> String {
    xml_text_escape(value).replace('"', "&quot;")
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
        let selection = &document.select("/root/p/k").unwrap()[0];
        assert_eq!(document.selection_value(selection, 1), "one");
        assert_eq!(parse_xml(&document.outer_xml()).unwrap(), document);
    }

    #[test]
    fn xpath_subset_handles_descendants_attributes_and_predicates() {
        let document = parse_xml(
            "<root><p id='a'><k>one</k></p><group><p id='b'><k>two</k></p></group></root>",
        )
        .unwrap();
        let selected = document.select("//p[@id='b']/k").unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(document.selection_value(&selected[0], 1), "two");
        let attributes = document.select("//p/@id").unwrap();
        assert_eq!(
            attributes
                .iter()
                .map(|selection| document.selection_value(selection, 0))
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(document.select("//p[contains(k, 'o')]").is_err());
    }

    #[test]
    fn deterministic_table_ids_are_monotonic() {
        let table = DataTable::new();
        assert_eq!(table.next_id, 1);
        assert_eq!(table.columns[0].name, "id");
    }

    #[test]
    fn data_table_xml_matches_reference_dataset_shape_and_round_trips() {
        let mut table = DataTable::new();
        table.columns.extend([
            Column {
                name: "name".into(),
                value_type: DataType::String,
                nullable: true,
            },
            Column {
                name: "score".into(),
                value_type: DataType::Int32,
                nullable: false,
            },
        ]);
        table.rows.push(DataRow {
            id: 1,
            cells: vec![Cell::Null, Cell::String("A&B".into()), Cell::Integer(7)],
        });
        let schema = data_table_schema_xml("table", &table);
        assert_eq!(
            schema,
            concat!(
                "<?xml version=\"1.0\" encoding=\"utf-16\"?>\r\n",
                "<xs:schema id=\"NewDataSet\" xmlns=\"\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:msdata=\"urn:schemas-microsoft-com:xml-msdata\">\r\n",
                "  <xs:element name=\"NewDataSet\" msdata:IsDataSet=\"true\" msdata:MainDataTable=\"table\" msdata:CaseSensitive=\"true\" msdata:UseCurrentLocale=\"true\">\r\n",
                "    <xs:complexType>\r\n",
                "      <xs:choice minOccurs=\"0\" maxOccurs=\"unbounded\">\r\n",
                "        <xs:element name=\"table\" msdata:CaseSensitive=\"True\">\r\n",
                "          <xs:complexType>\r\n",
                "            <xs:sequence>\r\n",
                "              <xs:element name=\"id\" type=\"xs:long\" />\r\n",
                "              <xs:element name=\"name\" type=\"xs:string\" minOccurs=\"0\" />\r\n",
                "              <xs:element name=\"score\" type=\"xs:int\" />\r\n",
                "            </xs:sequence>\r\n",
                "          </xs:complexType>\r\n",
                "        </xs:element>\r\n",
                "      </xs:choice>\r\n",
                "    </xs:complexType>\r\n",
                "    <xs:unique name=\"Constraint1\" msdata:PrimaryKey=\"true\">\r\n",
                "      <xs:selector xpath=\".//table\" />\r\n",
                "      <xs:field xpath=\"id\" />\r\n",
                "    </xs:unique>\r\n",
                "  </xs:element>\r\n",
                "</xs:schema>"
            )
        );
        let data = data_table_data_xml("table", &table);
        assert_eq!(
            data,
            "<DocumentElement>\r\n  <table>\r\n    <id>1</id>\r\n    <name>A&amp;B</name>\r\n    <score>7</score>\r\n  </table>\r\n</DocumentElement>"
        );
        let parsed_schema = parse_data_table_schema("table", &schema).unwrap();
        let parsed = parse_data_table_xml("table", &parsed_schema, &data).unwrap();
        assert_eq!(parsed.columns, table.columns);
        assert_eq!(parsed.rows, table.rows);
    }

    #[test]
    fn data_table_xml_rejects_partial_or_mismatched_input_before_commit() {
        let table = DataTable::new();
        let schema = data_table_schema_xml("table", &table);
        let parsed_schema = parse_data_table_schema("table", &schema).unwrap();
        assert!(parse_data_table_schema("other", &schema).is_err());
        assert!(
            parse_data_table_xml(
                "table",
                &parsed_schema,
                "<DocumentElement><table><id>bad</id></table></DocumentElement>"
            )
            .is_err()
        );
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
