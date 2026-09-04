//! Transactional save, load, and frontend-provided storage orchestration.

mod candidate;
mod checks;
mod loads;
mod menus;
mod operations;
mod resource;

pub(super) const SAVE_CHECK_CHUNK_BYTES: u32 = 64 * 1024;
