// This is part of the split RuntimeSession implementation.
#[allow(clippy::wildcard_imports)]
use super::super::*;

struct PreparedTraditionalStart {
    vm: RuntimeVm,
    opaque_extensions: Vec<era_runtime_save::OpaqueSaveExtension>,
    replay_origin: ReplayOrigin,
}

include!("startup/start.rs");
include!("startup/snapshot.rs");
include!("startup/title.rs");
