//! Bounded report assembly: shared static evidence plus every occurrence streamed once.

use std::{
    collections::BTreeMap,
    error::Error,
    io::{self, Write},
    time::{Duration, Instant},
};

use erabasic_analyzer::{builtin_function_names, builtin_instruction_names};
use serde_json::{Value, json};

use super::{
    evidence,
    pipeline::{DiagnosticIndex, Pipeline},
    scan::{Appearance, Scan},
};

pub(super) fn object_prefix(output: &mut dyn Write, metadata: &Value) -> io::Result<()> {
    let object = metadata
        .as_object()
        .ok_or_else(|| io::Error::other("report metadata must be an object"))?;
    output.write_all(b"{")?;
    for (index, (key, value)) in object.iter().enumerate() {
        if index != 0 {
            output.write_all(b",")?;
        }
        serde_json::to_writer(&mut *output, key)?;
        output.write_all(b":")?;
        serde_json::to_writer(&mut *output, value)?;
    }
    Ok(())
}

fn statuses(
    appearance: &Appearance,
    known: bool,
    pipeline: &Pipeline,
    analyzer: &[usize],
    compiler: &[usize],
) -> (&'static str, &'static str) {
    let analyzer_status = if !known {
        "unknown"
    } else if appearance.activity != "active_ast" {
        "unverified"
    } else if analyzer.iter().any(|&id| pipeline.diagnostics[id].error) {
        "rejected"
    } else if pipeline.analyzer == "accepted" {
        "accepted_under_reported_audit_options"
    } else {
        "unverified_due_to_project_errors"
    };
    let compiler_status = if compiler
        .iter()
        .any(|&id| pipeline.diagnostics[id].code == "UnsupportedConstruct")
    {
        "compiler_trap"
    } else if compiler.iter().any(|&id| pipeline.diagnostics[id].error) {
        "rejected"
    } else if appearance.activity != "active_ast" {
        "unverified"
    } else if pipeline.compiler == "accepted" {
        "project_compiled_occurrence_not_executed"
    } else {
        "blocked"
    };
    (analyzer_status, compiler_status)
}

pub(super) fn write_project(
    output: &mut dyn Write,
    metadata: Value,
    scan: &Scan,
    pipeline: &Pipeline,
    vm: &evidence::SourceIndex,
    runtime: &evidence::SourceIndex,
) -> Result<String, Box<dyn Error>> {
    crate::watchdog::publish(json!({"phase": "coverage_prepare_report",
        "pending": "build_reference_graph", "appearances_total": scan.rows.len()}))?;
    let Scan {
        rows: appearances,
        user_functions,
        functions: parsed_functions,
        ..
    } = scan;
    let name = metadata["project"].as_str().unwrap_or("?");
    let registry = erabasic_compiler::default_host_registry();
    let functions = builtin_function_names();
    let instructions = builtin_instruction_names();
    let diagnostics = DiagnosticIndex::new(&pipeline.diagnostics);
    let graph = super::graph::Graph::build(appearances, parsed_functions, pipeline);
    let mut counts = BTreeMap::<String, BTreeMap<String, usize>>::new();
    let mut api_evidence = BTreeMap::<String, Value>::new();
    object_prefix(output, &metadata)?;
    output.write_all(b",\"rows\":[")?;
    let mut last_progress = None::<Instant>;
    for (index, appearance) in appearances.iter().enumerate() {
        *counts
            .entry(appearance.api.clone())
            .or_default()
            .entry(appearance.activity.clone())
            .or_default() += 1;
        api_evidence.entry(appearance.api.clone()).or_insert_with(|| json!({
            "registry": evidence::registration(registry.classification(&appearance.api)),
            "vm": vm.vm(&appearance.api), "runtime": runtime.references(&appearance.api),
            "required_service": evidence::required_service(&appearance.api),
            "frontends": {"tui": {"status": "unverified", "reason": "no_runtime_capability_handshake_or_execution_capture"}, "browser": {"status": "unverified", "reason": "no_runtime_capability_handshake_or_execution_capture"}, "tauri": {"status": "unverified", "reason": "no_runtime_capability_handshake_or_execution_capture"}},
            "migration": evidence::migration(&appearance.api, ""), "dynamic_verification": "unverified",
            "capture_refs": super::captures::api_refs(&metadata["execution_captures"], &appearance.api),
            "capture_policy": "bound_bytes_and_source_identity_are_provenance_not_a_behavioral_pass"
        }));
        let known = match appearance.form.as_str() {
            "expression" => {
                functions.contains(&appearance.api) || user_functions.contains(&appearance.api)
            }
            "instruction" => instructions.contains(&appearance.api),
            "declaration"
            | "operator"
            | "compound_assignment"
            | "variable_read"
            | "variable_write"
            | "variable_or_identifier" => true,
            _ => false,
        };
        let valid_span = appearance.span_status == "valid_decoded_utf8";
        let analyzer = if valid_span {
            diagnostics.overlapping(&appearance.path, appearance.span, "analyzer")
        } else {
            Vec::new()
        };
        let compiler = if valid_span {
            diagnostics.overlapping(&appearance.path, appearance.span, "compiler")
        } else {
            Vec::new()
        };
        let (analyzer_status, compiler_status) =
            statuses(appearance, known, pipeline, &analyzer, &compiler);
        // Declaration migration depends on REF/OUT modifiers, not just its API.
        let migration = if matches!(appearance.api.as_str(), "DIM" | "DIMS") {
            Some(evidence::migration(&appearance.api, &appearance.raw))
        } else {
            None
        };
        let row = json!({
            "appearance": appearance, "api_evidence_ref": appearance.api,
            "target_resolution_ref": graph.row_resolution[index],
            "target_source_evidence": if valid_span && appearance.activity == "active_ast" { "active_ast_not_execution" } else { "unverified_candidate" },
            "overload_resolution": "not_inferred_from_arity_consult_analyzer_diagnostics",
            "analyzer": {"status": analyzer_status, "catalog_known": known, "diagnostic_ids": analyzer},
            "compiler": {"status": compiler_status, "diagnostic_ids": compiler},
            "migration_override": migration
        });
        if index != 0 {
            output.write_all(b",")?;
        }
        serde_json::to_writer(&mut *output, &row)?;
        // Throttle the serialization of the complete diagnostic state, not the
        // independent 5-second watchdog. Only completed writes advance progress.
        if last_progress.is_none_or(|time| time.elapsed() >= Duration::from_secs(1))
            || index + 1 == appearances.len()
        {
            output.flush()?;
            crate::watchdog::publish(
                json!({"phase": "coverage_rows", "case": name, "pending": appearance, "rows_completed": index + 1, "rows_total": appearances.len(), "diagnostics": pipeline.diagnostics, "lastFullResponse": null}),
            )?;
            last_progress = Some(Instant::now());
        }
    }
    output.write_all(b"],\"parser_functions\":")?;
    serde_json::to_writer(&mut *output, parsed_functions)?;
    output.write_all(b",\"function_symbols\":")?;
    serde_json::to_writer(&mut *output, &graph.symbols)?;
    output.write_all(b",\"target_resolutions\":")?;
    serde_json::to_writer(&mut *output, &graph.resolutions)?;
    output.write_all(b",\"candidate_sets\":")?;
    serde_json::to_writer(&mut *output, &graph.candidate_sets)?;
    output.write_all(b",\"reference_slices\":")?;
    serde_json::to_writer(&mut *output, &graph.slices)?;
    output.write_all(b",\"api_evidence\":")?;
    serde_json::to_writer(&mut *output, &api_evidence)?;
    output.write_all(b",\"api_counts\":")?;
    serde_json::to_writer(&mut *output, &counts)?;
    output.write_all(b"}")?;
    output.flush()?;
    let mut markdown = format!(
        "## {name}\n\n{} appearances; {} diagnostics. CSV: {}; analyzer: {}; compiler: {}.\n\nLocations, raw syntax, arity, activity and diagnostic IDs are in `rows`; IDs resolve into `pipeline.diagnostics`. Shared registry/VM/runtime/service/frontend evidence is in `api_evidence`. No occurrences are sampled or omitted.\n\n| API | Occurrences by activity | Registry | Classification / batch |\n|---|---|---|---|\n",
        appearances.len(),
        pipeline.diagnostics.len(),
        pipeline.csv,
        pipeline.analyzer,
        pipeline.compiler
    );
    for (api, activity) in &counts {
        let evidence = &api_evidence[api];
        markdown.push_str(&format!(
            "| {} | {} | {} | {} / {} |\n",
            api.replace('|', "\\|"),
            activity
                .iter()
                .map(|(kind, count)| format!("{kind}: {count}"))
                .collect::<Vec<_>>()
                .join("; "),
            evidence["registry"]["classification"]
                .as_str()
                .unwrap_or("?"),
            evidence["migration"]["classification"],
            evidence["migration"]["batch"]
        ));
    }
    markdown.push('\n');
    Ok(markdown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use erabasic_analyzer::AnalyzerOptions;
    use erabasic_csv::{CsvLoadOptions, ProjectFiles};

    #[test]
    fn streamed_report_retains_every_occurrence_and_resolvable_evidence() {
        let sources = vec![(
            "ERB/a.erb".into(),
            "@SYSTEM_TITLE\nUNKNOWN_API 1\nUNKNOWN_API 2\nRETURN\n".into(),
        )];
        let options = AnalyzerOptions::analysis_mode();
        let scan = super::super::scan::scan(&sources, &options);
        let pipeline = super::super::pipeline::analyze(
            &sources,
            &ProjectFiles::default(),
            options,
            CsvLoadOptions::default(),
            true,
        );
        let mut bytes = Vec::new();
        let markdown = write_project(
            &mut bytes,
            json!({"project": "fixture", "pipeline": pipeline}),
            &scan,
            &pipeline,
            &Default::default(),
            &Default::default(),
        )
        .unwrap();
        let report: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(report["rows"].as_array().unwrap().len(), scan.rows.len());
        assert_eq!(report["api_counts"]["UNKNOWN_API"]["active_ast"], 2);
        for row in report["rows"].as_array().unwrap() {
            assert!(
                report["api_evidence"]
                    .get(row["api_evidence_ref"].as_str().unwrap())
                    .is_some()
            );
            for stage in ["analyzer", "compiler"] {
                for id in row[stage]["diagnostic_ids"].as_array().unwrap() {
                    let diagnostic =
                        &report["pipeline"]["diagnostics"][id.as_u64().unwrap() as usize];
                    assert_eq!(diagnostic["stage"], stage);
                    assert_eq!(diagnostic["path"], row["appearance"]["path"]);
                }
            }
        }
        assert!(markdown.contains("UNKNOWN_API"));
        assert!(
            !markdown.contains("UNKNOWN_API 1"),
            "Markdown summarizes APIs, JSON retains source"
        );
    }

    #[test]
    fn report_write_failure_is_not_a_successful_partial_report() {
        struct FailingWriter;
        impl Write for FailingWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("disk full"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(object_prefix(&mut FailingWriter, &json!({"project": "test"})).is_err());
    }

    #[test]
    fn reference_slices_follow_static_edges_but_do_not_promote_dynamic_candidates() {
        let sources = vec![("a.erb".into(), "@SYSTEM_TITLE\nCALL GRAPH_DB_INIT\nRETURN\n@GRAPH_DB_INIT\nPRINTFORM %GETMETH(\"CAN_MOVE_\" + ARGS, 0)%\nRETURN\n@CAN_MOVE_A\n#FUNCTION\nRETURNF 1\n@CAN_MOVE_WRONG\nRETURN\n@UNUSED\nPRINTFORM %EXISTMETH(ARGS)%\nRETURN\n".into())];
        let options = AnalyzerOptions::analysis_mode();
        let scan = super::super::scan::scan(&sources, &options);
        let pipeline = super::super::pipeline::analyze(
            &sources,
            &ProjectFiles::default(),
            options,
            CsvLoadOptions::default(),
            true,
        );
        let graph = super::super::graph::Graph::build(&scan.rows, &scan.functions, &pipeline);
        assert_eq!(graph.symbols.len(), 5);
        let title = &graph.slices[0];
        assert_eq!(title["root_functions"], json!([0]));
        assert_eq!(title["static_reference_closure"], json!([0, 1]));
        let dynamic = graph
            .resolutions
            .iter()
            .find(|entry| entry["target"]["dispatch"] == "dynamic_method")
            .unwrap();
        let candidates =
            &graph.candidate_sets[dynamic["candidate_set_ref"].as_u64().unwrap() as usize];
        assert_eq!(
            candidates.count, 2,
            "wrong-kind candidates must remain visible"
        );
        assert!(
            candidates
                .symbol_ids
                .iter()
                .any(|&id| graph.symbols[id]["name"] == "CAN_MOVE_WRONG")
        );
        assert_eq!(dynamic["candidate_checks"]["required_kind"], "method");
        assert_eq!(dynamic["validity"], "not_proven");
        let lookup = graph
            .resolutions
            .iter()
            .find(|entry| entry["target"]["executes_body"] == false)
            .unwrap();
        let all = &graph.candidate_sets[lookup["candidate_set_ref"].as_u64().unwrap() as usize];
        assert_eq!(all.selector, "all_function_symbols");
        assert_eq!(all.count, 5);
        assert!(all.symbol_ids.is_empty());

        let mut invalid = scan.rows.clone();
        let direct = invalid.iter_mut().find(|row| row.api == "CALL").unwrap();
        direct.span_status = "invalid_parser_span".into();
        direct.activity = "unverified_invalid_parser_span".into();
        let graph = super::super::graph::Graph::build(&invalid, &scan.functions, &pipeline);
        assert_eq!(graph.slices[0]["static_reference_closure"], json!([0]));
        assert!(
            graph.slices[0]["static_edges"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn streamed_gzip_retains_raw_and_encoded_hashes_and_final_flush_errors() {
        use sha2::{Digest as _, Sha256};
        use std::io::Read;
        let raw = "{\"source\":\"日本語\",\"complete\":true}\n".as_bytes();
        let mut writer = super::super::output::ReportOutput::new(Vec::new(), true);
        for piece in raw.chunks(3) {
            writer.write_all(piece).unwrap();
        }
        let (encoded, manifest) = writer.finish().unwrap();
        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(encoded.as_slice())
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, raw);
        assert_eq!(manifest["raw_json"]["bytes"], json!(raw.len()));
        assert_eq!(manifest["stored_file"]["bytes"], json!(encoded.len()));
        assert_eq!(
            manifest["raw_json"]["sha256"],
            format!("{:x}", Sha256::digest(raw))
        );
        assert_eq!(
            manifest["stored_file"]["sha256"],
            format!("{:x}", Sha256::digest(&encoded))
        );
        assert_eq!(
            manifest["raw_json"]["blake3"],
            blake3::hash(raw).to_hex().to_string()
        );
        struct FlushFailure;
        impl Write for FlushFailure {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::other("disk flush failed"))
            }
        }
        let mut writer = super::super::output::ReportOutput::new(FlushFailure, true);
        writer.write_all(raw).unwrap();
        assert!(
            writer.finish().is_err(),
            "failed trailer/flush cannot produce a completion manifest"
        );
    }
}
