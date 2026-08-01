use erabasic_analyzer::{
    AnalysisInput, AnalyzerDiagnosticCode, AnalyzerOptions, ArgumentConstraint, CallableSignature,
    ExtensionRegistry, InstructionSignature, ProjectSource, SourcePayload, analyze_project,
};
use erabasic_csv::{
    CsvLoadOptions, FilePayload as CsvFilePayload, FrontendFile, ProjectFiles, load_project,
};
use erabasic_hir::{HirArgument, HirStatementKind, SemanticType};
use erabasic_parser::ArgumentStyle;

fn empty_project() -> erabasic_data::ProjectData {
    load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .expect("the default project schema should load")
}

fn source(path: &str, text: &str) -> ProjectSource {
    ProjectSource {
        relative_path: path.into(),
        payload: SourcePayload::Utf8(text.into()),
    }
}

#[path = "semantic_analysis/frontend.rs"]
mod frontend;

#[path = "semantic_analysis/declarations.rs"]
mod declarations;

#[path = "semantic_analysis/syntax.rs"]
mod syntax;

#[path = "semantic_analysis/scopes.rs"]
mod scopes;

#[path = "semantic_analysis/reference.rs"]
mod reference;

#[path = "semantic_analysis/data_and_native.rs"]
mod data_and_native;
