//! Source citations are evidence to inspect, never proof of an executed implementation.

use std::{collections::BTreeMap, fs, io, path::Path};

use erabasic_compiler::ExecutionBinding;
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize)]
pub(super) struct SourceReference {
    path: String,
    line: usize,
}

#[derive(Default, Serialize)]
pub(super) struct SourceIndex {
    pub files: BTreeMap<String, String>,
    #[serde(skip)]
    names: BTreeMap<String, Vec<SourceReference>>,
}

impl SourceIndex {
    pub fn collect(root: &Path, relative: &str) -> io::Result<Self> {
        let mut result = Self::default();
        result.walk(root, &root.join(relative))?;
        Ok(result)
    }

    fn walk(&mut self, root: &Path, path: &Path) -> io::Result<()> {
        if path.is_dir() {
            let mut entries = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
            entries.sort_by_key(fs::DirEntry::file_name);
            for entry in entries {
                let kind = entry.file_type()?;
                if kind.is_symlink() || entry.file_name() == "tests" {
                    continue;
                }
                self.walk(root, &entry.path())?;
            }
        } else if path.extension().is_some_and(|extension| extension == "rs")
            && path.file_name().is_some_and(|name| name != "tests.rs")
        {
            crate::watchdog::publish(
                json!({"phase": "source_evidence", "case": path, "pending": "read_source", "files_completed": self.files.len(), "lastFullResponse": null}),
            )?;
            let content = fs::read_to_string(path)?;
            let relative = path
                .strip_prefix(root)
                .map_err(io::Error::other)?
                .to_string_lossy()
                .replace('\\', "/");
            self.files.insert(
                relative.clone(),
                blake3::hash(content.as_bytes()).to_hex().to_string(),
            );
            for (line_number, line) in content.lines().enumerate() {
                if line.trim() == "#[cfg(test)]" {
                    break;
                }
                // Deliberately only an index of exact quoted names. A textual hit is
                // not a dispatch arm, and absence here does not prove unsupported.
                for token in line.split('"').skip(1).step_by(2) {
                    if !token.is_empty()
                        && token
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    {
                        self.names
                            .entry(token.to_ascii_uppercase())
                            .or_default()
                            .push(SourceReference {
                                path: relative.clone(),
                                line: line_number + 1,
                            });
                    }
                }
            }
        }
        Ok(())
    }

    pub fn references(&self, name: &str) -> Value {
        json!({"status": "unverified", "evidence_kind": "exact_string_source_reference_not_dispatch_proof", "locations": self.names.get(name).map(Vec::as_slice).unwrap_or_default()})
    }

    pub fn vm(&self, name: &str) -> Value {
        self.references(name)
    }
}

pub(super) fn registration(binding: Option<&ExecutionBinding>) -> Value {
    match binding {
        Some(ExecutionBinding::ExpressionMethod { result }) => {
            json!({"classification": "ExpressionMethod", "result": result, "lowering": "typed_resolve_capture_invoke", "implementation_verified": false})
        }
        Some(ExecutionBinding::Native(contract)) => {
            json!({"classification": "Native", "contract": contract, "implementation_verified": false})
        }
        Some(ExecutionBinding::BitArray) => {
            json!({"classification": "BitArray", "lowering": "staged_vm_array_backing", "implementation_verified": false})
        }
        Some(ExecutionBinding::ArrayMatch) => {
            json!({"classification": "ArrayMatch", "lowering": "staged_vm_array_scan", "implementation_verified": false})
        }
        Some(ExecutionBinding::Host(binding)) => {
            json!({"classification": "Host", "namespace": binding.namespace, "operation": binding.name, "abi_version": binding.abi_version, "capability": binding.capability, "contract": binding.contract, "implementation_verified": false})
        }
        Some(ExecutionBinding::Unsupported { reason }) => {
            json!({"classification": "Unsupported", "reason": reason, "lowering_when_reached": "UnsupportedConstruct diagnostic and Trap; actual compilation is reported separately"})
        }
        Some(ExecutionBinding::UnsupportedCapability { capability, reason }) => {
            json!({"classification": "UnsupportedCapability", "capability": capability, "reason": reason, "lowering_when_reached": "MissingCapability diagnostic; no executable artifact or runtime Trap"})
        }
        None => json!({"classification": "Unregistered", "implementation_verified": false}),
    }
}

pub(super) fn required_service(api: &str) -> Option<Value> {
    use era_runtime_protocol::*;
    let (kind, operation, version) = match api {
        "GETKEY" | "GETKEYTRIGGERED" => (
            ServiceKind::InputState,
            GET_KEY_STATE_OPERATION,
            GET_KEY_STATE_OPERATION_VERSION,
        ),
        "MOUSEX" | "MOUSEY" | "MOUSEB" => (
            ServiceKind::InputState,
            POINTER_STATE_OPERATION,
            POINTER_STATE_OPERATION_VERSION,
        ),
        "HTML_STRINGLEN" => (
            ServiceKind::PresentationQuery,
            HTML_STRING_LEN_OPERATION,
            HTML_STRING_LEN_OPERATION_VERSION,
        ),
        "HTML_SUBSTRING" => (
            ServiceKind::PresentationQuery,
            HTML_SUBSTRING_OPERATION,
            HTML_SUBSTRING_OPERATION_VERSION,
        ),
        "HTML_STRINGLINES" => (
            ServiceKind::PresentationQuery,
            HTML_STRING_LINES_OPERATION,
            HTML_STRING_LINES_OPERATION_VERSION,
        ),
        "HTML_GETPRINTEDSTR" => (
            ServiceKind::PresentationQuery,
            HTML_GET_PRINTED_STR_OPERATION,
            HTML_GET_PRINTED_STR_OPERATION_VERSION,
        ),
        "GGETCOLOR" => (
            ServiceKind::Canvas,
            SAMPLE_CANVAS_PIXEL_OPERATION,
            SAMPLE_CANVAS_PIXEL_OPERATION_VERSION,
        ),
        "SQL_CONNECT"
        | "SQL_DISCONNECT"
        | "SQL_EXECUTE_NONQUERY"
        | "SQL_P_EXECUTE_NONQUERY"
        | "SQL_EXECUTE_SCALAR_LONG"
        | "SQL_EXECUTE_SCALAR_STRING"
        | "SQL_P_EXECUTE_SCALAR_LONG"
        | "SQL_P_EXECUTE_SCALAR_STRING"
        | "SQL_EXECUTE_READER"
        | "SQL_P_EXECUTE_READER"
        | "SQL_READER_READ"
        | "SQL_READER_GET_LONG"
        | "SQL_READER_GET_STRING"
        | "SQL_READER_ISNULL"
        | "SQL_READER_CLOSE"
        | "SQL_IMPORT_MAP_XML" => (ServiceKind::Sql, SQL_OPERATION, SQL_OPERATION_VERSION),
        _ => return None,
    };
    Some(
        json!({"kind": kind, "operation": operation, "version": version, "mapping_status": "source_mapped_not_executed"}),
    )
}

pub(super) fn migration(api: &str, raw: &str) -> Value {
    let (classification, batch, fixture) = match api {
        "GETMETH" | "GETMETHS" | "EXISTMETH" => ("S03", 1, None),
        "DT_COLUMN_OPTIONS" => ("S12", 1, None),
        "PRINTC" | "PRINTLC" | "PRINTFORMC" | "PRINTFORMLC" | "HTML_PRINTC" | "HTML_PRINTLC"
        | "GETLINEY" => ("C04", 4, Some("printc")),
        "RAND" | "RANDOMIZE" | "INITRAND" | "DUMPRAND" => ("D11", 2, Some("rng")),
        "GETKEY" | "GETKEYTRIGGERED" => ("D12", 2, Some("getkey")),
        "TOINT" => ("D12", 2, Some("toint")),
        "OPERATOR_ADD" | "OPERATOR_SUBTRACT" | "OPERATOR_MULTIPLY" | "OPERATOR_DIVIDE"
        | "OPERATOR_MODULO" => ("D07", 2, Some("arithmetic")),
        "CALL" | "CALLFORM" | "TRYCALL" | "TRYCALLFORM" => ("D06", 2, Some("extra-args")),
        "HTML_STRINGLEN" | "HTML_SUBSTRING" | "HTML_STRINGLINES" | "MOUSEX" | "MOUSEY"
        | "MOUSEB" | "GGETCOLOR" => ("S04", 1, None),
        "REF" | "REFS" | "REFF" | "ARGLEN" | "VARIADIC" => ("D03", 6, Some("ref")),
        "DIM" | "DIMS"
            if raw.split_whitespace().any(|word| {
                word.eq_ignore_ascii_case("REF") || word.eq_ignore_ascii_case("OUT")
            }) =>
        {
            ("D03", 6, Some("ref"))
        }
        name if name.starts_with("SQL_") => ("C01", 3, None),
        name if name.starts_with("UNCHECKED_") => ("S11", 2, Some("arithmetic")),
        _ => {
            return json!({"classification": null, "batch": null, "fixture": null, "status": "unmapped_requires_triage"});
        }
    };
    json!({"classification": classification, "batch": batch, "fixture": fixture, "status": "scope_mapping_not_implementation_or_execution_evidence"})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_registration_is_not_implementation_evidence() {
        let registry = erabasic_compiler::default_host_registry();
        let evidence = registration(registry.classification("DT_COLUMN_OPTIONS"));
        assert_eq!(evidence["classification"], "Native");
        assert_eq!(evidence["implementation_verified"], false);
        assert_eq!(
            SourceIndex::default().vm("DT_COLUMN_OPTIONS")["status"],
            "unverified"
        );
    }

    #[test]
    fn unmapped_service_is_not_misreported_as_missing() {
        assert!(required_service("SOME_FUTURE_API").is_none());
        assert_eq!(
            required_service("GETKEY").unwrap()["operation"],
            "get_key_state"
        );
        assert_eq!(migration("DT_COLUMN_OPTIONS", "")["classification"], "S12");
        for api in ["MOUSEX", "MOUSEY", "MOUSEB"] {
            let service = required_service(api).unwrap();
            assert_eq!(
                service["operation"],
                era_runtime_protocol::POINTER_STATE_OPERATION
            );
            assert_eq!(
                service["version"],
                json!(era_runtime_protocol::POINTER_STATE_OPERATION_VERSION)
            );
        }
        assert_eq!(
            required_service("HTML_STRINGLEN").unwrap()["version"],
            json!(era_runtime_protocol::HTML_STRING_LEN_OPERATION_VERSION)
        );
        let sql = required_service("SQL_P_EXECUTE_NONQUERY").unwrap();
        assert_eq!(sql["kind"], json!(era_runtime_protocol::ServiceKind::Sql));
        assert_eq!(sql["operation"], era_runtime_protocol::SQL_OPERATION);
        assert_eq!(migration("SQL_CONNECT", "")["classification"], "C01");
    }
}
