use serde::{Deserialize, Serialize};

use crate::{BytecodeType, SymbolKey};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCapability {
    Text,
    Graphics,
    Audio,
    Input,
    Clock,
    Storage,
    Extension,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct HostEffect {
    pub pure: bool,
    pub may_suspend: bool,
    pub may_error: bool,
    pub mutates_runtime: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeImport {
    pub key: SymbolKey,
    pub namespace: String,
    pub name: String,
    pub abi_version: u32,
    pub parameters: Vec<BytecodeType>,
    pub result: Option<BytecodeType>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeImportKind {
    Native,
    Host,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeImport {
    pub import: RuntimeImport,
    pub effect: HostEffect,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostImport {
    pub import: RuntimeImport,
    pub effect: HostEffect,
    pub capability: HostCapability,
    pub snapshot_safe: bool,
}
