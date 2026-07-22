use serde::{Deserialize, Serialize};

use crate::Digest;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct FormatVersion {
    pub major: u16,
    pub minor: u16,
}

pub const CONTAINER_VERSION: FormatVersion = FormatVersion {
    major: 15,
    minor: 0,
};
pub const ISA_VERSION: FormatVersion = FormatVersion { major: 7, minor: 0 };
pub const COMPILER_ABI_VERSION: u32 = 30;
pub const NATIVE_ABI_VERSION: u32 = 15;
pub const HOST_ABI_VERSION: u32 = 12;
pub const VM_ABI_VERSION: u32 = 14;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProgramVersion {
    pub vm_abi: u32,
    pub host_abi: u32,
    pub execution_id: Digest,
}

impl ProgramVersion {
    /// Snapshots are executable-state artifacts, so all three fields must match.
    #[must_use]
    pub fn is_snapshot_compatible_with(self, snapshot: Self) -> bool {
        self == snapshot
    }
}
