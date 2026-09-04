#[allow(clippy::wildcard_imports)]
use super::*;
use crate::HostWrite;

#[derive(Clone, Copy)]
struct ResolvedPlaceWrite {
    generation: GenerationId,
    key: SymbolKey,
    storage: BytecodeStorage,
    mutable: bool,
    owner: Option<SymbolKey>,
}

mod access;
mod host_writes;
mod identities;
mod runtime_variables;
mod writes;
