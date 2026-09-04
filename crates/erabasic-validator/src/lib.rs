//! Structural and type validation for HIR and untrusted bytecode artifacts.

mod bytecode;
mod diagnostic;
mod hir;
mod limits;

pub use bytecode::{
    ValidatedArtifact, ValidatedOperandStacks, ValidatedStackState, ValidatedStackToken,
    ValidationContext, validate_bytecode, validate_compiler_output,
};
pub use diagnostic::{ValidationCode, ValidationDiagnostic, ValidationReport};
pub use hir::validate_hir;
pub use limits::ValidationLimits;
