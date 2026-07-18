//! Project-level semantic analysis for the pinned Emuera `EraBasic` dialect.
//!
//! The main entry point consumes an in-memory UTF-8 source snapshot and the data
//! produced by `erabasic-csv`. It performs no filesystem I/O and returns a typed,
//! serializable HIR even when individual lines contain recoverable errors.

mod catalog;
mod context;
mod control_flow;
mod declarations;
mod diagnostic;
mod expression;
mod input;
mod options;
mod portability;
mod project;
mod symbols;

pub use catalog::{
    ArgumentConstraint, CallablePortability, CallableSignature, ExtensionCallableKind,
    ExtensionRegistry, InstructionSignature, builtin_callable_portability, builtin_function_names,
    builtin_instruction_names,
};
pub use diagnostic::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, AnalyzerSourceLocation,
};
pub use input::{AnalysisInput, ProjectSource, SourceIoError, SourceIoErrorKind, SourcePayload};
pub use options::{AnalyzerOptions, WarningPolicy};
pub use project::{
    AnalysisReport, AnalyzedProject, ParsedProjectSource, analyze_parsed_project, analyze_project,
};
