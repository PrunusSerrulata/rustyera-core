use std::fmt;

use erabasic_bytecode::{ResolvedSourceLocation, SymbolKey};
use serde::{Deserialize, Serialize};

use crate::{FiberId, GenerationId};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VmFaultCode {
    InvalidInstruction,
    StackUnderflow,
    TypeMismatch,
    Bounds,
    DivideByZero,
    MissingSymbol,
    Host,
    Native,
    Trap,
    ResourceLimit,
    RunawayExecution,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VmFault {
    pub correlation_id: u64,
    pub parent_correlation_id: Option<u64>,
    pub category: crate::FaultCategory,
    pub code: VmFaultCode,
    pub message: String,
    pub fiber: FiberId,
    pub generation: GenerationId,
    pub function: SymbolKey,
    pub function_name: String,
    pub instruction: u32,
    pub command: String,
    pub source: Option<ResolvedSourceLocation>,
    pub secondary: Option<Box<VmFault>>,
}

impl VmFault {
    pub(crate) fn from_origin(
        fiber: FiberId,
        origin: crate::VmExecutionOrigin,
        failure: crate::ExecutionFailure,
    ) -> Self {
        let mut identity = Vec::with_capacity(64 + failure.message.len());
        identity.extend_from_slice(&fiber.0.to_le_bytes());
        identity.extend_from_slice(&origin.generation.0.to_le_bytes());
        identity.extend_from_slice(&origin.function.0);
        identity.extend_from_slice(&origin.instruction.to_le_bytes());
        identity.push(fault_category_discriminant(failure.category));
        identity.push(fault_code_discriminant(failure.code));
        identity.extend_from_slice(failure.message.as_bytes());
        let digest = erabasic_bytecode::Digest::hash("rustyera.vm.fault.v1", &[&identity]);
        let correlation_id = u64::from_le_bytes(
            digest.0[..8]
                .try_into()
                .expect("fault digest contains eight identity bytes"),
        );
        Self {
            correlation_id,
            parent_correlation_id: None,
            category: failure.category,
            code: failure.code,
            message: failure.message,
            fiber,
            generation: origin.generation,
            function: origin.function,
            function_name: origin.function_name,
            instruction: origin.instruction,
            command: origin.command,
            source: origin.source,
            secondary: None,
        }
    }

    pub(crate) fn attach_secondary(&mut self, mut secondary: Self) {
        secondary.parent_correlation_id = Some(self.correlation_id);
        secondary.secondary = None;
        self.secondary = Some(Box::new(secondary));
    }

    #[must_use]
    pub fn origin(&self) -> crate::VmExecutionOrigin {
        crate::VmExecutionOrigin {
            generation: self.generation,
            function: self.function,
            function_name: self.function_name.clone(),
            instruction: self.instruction,
            command: self.command.clone(),
            source: self.source.clone(),
        }
    }
}

const fn fault_category_discriminant(category: crate::FaultCategory) -> u8 {
    match category {
        crate::FaultCategory::Script(kind) => match kind {
            crate::ScriptFaultKind::Parse => 0,
            crate::ScriptFaultKind::Resolve => 1,
            crate::ScriptFaultKind::Argument => 2,
            crate::ScriptFaultKind::Bounds => 3,
            crate::ScriptFaultKind::Arithmetic => 4,
            crate::ScriptFaultKind::Assertion => 5,
            crate::ScriptFaultKind::ExplicitThrow => 6,
            crate::ScriptFaultKind::Operation => 7,
        },
        crate::FaultCategory::ResourceLimit => 8,
        crate::FaultCategory::Cancellation => 9,
        crate::FaultCategory::InternalInvariant => 10,
        crate::FaultCategory::HostContract => 11,
        crate::FaultCategory::Protocol => 12,
        crate::FaultCategory::Permission => 13,
        crate::FaultCategory::Infrastructure => 14,
    }
}

const fn fault_code_discriminant(code: VmFaultCode) -> u8 {
    match code {
        VmFaultCode::InvalidInstruction => 0,
        VmFaultCode::StackUnderflow => 1,
        VmFaultCode::TypeMismatch => 2,
        VmFaultCode::Bounds => 3,
        VmFaultCode::DivideByZero => 4,
        VmFaultCode::MissingSymbol => 5,
        VmFaultCode::Host => 6,
        VmFaultCode::Native => 7,
        VmFaultCode::Trap => 8,
        VmFaultCode::ResourceLimit => 9,
        VmFaultCode::RunawayExecution => 10,
    }
}

impl fmt::Display for VmFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for VmFault {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VmError {
    /// Classified at the script operation which failed, never from its message.
    ScriptFailure(crate::ExecutionFailure),
    MissingFunction(SymbolKey),
    InvalidArguments(String),
    ResourceLimit(&'static str),
    InvalidState(String),
    UnknownFiber(FiberId),
    StaleHostRequest(crate::HostRequestId),
    HotReload(String),
    Snapshot(String),
    Save(String),
}

impl fmt::Display for VmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for VmError {}
