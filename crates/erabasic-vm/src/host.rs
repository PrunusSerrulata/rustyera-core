use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::BuildHasherDefault;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use erabasic_bytecode::{BytecodeArtifact, HostImport, RuntimeImport, SymbolKey};
use erabasic_data::LegacyEncoding;
use serde::{Deserialize, Serialize};

use crate::memory::SymbolKeyHasher;
use crate::sfmt::Sfmt19937;
use crate::structured::{ColumnIdentityStamp, StructuredExtension, StructuredScope};
use crate::structured::{StructuredNative, StructuredState, bundle_key, is_structured_name};
use crate::{
    ExecutionFailure, FaultCategory, FiberId, HostRequestId, HostWrite, PlaceDescriptor,
    ScriptFaultKind, VmExecutionOrigin, VmFaultCode, VmValue,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostWaitStability {
    StableInput,
    Transient,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostCallRequest {
    /// Explicit omitted source slots, separate from a real Integer MIN value.
    pub omitted_arguments: Vec<usize>,
    pub id: HostRequestId,
    pub fiber: FiberId,
    pub import: RuntimeImport,
    pub arguments: Vec<VmValue>,
    pub origin: VmExecutionOrigin,
}

/// Borrowed call-site data offered before the VM materializes a persistent Host request.
/// Implementations may handle only operations that are immediately and infallibly ready;
/// returning [`ImmediateHostCallResult::Unsupported`] preserves the ordinary caller-pumped
/// boundary.
pub struct ImmediateHostCall<'a> {
    pub fiber: FiberId,
    pub import: &'a HostImport,
    pub normalized_name: &'a str,
    pub arguments: &'a [VmValue],
    pub omitted_arguments: &'a [usize],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImmediateHostCallResult {
    Unsupported,
    /// The call completed synchronously. Like [`HostCallResult::Ready`], this does not
    /// implicitly yield the current fiber; the configured fiber quantum still bounds
    /// cooperative scheduling.
    Ready(HostReady),
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
    Error(ExecutionFailure),
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
    /// Returns whether an immediately handled Host operation is a deterministic function of its
    /// arguments. Path memoization may only cross operations whose current implementation makes
    /// this guarantee; the default keeps arbitrary and replaceable hosts as hard boundaries.
    fn path_memo_safe(&self, _import: &RuntimeImport) -> bool {
        false
    }

    fn call_immediate(&mut self, _request: ImmediateHostCall<'_>) -> ImmediateHostCallResult {
        ImmediateHostCallResult::Unsupported
    }

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
    /// Registry/transaction key. Dynamic physical import.key is deliberately separate.
    pub service_key: SymbolKey,
    pub omitted_arguments: Vec<usize>,
    pub import: RuntimeImport,
    pub arguments: Vec<VmValue>,
    /// Immutable snapshots of place arguments. Native services cannot dereference
    /// VM places themselves; the interpreter resolves every view before calling
    /// the service and validates returned writes again before committing them.
    pub places: Vec<NativePlaceView>,
    /// Reference pseudo-variables used by legacy multi-result functions.
    pub implicit_places: BTreeMap<String, NativePlaceView>,
}

impl NativeCallRequest {
    #[must_use]
    pub fn argument(&self, index: usize) -> Option<&VmValue> {
        (!self.omitted_arguments.contains(&index))
            .then(|| self.arguments.get(index))
            .flatten()
    }
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
    fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, ExecutionFailure>;

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

mod core;
mod maps;
mod services;
#[cfg(test)]
mod tests;

use core::CoreNative;
pub use core::{evaluate_pure_native, evaluate_pure_native_with_compatibility};
#[cfg(test)]
use core::{parse_era_numeric, substring_legacy_bytes, substring_scalars};
pub(crate) use services::apply_owned_width_with_mode;
#[cfg(test)]
use services::apply_width;
#[cfg(test)]
pub(crate) use services::apply_width_with_mode;
use services::{CompilerNative, RandomNative};

// Native execution keeps the historical Native fault code. These source constructors choose
// catchability explicitly; no caller infers it from a message or the old broad fault code.
pub(crate) fn native_contract_failure(message: impl Into<String>) -> ExecutionFailure {
    ExecutionFailure::classified(FaultCategory::HostContract, VmFaultCode::Native, message)
}

fn native_script_failure(kind: ScriptFaultKind, message: impl Into<String>) -> ExecutionFailure {
    ExecutionFailure::script(kind, VmFaultCode::Native, message)
}

fn native_resource_failure(message: impl Into<String>) -> ExecutionFailure {
    ExecutionFailure::classified(FaultCategory::ResourceLimit, VmFaultCode::Native, message)
}

fn core_native_name(name: &str) -> bool {
    matches!(
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
            | "unchecked_add"
            | "unchecked_sub"
            | "unchecked_mul"
            | "unchecked_neg"
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
    )
}

impl HostCallRequest {
    /// Source-aware argument lookup; an explicit omission is not a literal MIN.
    #[must_use]
    pub fn argument(&self, index: usize) -> Option<&VmValue> {
        if self.omitted_arguments.binary_search(&index).is_ok() {
            None
        } else {
            self.arguments.get(index)
        }
    }
}

#[cfg(test)]
mod source_presence_tests {
    use super::*;
    #[test]
    fn direct_host_omission_does_not_remove_slots_or_hide_literal_minimum() {
        let request = HostCallRequest {
            id: crate::HostRequestId(1),
            fiber: crate::FiberId(1),
            import: erabasic_bytecode::RuntimeImport {
                key: erabasic_bytecode::SymbolKey::default(),
                namespace: "test".into(),
                name: "source-presence".into(),
                abi_version: 1,
                parameters: vec![erabasic_bytecode::BytecodeType::Integer; 3],
                result: None,
            },
            arguments: vec![
                VmValue::Integer(i64::MIN),
                VmValue::Integer(i64::MIN),
                VmValue::Integer(9),
            ],
            omitted_arguments: vec![1],
            origin: crate::VmExecutionOrigin {
                generation: crate::GenerationId(1),
                function: erabasic_bytecode::SymbolKey::default(),
                function_name: "test".into(),
                instruction: 0,
                command: "source-presence".into(),
                source: None,
            },
        };
        assert_eq!(request.argument(0), Some(&VmValue::Integer(i64::MIN)));
        assert_eq!(request.argument(1), None);
        assert_eq!(request.argument(2), Some(&VmValue::Integer(9)));
        assert_eq!(request.arguments.len(), 3);
    }
}

mod registry;
use registry::CharacterWidthModeHandle;
pub use registry::NativeServiceRegistry;
#[cfg(test)]
use registry::compiler_native_path_memo_safe;
