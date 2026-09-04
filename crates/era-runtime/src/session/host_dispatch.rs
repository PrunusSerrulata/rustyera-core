//! Translation of VM host requests into runtime-owned semantic operations.

// This is one part of the same split `RuntimeSession` implementation.
#[allow(clippy::wildcard_imports)]
use super::*;

mod control;
mod graphics;
mod input_extensions;
mod presentation;
mod services;
mod sql;
mod storage;

pub(in crate::session) use services::immediate_tag_split_targets;

include!("host_dispatch/preparation.rs");
include!("host_dispatch/routing.rs");
include!("host_dispatch/html_support.rs");
