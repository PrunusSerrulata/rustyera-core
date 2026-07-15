use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use erabasic_bytecode::{BytecodeArtifact, RuntimeImport, SymbolKey};
use serde::{Deserialize, Serialize};

use crate::sfmt::Sfmt19937;
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
    random: Option<Arc<Mutex<Sfmt19937>>>,
}

impl NativeServiceRegistry {
    /// Register the small VM-native services emitted directly by the compiler.
    /// Project-specific builtins remain explicit services and fail closed when absent.
    #[must_use]
    pub fn for_artifact(artifact: &BytecodeArtifact) -> Self {
        Self::for_artifact_with_seed(artifact, 0)
    }

    #[must_use]
    pub fn for_artifact_with_seed(artifact: &BytecodeArtifact, seed: u64) -> Self {
        let mut registry = Self::default();
        let random = Arc::new(Mutex::new(Sfmt19937::new(seed)));
        registry.random = Some(Arc::clone(&random));
        for native in &artifact.native_imports {
            let name = native.import.name.as_str();
            if matches!(name, "format_integer" | "format_string") || name.starts_with("control_") {
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
                    | "max"
                    | "min"
                    | "limit"
                    | "inrange"
                    | "tostr"
                    | "substring"
                    | "substringu"
                    | "strfind"
                    | "replace"
                    | "unicodetostr"
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
    ) -> Result<Option<VmValue>, String> {
        self.services
            .get_mut(&key)
            .ok_or_else(|| format!("native service {key:?} is not registered"))?
            .call(request)
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

struct CoreNative {
    name: String,
}

impl NativeService for CoreNative {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::too_many_lines
    )]
    fn call(&mut self, request: NativeCallRequest) -> Result<Option<VmValue>, String> {
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
            "strlen" => VmValue::Integer(i64::try_from(string(0)?.len()).unwrap_or(i64::MAX)),
            "strlenu" => {
                VmValue::Integer(i64::try_from(string(0)?.chars().count()).unwrap_or(i64::MAX))
            }
            "toint" => VmValue::Integer(
                string(0)?
                    .trim()
                    .parse()
                    .map_err(|_| "TOINT input is not an integer")?,
            ),
            "isnumeric" => VmValue::Integer(i64::from(string(0)?.trim().parse::<i64>().is_ok())),
            "unicode" => VmValue::Integer(
                string(0)?
                    .chars()
                    .next()
                    .map_or(0, |character| i64::from(u32::from(character))),
            ),
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
            "replace" => VmValue::String(string(0)?.replace(string(1)?, string(2)?)),
            "unicodetostr" => {
                let scalar = u32::try_from(integer(0)?)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or("UNICODETOSTR argument is not a Unicode scalar")?;
                VmValue::String(scalar.to_string())
            }
            _ => return Err(format!("unknown core-native service {}", self.name)),
        };
        Ok(Some(result))
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
    fn call(&mut self, request: NativeCallRequest) -> Result<Option<VmValue>, String> {
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
                Ok(Some(VmValue::Integer(
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
                Ok(None)
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
                })
                .is_err()
        );
    }
}
