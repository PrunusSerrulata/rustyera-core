//! Complex native operations executed transactionally against VM state.
//!
//! These helpers are kept out of the bytecode dispatch loop so array and
//! character mutation rules can be reviewed as one cohesive subsystem.

use super::{
    BytecodeStorage, BytecodeType, Fiber, NativePlaceView, NativeReady, PlaceDescriptor, Vm,
    VmError, VmValue, array_snapshot_any_rank, character_series, global_unindexed_place,
    indexed_place,
};

mod arrays;
mod variables;

pub(super) use arrays::*;
pub(super) use variables::*;
