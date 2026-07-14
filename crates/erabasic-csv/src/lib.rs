//! In-memory loader for the CSV dialect used by the pinned Emuera reference.
//!
//! Application frontends own all filesystem access. They submit relative paths together
//! with either decoded UTF-8 content or the I/O error obtained for that path. This crate
//! is the implemented project-loading boundary, not a concrete frontend or runtime.

mod characters;
mod deferred;
mod diagnostic;
mod extensions;
mod gamebase;
mod input;
mod loader;
mod options;
mod reader;
mod special;
mod tables;
mod variable_size;

pub use deferred::resolve_deferred_indices;
pub use diagnostic::{CsvDiagnostic, CsvDiagnosticCode, CsvDiagnosticSeverity, CsvSourceLocation};
pub use input::{FilePayload, FrontendFile, FrontendIoError, FrontendIoErrorKind, ProjectFiles};
pub use loader::{CsvLoadReport, load_project};
pub use options::CsvLoadOptions;
