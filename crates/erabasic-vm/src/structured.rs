use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use erabasic_bytecode::SymbolKey;
use erabasic_data::ExtensionData;
use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};

use crate::{
    ExecutionFailure, FaultCategory, HostWrite, NativeCallRequest, NativePlaceView, NativeReady,
    PlaceDescriptor, ScriptFaultKind, VmFaultCode, VmValue,
};

mod column_identity;
mod column_options;
mod data_table;
mod legacy;
mod map_calls;
mod map_leases;
mod xml;

use data_table::{
    argument_key, array_writes, cell_for_column, data_table_data_xml, data_table_pairs,
    data_table_schema_xml, data_type_code, explicit_place, implicit_place, integer_argument,
    optional_integer, optional_string, parse_data_table_schema, parse_data_table_xml,
    parse_data_type, ready_integer, result_count_write, row_matches, sort_rows, string_argument,
    xml_target_key, xml_target_string,
};
use xml::{parse_xml, xml_attribute_escape, xml_text_escape};

pub(crate) const STRUCTURED_BUNDLE_VERSION: u32 = 4;

pub(crate) fn bundle_key() -> SymbolKey {
    // Provider identity remains stable across compatible payload revisions; the
    // explicit four-byte bundle header rejects older serialized state.
    SymbolKey::derive("rustyera.native.bundle", b"structured-data-v3")
}

pub(crate) fn is_structured_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("map_") || name.starts_with("xml_") || name.starts_with("dt_")
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct StructuredState {
    maps: BTreeMap<String, OrderedMap>,
    map_leases: map_leases::MapLeaseBook,
    xml_documents: BTreeMap<String, XmlDocument>,
    data_tables: BTreeMap<String, DataTable>,
    next_column_identity: u64,
    column_identity_revision: u64,
}

impl Default for StructuredState {
    fn default() -> Self {
        Self {
            maps: BTreeMap::new(),
            map_leases: map_leases::MapLeaseBook::default(),
            xml_documents: BTreeMap::new(),
            data_tables: BTreeMap::new(),
            next_column_identity: 1,
            column_identity_revision: 0,
        }
    }
}

pub(crate) use column_identity::ColumnIdentityStamp;
pub(crate) use column_options::is_internal_column_native;
pub(crate) use map_calls::MapOperation;
pub(crate) use map_leases::{MapLease, MapLeaseOrigin, MapLeaseOwner, MapLeaseStamp};

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
    identity: u64,
    name: String,
    value_type: DataType,
    nullable: bool,
    default_value: Cell,
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
                identity: 0,
                name: "id".into(),
                value_type: DataType::Int64,
                nullable: false,
                default_value: Cell::Null,
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

    fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, crate::ExecutionFailure> {
        let mut state = self.state.lock().map_err(|_| {
            crate::host::native_contract_failure("structured native state lock is poisoned")
        })?;
        state.call(&self.name, &request).map_err(|mut failure| {
            // All structured native errors historically used Native. Keep that legacy code
            // without deriving or changing source-assigned catch permission.
            failure.code = VmFaultCode::Native;
            failure
        })
    }

    fn requires_rollback_checkpoint(&self) -> bool {
        false
    }

    // The registry serializes the shared bundle once under a stable bundle key.
    fn snapshot(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(Some(Vec::new()))
    }
}

impl StructuredState {
    pub(crate) fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate_identity_state()
            .map_err(|failure| failure.to_string())?;
        self.validate_map_leases()
            .map_err(|failure| failure.to_string())?;
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
        let state: Self = serde_json::from_slice(&bytes[4..]).map_err(|error| error.to_string())?;
        state
            .validate_identity_state()
            .map_err(|failure| failure.to_string())?;
        state
            .validate_map_leases()
            .map_err(|failure| failure.to_string())?;
        Ok(state)
    }

    pub(crate) fn clear_for_transaction(
        &mut self,
        extensions: &ExtensionData,
        transaction: &crate::VmRuntimeStateTransaction,
    ) -> Result<(), ExecutionFailure> {
        match transaction {
            crate::VmRuntimeStateTransaction::ResetNewGame
            | crate::VmRuntimeStateTransaction::ResetGameData
            | crate::VmRuntimeStateTransaction::RestoreOrdinary(_)
            | crate::VmRuntimeStateTransaction::RestoreOrdinaryWithLastLoad { .. } => {
                self.clear_declared(
                    &extensions.save_maps,
                    &extensions.save_xmls,
                    &extensions.save_data_tables,
                )?;
            }
            crate::VmRuntimeStateTransaction::ResetGlobalData => {
                self.clear_declared(
                    &extensions.global_maps,
                    &extensions.global_xmls,
                    &extensions.global_data_tables,
                )?;
                self.clear_declared(
                    &extensions.static_maps,
                    &extensions.static_xmls,
                    &extensions.static_data_tables,
                )?;
            }
            crate::VmRuntimeStateTransaction::OverlayGlobal(_) => {
                self.clear_declared(
                    &extensions.global_maps,
                    &extensions.global_xmls,
                    &extensions.global_data_tables,
                )?;
            }
            crate::VmRuntimeStateTransaction::AppendCharacters(_)
            | crate::VmRuntimeStateTransaction::SetLastLoad { .. }
            | crate::VmRuntimeStateTransaction::Mutate { .. } => {}
        }
        Ok(())
    }

    fn clear_declared(
        &mut self,
        maps: &BTreeSet<String>,
        xmls: &BTreeSet<String>,
        tables: &BTreeSet<String>,
    ) -> Result<(), ExecutionFailure> {
        for key in maps {
            if self.maps.contains_key(key) {
                self.replace_map_binding(key, OrderedMap::default())?;
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
        Ok(())
    }

    pub(crate) fn export_extensions(
        &self,
        declarations: &ExtensionData,
        scope: StructuredScope,
    ) -> Vec<StructuredExtension> {
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
                    schema: data_table_schema_xml(key, table),
                    data: data_table_data_xml(key, table),
                });
            }
        }
        output
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
                    self.replace_map_binding(key, map)
                        .map_err(|failure| failure.to_string())?;
                    imported.insert((0x20, key.clone()));
                }
                StructuredExtension::Xml { key, document } if xmls.contains(key) => {
                    self.xml_documents.insert(
                        key.clone(),
                        parse_xml(document).map_err(|failure| failure.to_string())?,
                    );
                    imported.insert((0x21, key.clone()));
                }
                StructuredExtension::DataTable { key, schema, data } if tables.contains(key) => {
                    let table = decode_data_table_extension(key, schema, data)?;
                    self.install_fresh_table(key.clone(), table)
                        .map_err(|failure| failure.to_string())?;
                    imported.insert((0x22, key.clone()));
                }
                _ => {}
            }
        }
        Ok(imported)
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn call(
        &mut self,
        name: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, ExecutionFailure> {
        let name = name.to_ascii_lowercase();
        if name.starts_with("map_") {
            return self.call_map(&name, request);
        }
        if name.starts_with("xml_") {
            return self.call_xml(&name, request);
        }
        if name.starts_with("dt__column_") {
            return self.call_column_option(&name, request);
        }
        if name.starts_with("dt_") {
            return self.call_data_table(&name, request);
        }
        Err(contract_failure(format!(
            "unknown structured native service {name}"
        )))
    }

    #[allow(clippy::too_many_lines)]
    fn call_map(
        &mut self,
        name: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, ExecutionFailure> {
        if matches!(
            name,
            "map_create"
                | "map_release"
                | "map_set"
                | "map_remove"
                | "map_clear"
                | "map_fromxml"
                | "map_merge"
        ) {
            self.bump_map_revision()?;
        }
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
                self.retire_map_binding(&map_name);
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
            "map_merge" => self.merge_maps(request),
            name if MapOperation::from_name(name).is_some() => Err(contract_failure(
                "MAP extension requires staged capture; eager dispatch is invalid",
            )),
            _ => Err(contract_failure(format!("unsupported map native {name}"))),
        }
    }
}

fn decode_data_table_extension(key: &str, schema: &str, data: &str) -> Result<DataTable, String> {
    let mut table = if schema.trim_start().starts_with('<') {
        let schema = parse_data_table_schema(key, schema).map_err(|failure| failure.to_string())?;
        parse_data_table_xml(key, &schema, data).map_err(|failure| failure.to_string())?
    } else {
        // RustyEra briefly wrote its internal serde representation before the
        // reference-compatible DataSet XML boundary was implemented. Keep those
        // saves loadable, but never emit this legacy representation again.
        legacy::decode_table(key, schema, data)?
    };
    table.next_id = table
        .rows
        .iter()
        .map(|row| row.id)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| format!("DataTable extension {key} row id overflowed"))?;
    Ok(table)
}

mod data_calls;
mod xml_calls;

#[cfg(test)]
mod tests;

fn contract_failure(message: impl Into<String>) -> ExecutionFailure {
    ExecutionFailure::classified(FaultCategory::HostContract, VmFaultCode::Native, message)
}

fn parse_failure(message: impl Into<String>) -> ExecutionFailure {
    ExecutionFailure::script(ScriptFaultKind::Parse, VmFaultCode::Native, message)
}

fn argument_failure(message: impl Into<String>) -> ExecutionFailure {
    ExecutionFailure::script(ScriptFaultKind::Argument, VmFaultCode::Native, message)
}

fn resource_failure(message: impl Into<String>) -> ExecutionFailure {
    ExecutionFailure::classified(FaultCategory::ResourceLimit, VmFaultCode::Native, message)
}
