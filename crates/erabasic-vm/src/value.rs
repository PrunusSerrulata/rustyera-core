use erabasic_bytecode::{BytecodeType, SymbolKey};
use serde::{Deserialize, Serialize};

use crate::{FiberId, FrameId};

/// Values crossing the VM/runtime boundary are deliberately small and Serde-friendly.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum VmValue {
    Integer(i64),
    String(String),
    IntegerPlace(Box<PlaceDescriptor>),
    StringPlace(Box<PlaceDescriptor>),
}

impl VmValue {
    #[must_use]
    pub const fn value_type(&self) -> BytecodeType {
        match self {
            Self::Integer(_) => BytecodeType::Integer,
            Self::String(_) => BytecodeType::String,
            Self::IntegerPlace(_) => BytecodeType::IntegerPlace,
            Self::StringPlace(_) => BytecodeType::StringPlace,
        }
    }

    #[must_use]
    pub fn default_for(value_type: BytecodeType) -> Self {
        match value_type {
            BytecodeType::Integer => Self::Integer(0),
            BytecodeType::String => Self::String(String::new()),
            BytecodeType::IntegerPlace => Self::IntegerPlace(Box::default()),
            BytecodeType::StringPlace => Self::StringPlace(Box::default()),
        }
    }
}

/// A place is opaque to the host. It can only be returned in a [`HostWrite`](crate::HostWrite),
/// where the VM validates its variable, frame, character and indices again.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlaceDescriptor {
    pub variable: SymbolKey,
    pub indices: Vec<u64>,
    pub character: Option<u64>,
    pub fiber: Option<FiberId>,
    pub frame: Option<FrameId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostWrite {
    pub target: PlaceDescriptor,
    pub value: VmValue,
}
