use std::collections::BTreeMap;

use erabasic_bytecode::{BytecodeArtifact, BytecodeType, RuntimeImport, SymbolKey};
use serde::{Deserialize, Serialize};

use crate::{FiberId, HostRequestId, HostWrite, VmValue};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostWaitStability {
    StableInput,
    Transient,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostCallRequest {
    pub id: HostRequestId,
    pub fiber: FiberId,
    pub import: RuntimeImport,
    pub arguments: Vec<VmValue>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostReady {
    pub value: Option<VmValue>,
    pub writes: Vec<HostWrite>,
}

impl HostReady {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            value: None,
            writes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostCallResult {
    Ready(HostReady),
    Pending {
        stability: HostWaitStability,
        rebind_payload: Vec<u8>,
    },
    Error(String),
    /// Runtime-port adapter sentinel. Unlike `Pending`, this only unwinds the
    /// interpreter; the runtime must immediately classify the captured request.
    Deferred,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostRebindRequest {
    pub id: HostRequestId,
    pub fiber: FiberId,
    pub import: RuntimeImport,
    pub payload: Vec<u8>,
}

pub trait VmHost {
    fn call(&mut self, request: HostCallRequest) -> HostCallResult;

    /// Implementations must apply this batch atomically. Returning an error means
    /// that no wait was rebound and VM restore remains side-effect free.
    ///
    /// # Errors
    ///
    /// Returns an error when the host cannot recreate every wait atomically.
    fn rebind_snapshot(&mut self, requests: &[HostRebindRequest]) -> Result<(), String> {
        if requests.is_empty() {
            Ok(())
        } else {
            Err("this host does not support snapshot wait rebinding".into())
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeCallRequest {
    pub import: RuntimeImport,
    pub arguments: Vec<VmValue>,
}

pub trait NativeService: Send {
    /// # Errors
    ///
    /// Returns an error when the service rejects the request or cannot produce a result.
    fn call(&mut self, request: NativeCallRequest) -> Result<Option<VmValue>, String>;

    /// `None` marks state that cannot participate in a VM snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when service state cannot be serialized.
    fn snapshot(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(Some(Vec::new()))
    }

    /// # Errors
    ///
    /// Returns an error when the serialized state is invalid for this service.
    fn restore(&mut self, state: &[u8]) -> Result<(), String> {
        if state.is_empty() {
            Ok(())
        } else {
            Err("stateless native service received non-empty state".into())
        }
    }
}

#[derive(Default)]
pub struct NativeServiceRegistry {
    services: BTreeMap<SymbolKey, Box<dyn NativeService>>,
}

impl NativeServiceRegistry {
    /// Register the small VM-native services emitted directly by the compiler.
    /// Project-specific builtins remain explicit services and fail closed when absent.
    #[must_use]
    pub fn for_artifact(artifact: &BytecodeArtifact) -> Self {
        let mut registry = Self::default();
        for native in &artifact.native_imports {
            let name = native.import.name.as_str();
            if matches!(name, "format_integer" | "format_string") || name.starts_with("control_") {
                registry.register(
                    native.import.key,
                    CompilerNative {
                        name: name.into(),
                        result: native.import.result,
                    },
                );
            }
        }
        registry
    }

    pub fn register(&mut self, key: SymbolKey, service: impl NativeService + 'static) -> bool {
        self.services.insert(key, Box::new(service)).is_none()
    }

    pub(crate) fn call(
        &mut self,
        key: SymbolKey,
        request: NativeCallRequest,
    ) -> Result<Option<VmValue>, String> {
        self.services
            .get_mut(&key)
            .ok_or_else(|| format!("native service {key:?} is not registered"))?
            .call(request)
    }

    pub(crate) fn snapshots(&self) -> Result<BTreeMap<SymbolKey, Vec<u8>>, String> {
        self.services
            .iter()
            .map(|(key, service)| {
                service
                    .snapshot()?
                    .map(|state| (*key, state))
                    .ok_or_else(|| format!("native service {key:?} is not snapshot-capable"))
            })
            .collect()
    }

    pub(crate) fn restore_snapshots(
        &mut self,
        states: &BTreeMap<SymbolKey, Vec<u8>>,
    ) -> Result<(), String> {
        let previous = self.snapshots()?;
        for (key, state) in states {
            let outcome = self.services.get_mut(key).map_or_else(
                || Err(format!("native service {key:?} is not registered")),
                |service| service.restore(state),
            );
            if let Err(error) = outcome {
                for (rollback_key, rollback) in &previous {
                    if let Some(service) = self.services.get_mut(rollback_key) {
                        let _ = service.restore(rollback);
                    }
                }
                return Err(error);
            }
        }
        Ok(())
    }
}

struct CompilerNative {
    name: String,
    result: Option<BytecodeType>,
}

impl NativeService for CompilerNative {
    fn call(&mut self, request: NativeCallRequest) -> Result<Option<VmValue>, String> {
        match self.name.as_str() {
            "format_integer" => {
                let Some(VmValue::Integer(value)) = request.arguments.first() else {
                    return Err("format_integer expects an integer".into());
                };
                Ok(Some(VmValue::String(apply_width(
                    &value.to_string(),
                    request.arguments.get(1),
                )?)))
            }
            "format_string" => {
                let Some(value) = request.arguments.first() else {
                    return Err("format_string expects a value".into());
                };
                let value = match value {
                    VmValue::Integer(value) => value.to_string(),
                    VmValue::String(value) => value.clone(),
                    VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => {
                        return Err("format_string cannot dereference a place".into());
                    }
                };
                Ok(Some(VmValue::String(apply_width(
                    &value,
                    request.arguments.get(1),
                )?)))
            }
            name if name.starts_with("control_") => Ok(self.result.map(VmValue::default_for)),
            _ => Err(format!("unknown compiler-native service {}", self.name)),
        }
    }
}

fn apply_width(value: &str, width: Option<&VmValue>) -> Result<String, String> {
    let Some(width) = width else {
        return Ok(value.into());
    };
    let VmValue::Integer(signed_width) = width else {
        return Err("format width must be an integer".into());
    };
    let left_align = *signed_width < 0;
    let width = usize::try_from(signed_width.unsigned_abs())
        .map_err(|_| "format width exceeds this platform".to_owned())?;
    let characters = value.chars().count();
    if characters >= width {
        return Ok(value.into());
    }
    let padding = " ".repeat(width - characters);
    Ok(if left_align {
        format!("{value}{padding}")
    } else {
        format!("{padding}{value}")
    })
}
