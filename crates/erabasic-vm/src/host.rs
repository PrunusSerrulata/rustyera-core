use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use erabasic_bytecode::{BytecodeArtifact, RuntimeImport, SymbolKey};
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
    /// # Errors
    ///
    /// Returns an error when the service rejects the request or cannot produce a result.
    fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, String>;

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
    random: Option<Arc<Mutex<Sfmt19937>>>,
    structured: Option<Arc<Mutex<StructuredState>>>,
    structured_keys: BTreeSet<SymbolKey>,
    extensions: erabasic_data::ExtensionData,
}

type PreparedStructuredImport = (Option<Vec<u8>>, BTreeSet<(u8, String)>);

impl NativeServiceRegistry {
    pub(crate) fn fork_for_artifact(&self, artifact: &BytecodeArtifact) -> Result<Self, String> {
        let snapshots = self.snapshots()?;
        let mut fork = Self::for_artifact(artifact);
        fork.restore_snapshots(&snapshots)?;
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
            } else if matches!(name, "format_integer" | "format_string")
                || name.starts_with("control_")
            {
                registry.register(native.import.key, CompilerNative { name: name.into() });
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
                    | "toint"
                    | "isnumeric"
                    | "unicode"
                    | "convert"
                    | "color_fromrgb"
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
                registry.register(native.import.key, CoreNative { name: name.into() });
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
    ) -> Result<NativeReady, String> {
        self.services
            .get_mut(&key)
            .ok_or_else(|| format!("native service {key:?} is not registered"))?
            .call(request)
    }

    pub(crate) fn checkpoint(&self, key: SymbolKey) -> Result<Option<Vec<u8>>, String> {
        if self.structured_keys.contains(&key) {
            return self
                .structured
                .as_ref()
                .ok_or_else(|| "structured native bundle is not registered".to_owned())?
                .lock()
                .map_err(|_| "structured native state lock is poisoned".to_owned())?
                .encode()
                .map(Some);
        }
        self.services
            .get(&key)
            .ok_or_else(|| format!("native service {key:?} is not registered"))?
            .snapshot()
    }

    pub(crate) fn rollback(&mut self, key: SymbolKey, state: &[u8]) -> Result<(), String> {
        if self.structured_keys.contains(&key) {
            let decoded = StructuredState::decode(state)?;
            *self
                .structured
                .as_ref()
                .ok_or_else(|| "structured native bundle is not registered".to_owned())?
                .lock()
                .map_err(|_| "structured native state lock is poisoned".to_owned())? = decoded;
            return Ok(());
        }
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
        Ok(target)
    }
}

struct CoreNative {
    name: String,
}

/// Evaluate a side-effect-free core native through the same implementation used by bytecode.
///
/// Debuggers use this deliberately narrow entry point instead of maintaining a second set of
/// `EraBasic` numeric and string semantics. Natives which require implicit variables, mutate
/// state, consume entropy, or cross the Host boundary are rejected here.
///
/// # Errors
///
/// Returns an error when the native is not in the pure whitelist, its arguments do not match the
/// reference signature, or its ordinary evaluation reports a domain or format error.
pub fn evaluate_pure_native(name: &str, arguments: Vec<VmValue>) -> Result<VmValue, String> {
    let name = name.to_ascii_lowercase();
    if !matches!(
        name.as_str(),
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
            | "toint"
            | "isnumeric"
            | "unicode"
            | "convert"
            | "color_fromrgb"
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
            | "replace"
            | "escape"
            | "unicodetostr"
            | "encodetouni"
            | "unicodebyte"
            | "charatu"
            | "tolower"
            | "toupper"
    ) {
        return Err(format!("{name} is not a pure core-native service"));
    }
    let request = NativeCallRequest {
        import: RuntimeImport {
            key: SymbolKey::default(),
            namespace: "debug".into(),
            name: name.clone(),
            abi_version: 1,
            parameters: Vec::new(),
            result: None,
        },
        arguments,
        places: Vec::new(),
        implicit_places: BTreeMap::new(),
    };
    CoreNative { name }
        .call(request)?
        .value
        .ok_or_else(|| "pure core-native service returned no value".into())
}

impl NativeService for CoreNative {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::too_many_lines
    )]
    fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, String> {
        let args = &request.arguments;
        let integer = |index: usize| match args.get(index) {
            Some(VmValue::Integer(value)) => Ok(*value),
            _ => Err(format!(
                "{} argument {} must be integer",
                self.name,
                index + 1
            )),
        };
        let string = |index: usize| match args.get(index) {
            Some(VmValue::String(value)) => Ok(value.as_str()),
            _ => Err(format!(
                "{} argument {} must be string",
                self.name,
                index + 1
            )),
        };
        let result = match self.name.as_str() {
            "abs" => VmValue::Integer(
                integer(0)?
                    .checked_abs()
                    .ok_or("ABS cannot accept the minimum signed integer")?,
            ),
            "sign" => VmValue::Integer(integer(0)?.signum()),
            "sqrt" => {
                let value = integer(0)?;
                if value < 0 {
                    return Err("SQRT argument 1 is negative".into());
                }
                VmValue::Integer((value as f64).sqrt() as i64)
            }
            "cbrt" => {
                let value = integer(0)?;
                if value < 0 {
                    return Err("CBRT argument 1 is negative".into());
                }
                VmValue::Integer((value as f64).powf(1.0 / 3.0) as i64)
            }
            "log" => checked_float_to_integer((integer(0)? as f64).ln(), "LOG")?,
            "log10" => checked_float_to_integer((integer(0)? as f64).log10(), "LOG10")?,
            "exponent" => checked_float_to_integer((integer(0)? as f64).exp(), "EXPONENT")?,
            "power" => {
                checked_float_to_integer((integer(0)? as f64).powf(integer(1)? as f64), "POWER")?
            }
            "getbit" => {
                let bit = u32::try_from(integer(1)?).map_err(|_| "GETBIT index is negative")?;
                VmValue::Integer(if bit < 64 {
                    (integer(0)? >> bit) & 1
                } else {
                    0
                })
            }
            "bitcount" => VmValue::Integer(i64::from(integer(0)?.count_ones())),
            "strlen" | "strlens" => {
                VmValue::Integer(i64::try_from(string(0)?.len()).unwrap_or(i64::MAX))
            }
            "strlenu" | "strlensu" => {
                // Emuera runs on .NET, so the U variants count UTF-16 code units rather than
                // Unicode scalar values. This remains observable for supplementary characters.
                VmValue::Integer(
                    i64::try_from(string(0)?.encode_utf16().count()).unwrap_or(i64::MAX),
                )
            }
            "toint" => VmValue::Integer(parse_era_numeric(string(0)?, false)?.unwrap_or(0)),
            "isnumeric" => {
                VmValue::Integer(i64::from(parse_era_numeric(string(0)?, true)?.is_some()))
            }
            "convert" => {
                let value = integer(0)?;
                VmValue::String(match integer(1)? {
                    2 => format!("{:b}", value.cast_unsigned()),
                    8 => format!("{:o}", value.cast_unsigned()),
                    10 => value.to_string(),
                    16 => format!("{:x}", value.cast_unsigned()),
                    _ => return Err("CONVERT base must be 2, 8, 10, or 16".into()),
                })
            }
            "color_fromrgb" => {
                let channels = [integer(0)?, integer(1)?, integer(2)?];
                if channels.iter().any(|value| !(0..=255).contains(value)) {
                    return Err("COLOR_FROMRGB channels must be between 0 and 255".into());
                }
                VmValue::Integer((channels[0] << 16) | (channels[1] << 8) | channels[2])
            }
            "unicode" => {
                let value = u32::try_from(integer(0)?)
                    .map_err(|_| "UNICODE argument is outside the UTF-16 code-unit range")?;
                if value > u32::from(u16::MAX) {
                    return Err("UNICODE argument is outside the UTF-16 code-unit range".into());
                }
                // Rust strings cannot contain isolated UTF-16 surrogates.  BMP
                // scalar values otherwise have the same UTF-8 observable text.
                let scalar = char::from_u32(value)
                    .ok_or("UNICODE argument is an isolated UTF-16 surrogate")?;
                let control = (value < 0x1f && value != 0x0a && value != 0x0d)
                    || (0x7f..=0x9f).contains(&value);
                VmValue::String(if control {
                    String::new()
                } else {
                    scalar.to_string()
                })
            }
            "max" => VmValue::Integer(
                args.iter()
                    .enumerate()
                    .map(|(index, _)| integer(index))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .max()
                    .ok_or("MAX requires an argument")?,
            ),
            "min" => VmValue::Integer(
                args.iter()
                    .enumerate()
                    .map(|(index, _)| integer(index))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .min()
                    .ok_or("MIN requires an argument")?,
            ),
            "limit" => {
                let minimum = integer(1)?;
                let maximum = integer(2)?;
                if minimum > maximum {
                    return Err("LIMIT minimum exceeds maximum".into());
                }
                VmValue::Integer(integer(0)?.clamp(minimum, maximum))
            }
            "inrange" => VmValue::Integer(i64::from(
                (integer(1)?..=integer(2)?).contains(&integer(0)?),
            )),
            "tostr" => VmValue::String(integer(0)?.to_string()),
            "substring" => VmValue::String(substring_utf8_bytes(
                string(0)?,
                integer(1)?,
                args.get(2).map(|_| integer(2)).transpose()?,
            )?),
            "substringu" => VmValue::String(substring_scalars(
                string(0)?,
                integer(1)?,
                args.get(2).map(|_| integer(2)).transpose()?,
            )?),
            "strfind" => {
                let start = usize::try_from(integer(2).unwrap_or(0)).unwrap_or(usize::MAX);
                let haystack = string(0)?;
                let start = utf8_boundary_at_or_after(haystack, start).min(haystack.len());
                VmValue::Integer(
                    haystack[start..]
                        .find(string(1)?)
                        .and_then(|offset| i64::try_from(start + offset).ok())
                        .unwrap_or(-1),
                )
            }
            "strfindu" => {
                let haystack = string(0)?;
                let start = integer(2).unwrap_or(0);
                let Ok(start) = usize::try_from(start) else {
                    return Ok(NativeReady::value(VmValue::Integer(-1)));
                };
                let scalar_count = haystack.chars().count();
                if start >= scalar_count {
                    VmValue::Integer(-1)
                } else {
                    let byte_start = haystack
                        .char_indices()
                        .nth(start)
                        .map_or(haystack.len(), |(offset, _)| offset);
                    VmValue::Integer(
                        haystack[byte_start..]
                            .find(string(1)?)
                            .and_then(|offset| {
                                i64::try_from(haystack[..byte_start + offset].chars().count()).ok()
                            })
                            .unwrap_or(-1),
                    )
                }
            }
            "strcount" => {
                let regex = regex::Regex::new(string(1)?)
                    .map_err(|error| format!("STRCOUNT argument 2 is not a regex: {error}"))?;
                VmValue::Integer(
                    i64::try_from(regex.find_iter(string(0)?).count()).unwrap_or(i64::MAX),
                )
            }
            "getpalamlv" | "getexplv" => {
                let maximum = usize::try_from(integer(1)?)
                    .map_err(|_| format!("{} maximum level is negative", self.name))?;
                let variable = if self.name == "getpalamlv" {
                    "PALAMLV"
                } else {
                    "EXPLV"
                };
                let levels = request
                    .implicit_places
                    .get(variable)
                    .ok_or_else(|| format!("{variable} is not available"))?;
                let mut level = maximum;
                for index in 0..maximum {
                    let threshold = match levels.values.get(index + 1) {
                        Some(VmValue::Integer(value)) => *value,
                        _ => return Err(format!("{variable}[{}] is unavailable", index + 1)),
                    };
                    if integer(0)? < threshold {
                        level = index;
                        break;
                    }
                }
                VmValue::Integer(i64::try_from(level).unwrap_or(i64::MAX))
            }
            "replace" => VmValue::String(string(0)?.replace(string(1)?, string(2)?)),
            "escape" => VmValue::String(regex::escape(string(0)?)),
            "charatu" => {
                let position = usize::try_from(integer(1)?).unwrap_or(usize::MAX);
                VmValue::String(
                    string(0)?
                        .chars()
                        .nth(position)
                        .map_or_else(String::new, |value| value.to_string()),
                )
            }
            "encodetouni" => {
                let value = string(0)?;
                if value.is_empty() {
                    VmValue::Integer(-1)
                } else {
                    let position = usize::try_from(integer(1).unwrap_or(0))
                        .map_err(|_| "ENCODETOUNI position is negative")?;
                    let scalar = value
                        .chars()
                        .nth(position)
                        .ok_or("ENCODETOUNI position exceeds the string")?;
                    VmValue::Integer(i64::from(u32::from(scalar)))
                }
            }
            "unicodebyte" => VmValue::Integer(i64::from(u32::from(
                string(0)?
                    .chars()
                    .next()
                    .ok_or("UNICODEBYTE input is empty")?,
            ))),
            "tolower" => VmValue::String(string(0)?.to_lowercase()),
            "toupper" => VmValue::String(string(0)?.to_uppercase()),
            "unicodetostr" => {
                let scalar = u32::try_from(integer(0)?)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or("UNICODETOSTR argument is not a Unicode scalar")?;
                VmValue::String(scalar.to_string())
            }
            _ => return Err(format!("unknown core-native service {}", self.name)),
        };
        Ok(NativeReady::value(result))
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn checked_float_to_integer(value: f64, operation: &str) -> Result<VmValue, String> {
    if value.is_nan() {
        return Err(format!("{operation} result is NaN"));
    }
    if value.is_infinite() {
        return Err(format!("{operation} result is infinite"));
    }
    if value >= i64::MAX as f64 || value <= i64::MIN as f64 {
        return Err(format!("{operation} result is outside signed 64-bit range"));
    }
    Ok(VmValue::Integer(value as i64))
}

/// Parse the numeric grammar used by Emuera's TOINT and ISNUMERIC methods.
/// Unlike Rust's `FromStr`, the reference accepts binary/hex prefixes,
/// integer exponents, and a discarded decimal fraction, but never whitespace.
// The float conversion deliberately mirrors the reference's Math.Pow and
// unchecked Int64 cast instead of substituting an integer exponent algorithm.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::too_many_lines
)]
fn parse_era_numeric(value: &str, numeric_check: bool) -> Result<Option<i64>, String> {
    if value.is_empty() || !value.is_ascii() {
        return Ok(None);
    }
    let bytes = value.as_bytes();
    if !bytes[0].is_ascii_digit() && !matches!(bytes[0], b'+' | b'-') {
        return Ok(None);
    }
    if matches!(bytes[0], b'+' | b'-') && bytes.get(1).is_none_or(|next| !next.is_ascii_digit()) {
        return Ok(None);
    }

    let mut index = 0;
    let mut radix = 10;
    if bytes.starts_with(b"0x") || bytes.starts_with(b"0X") {
        radix = 16;
        index = 2;
    } else if bytes.starts_with(b"0b") || bytes.starts_with(b"0B") {
        radix = 2;
        index = 2;
    }
    let digits_start = index;
    if radix == 10
        && bytes
            .get(index)
            .is_some_and(|byte| matches!(byte, b'+' | b'-'))
    {
        index += 1;
    }
    let unsigned_start = index;
    while let Some(byte) = bytes.get(index) {
        if byte.is_ascii_digit() {
            if radix == 2 && !matches!(byte, b'0' | b'1') {
                return Err("binary notation contains a digit other than 0 or 1".into());
            }
            index += 1;
        } else if radix == 16 && byte.is_ascii_hexdigit() {
            index += 1;
        } else {
            break;
        }
    }
    if index == unsigned_start {
        return Ok(None);
    }
    let significand = if radix == 10 {
        value[digits_start..index]
            .parse::<i64>()
            .map_err(|_| "numeric significand exceeds signed 64-bit range")?
    } else {
        let raw = u64::from_str_radix(&value[unsigned_start..index], radix)
            .map_err(|_| "numeric significand exceeds 64-bit range")?;
        raw.cast_signed()
    };

    let mut result = significand;
    if bytes
        .get(index)
        .is_some_and(|byte| matches!(byte, b'p' | b'P' | b'e' | b'E'))
    {
        let exponent_base = if matches!(bytes[index], b'p' | b'P') {
            2_f64
        } else {
            10_f64
        };
        index += 1;
        if numeric_check && bytes.get(index).is_none_or(|byte| !byte.is_ascii_digit()) {
            return Ok(None);
        }
        let exponent_start = index;
        if !numeric_check
            && bytes
                .get(index)
                .is_some_and(|byte| matches!(byte, b'+' | b'-'))
        {
            index += 1;
        }
        let exponent_digits = index;
        while let Some(byte) = bytes.get(index) {
            let valid = if radix == 16 {
                byte.is_ascii_hexdigit()
            } else {
                byte.is_ascii_digit()
            };
            if !valid {
                break;
            }
            if radix == 2 && !matches!(byte, b'0' | b'1') {
                return Err("binary exponent contains a digit other than 0 or 1".into());
            }
            index += 1;
        }
        if index == exponent_digits {
            return if numeric_check {
                Ok(None)
            } else {
                Err("numeric exponent has no digits".into())
            };
        }
        let exponent = i32::from_str_radix(&value[exponent_start..index], radix)
            .map_err(|_| "numeric exponent exceeds supported range")?;
        let expanded = (significand as f64) * exponent_base.powi(exponent);
        if !expanded.is_finite() || expanded > i64::MAX as f64 || expanded < i64::MIN as f64 {
            return Err("numeric value exceeds signed 64-bit range".into());
        }
        result = expanded as i64;
    }

    if bytes.get(index) == Some(&b'.') {
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
    }
    Ok((index == bytes.len()).then_some(result))
}

fn substring_utf8_bytes(value: &str, start: i64, length: Option<i64>) -> Result<String, String> {
    let start = usize::try_from(start).map_err(|_| "SUBSTRING start is negative")?;
    let start = utf8_boundary_at_or_after(value, start.min(value.len()));
    let requested_end = match length {
        Some(length) => start
            .saturating_add(usize::try_from(length).map_err(|_| "SUBSTRING length is negative")?),
        None => value.len(),
    };
    let end = utf8_boundary_at_or_after(value, requested_end.min(value.len()));
    Ok(value[start..end].into())
}

fn substring_scalars(value: &str, start: i64, length: Option<i64>) -> Result<String, String> {
    let start = usize::try_from(start).map_err(|_| "SUBSTRINGU start is negative")?;
    let length = length
        .map(|length| usize::try_from(length).map_err(|_| "SUBSTRINGU length is negative"))
        .transpose()?;
    Ok(value
        .chars()
        .skip(start)
        .take(length.unwrap_or(usize::MAX))
        .collect())
}

fn utf8_boundary_at_or_after(value: &str, mut offset: usize) -> usize {
    while offset < value.len() && !value.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

struct CompilerNative {
    name: String,
}

impl NativeService for CompilerNative {
    fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, String> {
        match self.name.as_str() {
            "format_integer" => {
                let Some(VmValue::Integer(value)) = request.arguments.first() else {
                    return Err("format_integer expects an integer".into());
                };
                Ok(NativeReady::value(VmValue::String(apply_width(
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
                Ok(NativeReady::value(VmValue::String(apply_width(
                    &value,
                    request.arguments.get(1),
                )?)))
            }
            name if name.starts_with("control_") => Err(format!(
                "compiler control placeholder {name} reached execution"
            )),
            _ => Err(format!("unknown compiler-native service {}", self.name)),
        }
    }
}

struct RandomNative {
    name: String,
    state: Arc<Mutex<Sfmt19937>>,
}

impl NativeService for RandomNative {
    fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "SFMT state lock is poisoned".to_owned())?;
        match self.name.as_str() {
            "rand" => {
                let Some(VmValue::Integer(maximum)) = request.arguments.first() else {
                    return Err("RAND expects an integer maximum".into());
                };
                if *maximum <= 0 {
                    return Err("RAND maximum must be positive".into());
                }
                let value = state.next_u64() % (*maximum).cast_unsigned();
                Ok(NativeReady::value(VmValue::Integer(
                    i64::try_from(value).expect("RAND modulo positive i64 fits i64"),
                )))
            }
            "randomize" => {
                let seed = match request.arguments.first() {
                    Some(VmValue::Integer(seed)) => (*seed).cast_unsigned(),
                    None => 0,
                    _ => return Err("RANDOMIZE seed must be an integer".into()),
                };
                state.reseed(seed);
                Ok(NativeReady::default())
            }
            "initrand" | "dumprand" => Err(format!(
                "{} must be executed through the VM place transaction",
                self.name.to_ascii_uppercase()
            )),
            _ => Err(format!("unknown random-native service {}", self.name)),
        }
    }

    fn snapshot(&self) -> Result<Option<Vec<u8>>, String> {
        self.state
            .lock()
            .map(|state| Some(state.encode()))
            .map_err(|_| "SFMT state lock is poisoned".into())
    }

    fn restore(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.state
            .lock()
            .map_err(|_| "SFMT state lock is poisoned".to_owned())?
            .decode(bytes)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_u_substring_uses_utf8_bytes_and_advances_to_boundaries() {
        assert_eq!(substring_utf8_bytes("A界B", 1, Some(1)), Ok("界".into()));
        assert_eq!(substring_utf8_bytes("A界B", 2, Some(1)), Ok("B".into()));
        assert_eq!(substring_scalars("A界B", 1, Some(1)), Ok("界".into()));
    }

    #[test]
    fn era_numeric_parser_keeps_reference_prefix_fraction_and_whitespace_rules() {
        assert_eq!(parse_era_numeric("12.99", false), Ok(Some(12)));
        assert_eq!(parse_era_numeric("0x10", false), Ok(Some(16)));
        assert_eq!(parse_era_numeric("0b101", true), Ok(Some(5)));
        assert_eq!(parse_era_numeric("2e3", false), Ok(Some(2_000)));
        assert_eq!(parse_era_numeric(" 12", false), Ok(None));
        assert_eq!(parse_era_numeric("１２", true), Ok(None));
        assert_eq!(parse_era_numeric("12x", true), Ok(None));
    }

    #[test]
    fn random_native_rejects_non_positive_modulus() {
        let mut native = RandomNative {
            name: "rand".into(),
            state: Arc::new(Mutex::new(Sfmt19937::new(1))),
        };
        assert!(
            native
                .call(NativeCallRequest {
                    import: RuntimeImport {
                        key: SymbolKey([0; 16]),
                        namespace: "test".into(),
                        name: "rand".into(),
                        abi_version: 1,
                        parameters: vec![],
                        result: None,
                    },
                    arguments: vec![VmValue::Integer(0)],
                    places: Vec::new(),
                    implicit_places: BTreeMap::new(),
                })
                .is_err()
        );
    }

    #[test]
    fn regex_string_natives_match_non_overlapping_reference_semantics() {
        let request = |name: &str, arguments: Vec<VmValue>| NativeCallRequest {
            import: RuntimeImport {
                key: SymbolKey([0; 16]),
                namespace: "test".into(),
                name: name.into(),
                abi_version: 1,
                parameters: vec![],
                result: None,
            },
            arguments,
            places: Vec::new(),
            implicit_places: BTreeMap::new(),
        };
        let mut count = CoreNative {
            name: "strcount".into(),
        };
        assert_eq!(
            count
                .call(request(
                    "strcount",
                    vec![
                        VmValue::String("ababa".into()),
                        VmValue::String("aba".into())
                    ],
                ))
                .unwrap()
                .value,
            Some(VmValue::Integer(1))
        );
        let mut escape = CoreNative {
            name: "escape".into(),
        };
        assert_eq!(
            escape
                .call(request("escape", vec![VmValue::String("a+b".into())]))
                .unwrap()
                .value,
            Some(VmValue::String("a\\+b".into()))
        );
    }
}
