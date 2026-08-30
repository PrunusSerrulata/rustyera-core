//! Transactional save, load, and frontend-provided storage orchestration.

mod candidate;
mod loads;
mod menus;
mod operations;
mod resource;

pub(in crate::session) use loads::OwnedReplacementTransaction;
