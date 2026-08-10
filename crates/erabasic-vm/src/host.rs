use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use erabasic_bytecode::{BytecodeArtifact, RuntimeImport, SymbolKey};
use erabasic_data::LegacyEncoding;
use serde::{Deserialize, Serialize};

use crate::sfmt::Sfmt19937;
use crate::structured::{StructuredExtension, StructuredScope};
use crate::structured::{StructuredNative, StructuredState, bundle_key, is_structured_name};
use crate::{FiberId, HostRequestId, HostWrite, PlaceDescriptor, VmExecutionOrigin, VmValue};

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
    pub origin: VmExecutionOrigin,
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
    /// Immutable snapshots of place arguments. Native services cannot dereference
    /// VM places themselves; the interpreter resolves every view before calling
    /// the service and validates returned writes again before committing them.
    pub places: Vec<NativePlaceView>,
    /// Reference pseudo-variables used by legacy multi-result functions.
    pub implicit_places: BTreeMap<String, NativePlaceView>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativePlaceView {
    pub argument_index: usize,
    pub target: PlaceDescriptor,
    pub values: Vec<VmValue>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NativeReady {
    pub value: Option<VmValue>,
    pub writes: Vec<HostWrite>,
}

impl NativeReady {
    #[must_use]
    pub const fn value(value: VmValue) -> Self {
        Self {
            value: Some(value),
            writes: Vec::new(),
        }
    }
}

pub trait NativeService: Send {
    /// Names of legacy pseudo-variable arrays required to evaluate this service.
    /// Most services do not use them, so avoiding unconditional snapshots keeps
    /// ordinary scalar natives independent of large RESULT-family arrays.
    fn implicit_place_names(&self) -> &'static [&'static str] {
        &[]
    }

    /// # Errors
    ///
    /// Returns an error when the service rejects the request or cannot produce a result.
    fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, String>;

    /// Whether the interpreter must snapshot this service before invoking it.
    /// Services may opt out only when calls validate all fallible work before
    /// committing state and never combine state mutation with VM-place writes.
    fn requires_rollback_checkpoint(&self) -> bool {
        true
    }

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

#[derive(Clone)]
struct CharacterWidthModeHandle(Arc<AtomicU8>);

impl Default for CharacterWidthModeHandle {
    fn default() -> Self {
        Self(Arc::new(AtomicU8::new(0)))
    }
}

impl CharacterWidthModeHandle {
    fn get(&self) -> crate::CharacterWidthMode {
        match self.0.load(Ordering::Relaxed) {
            1 => crate::CharacterWidthMode::AmbiguousNarrow,
            2 => crate::CharacterWidthMode::AmbiguousWide,
            _ => crate::CharacterWidthMode::Automatic,
        }
    }

    fn set(&self, mode: crate::CharacterWidthMode) {
        let value = match mode {
            crate::CharacterWidthMode::Automatic => 0,
            crate::CharacterWidthMode::AmbiguousNarrow => 1,
            crate::CharacterWidthMode::AmbiguousWide => 2,
        };
        self.0.store(value, Ordering::Relaxed);
    }
}

#[derive(Default)]
pub struct NativeServiceRegistry {
    services: BTreeMap<SymbolKey, Box<dyn NativeService>>,
    random: Option<Arc<Mutex<Sfmt19937>>>,
    structured: Option<Arc<Mutex<StructuredState>>>,
    structured_keys: BTreeSet<SymbolKey>,
    extensions: erabasic_data::ExtensionData,
    character_width_mode: CharacterWidthModeHandle,
}

type PreparedStructuredImport = (Option<Vec<u8>>, BTreeSet<(u8, String)>);

impl NativeServiceRegistry {
    pub(crate) fn fork_for_artifact(&self, artifact: &BytecodeArtifact) -> Result<Self, String> {
        let snapshots = self.snapshots()?;
        let mut fork = Self::for_artifact(artifact);
        fork.restore_snapshots(&snapshots)?;
        fork.set_character_width_mode(self.character_width_mode.get());
        Ok(fork)
    }

    /// Register the small VM-native services emitted directly by the compiler.
    /// Project-specific builtins remain explicit services and fail closed when absent.
    #[must_use]
    pub fn for_artifact(artifact: &BytecodeArtifact) -> Self {
        Self::for_artifact_with_seed(artifact, 0)
    }

    #[must_use]
    pub fn for_artifact_with_seed(artifact: &BytecodeArtifact, seed: u64) -> Self {
        let mut registry = Self {
            extensions: artifact.project_data.static_data.extensions.clone(),
            ..Self::default()
        };
        let random = Arc::new(Mutex::new(Sfmt19937::new(seed)));
        let structured = Arc::new(Mutex::new(StructuredState::default()));
        registry.random = Some(Arc::clone(&random));
        for native in &artifact.native_imports {
            let name = native.import.name.as_str();
            if is_structured_name(name) {
                registry.structured = Some(Arc::clone(&structured));
                registry.structured_keys.insert(native.import.key);
                registry.register(
                    native.import.key,
                    StructuredNative::new(name, Arc::clone(&structured)),
                );
            } else if matches!(name, "format_integer" | "format_string" | "times")
                || name.starts_with("control_")
            {
                registry.register(
                    native.import.key,
                    CompilerNative {
                        name: name.into(),
                        character_width_mode: registry.character_width_mode.clone(),
                    },
                );
            } else if matches!(name, "rand" | "randomize" | "initrand" | "dumprand") {
                registry.register(
                    native.import.key,
                    RandomNative {
                        name: name.into(),
                        state: Arc::clone(&random),
                    },
                );
            } else if matches!(
                name,
                "abs"
                    | "sign"
                    | "sqrt"
                    | "cbrt"
                    | "log"
                    | "log10"
                    | "exponent"
                    | "power"
                    | "getbit"
                    | "bitcount"
                    | "strlen"
                    | "strlenu"
                    | "strform"
                    | "toint"
                    | "isnumeric"
                    | "unicode"
                    | "convert"
                    | "color_fromrgb"
                    | "color_fromname"
                    | "max"
                    | "min"
                    | "limit"
                    | "inrange"
                    | "tostr"
                    | "substring"
                    | "substringu"
                    | "strfind"
                    | "strfindu"
                    | "strcount"
                    | "strlens"
                    | "strlensu"
                    | "getpalamlv"
                    | "getexplv"
                    | "replace"
                    | "escape"
                    | "unicodetostr"
                    | "encodetouni"
                    | "unicodebyte"
                    | "charatu"
                    | "tolower"
                    | "toupper"
            ) {
                registry.register(
                    native.import.key,
                    CoreNative {
                        name: name.into(),
                        legacy_encoding: artifact.project_data.static_data.legacy_encoding,
                    },
                );
            }
        }
        registry
    }

    pub fn register(&mut self, key: SymbolKey, service: impl NativeService + 'static) -> bool {
        self.services.insert(key, Box::new(service)).is_none()
    }

    pub(crate) fn set_character_width_mode(&mut self, mode: crate::CharacterWidthMode) {
        self.character_width_mode.set(mode);
    }

    pub(crate) fn character_width_mode(&self) -> crate::CharacterWidthMode {
        self.character_width_mode.get()
    }

    pub(crate) fn call(
        &mut self,
        key: SymbolKey,
        request: NativeCallRequest,
    ) -> Result<NativeReady, String> {
        self.services
            .get_mut(&key)
            .ok_or_else(|| format!("native service {key:?} is not registered"))?
            .call(request)
    }

    pub(crate) fn implicit_place_names(
        &self,
        key: SymbolKey,
    ) -> Result<&'static [&'static str], String> {
        self.services
            .get(&key)
            .map(|service| service.implicit_place_names())
            .ok_or_else(|| format!("native service {key:?} is not registered"))
    }

    pub(crate) fn checkpoint(&self, key: SymbolKey) -> Result<Option<Vec<u8>>, String> {
        let service = self
            .services
            .get(&key)
            .ok_or_else(|| format!("native service {key:?} is not registered"))?;
        if service.requires_rollback_checkpoint() {
            service.snapshot()
        } else {
            Ok(None)
        }
    }

    pub(crate) fn rollback(&mut self, key: SymbolKey, state: &[u8]) -> Result<(), String> {
        self.services
            .get_mut(&key)
            .ok_or_else(|| format!("native service {key:?} is not registered"))?
            .restore(state)
    }

    pub(crate) fn prepare_structured_transaction(
        &self,
        transaction: &crate::VmRuntimeStateTransaction,
    ) -> Result<Option<Vec<u8>>, String> {
        let Some(structured) = &self.structured else {
            return Ok(None);
        };
        let mut candidate = structured
            .lock()
            .map_err(|_| "structured native state lock is poisoned".to_owned())?
            .clone();
        candidate.clear_for_transaction(&self.extensions, transaction);
        candidate.encode().map(Some)
    }

    pub(crate) fn prepare_structured_import(
        &self,
        transaction: &crate::VmRuntimeStateTransaction,
        scope: StructuredScope,
        values: &[StructuredExtension],
    ) -> Result<PreparedStructuredImport, String> {
        let Some(structured) = &self.structured else {
            return Ok((None, BTreeSet::new()));
        };
        let mut candidate = structured
            .lock()
            .map_err(|_| "structured native state lock is poisoned".to_owned())?
            .clone();
        candidate.clear_for_transaction(&self.extensions, transaction);
        let imported = candidate.import_extensions(&self.extensions, scope, values)?;
        Ok((Some(candidate.encode()?), imported))
    }

    pub(crate) fn structured_extensions(
        &self,
        scope: StructuredScope,
    ) -> Result<Vec<StructuredExtension>, String> {
        self.structured.as_ref().map_or_else(
            || Ok(Vec::new()),
            |structured| {
                structured
                    .lock()
                    .map_err(|_| "structured native state lock is poisoned".to_owned())?
                    .export_extensions(&self.extensions, scope)
            },
        )
    }

    pub(crate) fn commit_structured_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        let decoded = StructuredState::decode(bytes)?;
        *self
            .structured
            .as_ref()
            .ok_or_else(|| "structured native bundle is not registered".to_owned())?
            .lock()
            .map_err(|_| "structured native state lock is poisoned".to_owned())? = decoded;
        Ok(())
    }

    pub(crate) fn random_values(&self) -> Result<Vec<i64>, String> {
        self.random
            .as_ref()
            .ok_or_else(|| "random native service is not registered".to_owned())?
            .lock()
            .map(|state| state.era_values())
            .map_err(|_| "SFMT state lock is poisoned".into())
    }

    pub(crate) fn set_random_values(&mut self, values: &[i64]) -> Result<(), String> {
        let candidate = Sfmt19937::from_era_values(values)?;
        let mut state = self
            .random
            .as_ref()
            .ok_or_else(|| "random native service is not registered".to_owned())?
            .lock()
            .map_err(|_| "SFMT state lock is poisoned".to_owned())?;
        *state = candidate;
        Ok(())
    }

    pub(crate) fn snapshots(&self) -> Result<BTreeMap<SymbolKey, Vec<u8>>, String> {
        let mut snapshots = self
            .services
            .iter()
            .map(|(key, service)| {
                service
                    .snapshot()?
                    .map(|state| (*key, state))
                    .ok_or_else(|| format!("native service {key:?} is not snapshot-capable"))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        if let Some(structured) = &self.structured {
            snapshots.insert(
                bundle_key(),
                structured
                    .lock()
                    .map_err(|_| "structured native state lock is poisoned".to_owned())?
                    .encode()?,
            );
        }
        Ok(snapshots)
    }

    pub(crate) fn restore_snapshots(
        &mut self,
        states: &BTreeMap<SymbolKey, Vec<u8>>,
    ) -> Result<(), String> {
        let previous = self.snapshots()?;
        for (key, state) in states {
            let outcome = if *key == bundle_key() {
                self.structured.as_ref().map_or_else(
                    || Err("structured native bundle is not registered".into()),
                    |structured| {
                        let decoded = StructuredState::decode(state)?;
                        *structured
                            .lock()
                            .map_err(|_| "structured native state lock is poisoned".to_owned())? =
                            decoded;
                        Ok(())
                    },
                )
            } else {
                self.services.get_mut(key).map_or_else(
                    || Err(format!("native service {key:?} is not registered")),
                    |service| service.restore(state),
                )
            };
            if let Err(error) = outcome {
                for (rollback_key, rollback) in &previous {
                    if *rollback_key == bundle_key() {
                        if let (Some(structured), Ok(decoded)) =
                            (&self.structured, StructuredState::decode(rollback))
                            && let Ok(mut state) = structured.lock()
                        {
                            *state = decoded;
                        }
                    } else if let Some(service) = self.services.get_mut(rollback_key) {
                        let _ = service.restore(rollback);
                    }
                }
                return Err(error);
            }
        }
        Ok(())
    }

    /// Build the registry required by a replacement artifact while retaining every
    /// service state whose stable import identity still exists. New services start
    /// from their deterministic default; removed services are dropped only after the
    /// VM has accepted the replacement generation.
    pub(crate) fn migrated_for_artifact(
        &self,
        artifact: &BytecodeArtifact,
    ) -> Result<Self, String> {
        let previous = self.snapshots()?;
        let mut target = Self::for_artifact(artifact);
        let retained = previous
            .into_iter()
            .filter(|(key, _)| {
                target.services.contains_key(key)
                    || (*key == bundle_key() && target.structured.is_some())
            })
            .collect();
        target.restore_snapshots(&retained)?;
        target.set_character_width_mode(self.character_width_mode.get());
        Ok(target)
    }
}

mod core;
mod services;
#[cfg(test)]
mod tests;

use core::CoreNative;
pub use core::evaluate_pure_native;
#[cfg(test)]
use core::{parse_era_numeric, substring_legacy_bytes, substring_scalars};
use services::{CompilerNative, RandomNative};
#[cfg(test)]
use services::{apply_width, apply_width_with_mode};
