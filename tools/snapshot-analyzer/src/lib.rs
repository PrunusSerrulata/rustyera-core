//! Human-readable inspection of complete `RustyEra` runtime snapshots.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use era_runtime::{RuntimeOptions, RuntimeSnapshotInspection, inspect_runtime_snapshot};
use serde_json::Value;

/// Command-line usage shown by the standalone analyzer.
pub const USAGE: &str = "Usage: rustyera-snapshot-analyzer [--json] <SNAPSHOT>";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalyzeOptions {
    pub input: PathBuf,
    pub json: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Help,
    Analyze(AnalyzeOptions),
}

#[derive(Debug)]
pub struct AnalyzeError {
    message: String,
}

impl AnalyzeError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for AnalyzeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AnalyzeError {}

/// Parse command-line arguments after the executable name.
///
/// # Errors
///
/// Returns an error for unknown options or when the snapshot path is not the
/// sole positional argument.
pub fn parse_arguments(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Command, AnalyzeError> {
    let mut json = false;
    let mut positional = Vec::new();
    let mut options = true;
    for argument in arguments {
        if options && argument == "--" {
            options = false;
        } else if options && (argument == "--help" || argument == "-h") {
            return Ok(Command::Help);
        } else if options && argument == "--json" {
            json = true;
        } else if options && argument.to_string_lossy().starts_with('-') {
            return Err(AnalyzeError::new(format!(
                "unknown option {}; {USAGE}",
                argument.to_string_lossy()
            )));
        } else {
            positional.push(PathBuf::from(argument));
        }
    }
    if positional.len() != 1 {
        return Err(AnalyzeError::new(USAGE));
    }
    Ok(Command::Analyze(AnalyzeOptions {
        input: positional.remove(0),
        json,
    }))
}

/// Read and inspect one complete runtime snapshot.
///
/// # Errors
///
/// Returns an error if the file is unreadable, exceeds the runtime's default
/// transfer limit, or is not a structurally valid complete runtime snapshot.
pub fn analyze_file(path: &Path) -> Result<RuntimeSnapshotInspection, AnalyzeError> {
    let maximum_bytes = usize::try_from(RuntimeOptions::default().limits.maximum_transfer_bytes)
        .unwrap_or(usize::MAX);
    let metadata = fs::metadata(path).map_err(|error| {
        AnalyzeError::new(format!("cannot inspect input {}: {error}", path.display()))
    })?;
    if metadata.len() > u64::try_from(maximum_bytes).unwrap_or(u64::MAX) {
        return Err(AnalyzeError::new(format!(
            "snapshot exceeds the {maximum_bytes} byte analysis limit"
        )));
    }
    let bytes = fs::read(path).map_err(|error| {
        AnalyzeError::new(format!("cannot read input {}: {error}", path.display()))
    })?;
    inspect_runtime_snapshot(&bytes, maximum_bytes)
        .map_err(|error| AnalyzeError::new(format!("cannot decode runtime snapshot: {error}")))
}

/// Render the complete inspection as deterministic, pretty-printed JSON.
///
/// # Errors
///
/// Returns an error if the inspection cannot be serialized.
pub fn render_json(inspection: &RuntimeSnapshotInspection) -> Result<String, AnalyzeError> {
    let mut output = serde_json::to_string_pretty(inspection)
        .map_err(|error| AnalyzeError::new(format!("cannot render JSON: {error}")))?;
    output.push('\n');
    Ok(output)
}

/// Render every inspection leaf as deterministic sectioned text.
///
/// # Errors
///
/// Returns an error if the inspection cannot be converted to its shared JSON tree.
pub fn render_text(inspection: &RuntimeSnapshotInspection) -> Result<String, AnalyzeError> {
    let value = serde_json::to_value(inspection)
        .map_err(|error| AnalyzeError::new(format!("cannot render text: {error}")))?;
    let fields = value
        .as_object()
        .ok_or_else(|| AnalyzeError::new("runtime snapshot inspection is not an object"))?;
    let mut output = String::new();
    if let Some(version) = fields.get("inspection_schema_version") {
        append_value(&mut output, "inspection_schema_version", version);
    }
    for section in ["container", "payload", "validation"] {
        output.push('\n');
        output.push('[');
        output.push_str(section);
        output.push_str("]\n");
        if let Some(value) = fields.get(section) {
            append_tree(&mut output, "", value);
        }
    }
    Ok(output)
}

fn append_tree(output: &mut String, path: &str, value: &Value) {
    match value {
        Value::Object(fields) if fields.is_empty() => append_value(output, path, value),
        Value::Object(fields) => {
            for (name, value) in fields {
                let child = join_path(path, name);
                append_tree(output, &child, value);
            }
        }
        Value::Array(values) if values.is_empty() => append_value(output, path, value),
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                let child = format!("{path}[{index}]");
                append_tree(output, &child, value);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            append_value(output, path, value);
        }
    }
}

fn join_path(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.into()
    } else {
        format!("{parent}.{child}")
    }
}

fn append_value(output: &mut String, path: &str, value: &Value) {
    output.push_str(path);
    output.push_str(" = ");
    output.push_str(&serde_json::to_string(value).expect("JSON values are serializable"));
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use era_runtime::{
        RUNTIME_SNAPSHOT_INSPECTION_SCHEMA_VERSION, RuntimeSnapshotContainerInspection,
        RuntimeSnapshotValidation,
    };

    fn inspection() -> RuntimeSnapshotInspection {
        RuntimeSnapshotInspection {
            inspection_schema_version: RUNTIME_SNAPSHOT_INSPECTION_SCHEMA_VERSION,
            container: RuntimeSnapshotContainerInspection {
                magic: "RERARTS\\0".into(),
                format_version: 17,
                file_bytes: 100,
                compressed_payload_bytes: 40,
                uncompressed_payload_bytes: 80,
                payload_blake3: "abc".into(),
            },
            payload: serde_json::json!({
                "empty": [],
                "origin": "Diagnosis",
                "nested": {"value": 7},
            }),
            validation: RuntimeSnapshotValidation {
                runtime_container: "valid".into(),
                embedded_container: "valid".into(),
                artifact_compatibility: "not_checked".into(),
                restore_semantics: "not_checked".into(),
            },
        }
    }

    #[test]
    fn arguments_accept_one_path_and_optional_json() {
        assert_eq!(
            parse_arguments([OsString::from("--json"), OsString::from("state.bin")]).unwrap(),
            Command::Analyze(AnalyzeOptions {
                input: PathBuf::from("state.bin"),
                json: true,
            })
        );
        assert_eq!(
            parse_arguments([OsString::from("--"), OsString::from("-state.bin")]).unwrap(),
            Command::Analyze(AnalyzeOptions {
                input: PathBuf::from("-state.bin"),
                json: false,
            })
        );
        assert_eq!(
            parse_arguments([OsString::from("-h")]).unwrap(),
            Command::Help
        );
        assert!(parse_arguments(Vec::<OsString>::new()).is_err());
        assert!(parse_arguments([OsString::from("one"), OsString::from("two")]).is_err());
        assert!(parse_arguments([OsString::from("--unknown")]).is_err());
    }

    #[test]
    fn text_and_json_render_the_same_complete_tree() {
        let inspection = inspection();
        let text = render_text(&inspection).unwrap();
        assert!(text.contains("[container]\n"));
        assert!(text.contains("magic = \"RERARTS\\\\0\"\n"));
        assert!(text.contains("empty = []\n"));
        assert!(text.contains("nested.value = 7\n"));
        assert!(text.contains("artifact_compatibility = \"not_checked\"\n"));

        let json = render_json(&inspection).unwrap();
        assert!(json.ends_with('\n'));
        assert_eq!(
            serde_json::from_str::<Value>(&json).unwrap(),
            serde_json::to_value(inspection).unwrap()
        );
    }
}
