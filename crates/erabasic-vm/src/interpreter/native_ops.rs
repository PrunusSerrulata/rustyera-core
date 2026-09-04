//! Complex native operations executed transactionally against VM state.
//!
//! These helpers are kept out of the bytecode dispatch loop so array and
//! character mutation rules can be reviewed as one cohesive subsystem.

use super::{
    BytecodeStorage, BytecodeType, Fiber, NativePlaceView, NativeReady, PlaceDescriptor, Vm,
    VmError, VmValue, array_snapshot_any_rank, character_series, global_unindexed_place,
    indexed_place,
};
use crate::{FindElementCacheKey, FindElementNeedle};

mod array_queries;
mod arrays;
mod variable_access;
mod variables;

pub(super) use array_queries::*;
pub(super) use arrays::*;
pub(super) use variable_access::*;
pub(super) use variables::*;

pub(super) fn script_native_error(kind: crate::ScriptFaultKind, message: String) -> VmError {
    VmError::ScriptFailure(crate::ExecutionFailure::script(
        kind,
        crate::VmFaultCode::TypeMismatch,
        message,
    ))
}
