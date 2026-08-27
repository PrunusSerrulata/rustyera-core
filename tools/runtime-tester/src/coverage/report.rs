//! Bounded report assembly: shared static evidence plus every occurrence streamed once.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    io::{self, Write},
    time::{Duration, Instant},
};

use erabasic_analyzer::{builtin_function_names, builtin_instruction_names};
use serde_json::{Value, json};

use super::{
    evidence,
    pipeline::{DiagnosticIndex, Pipeline},
    scan::Appearance,
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
    appearances: &[Appearance],
    user_functions: &BTreeSet<String>,
    pipeline: &Pipeline,
    vm: &evidence::SourceIndex,
    runtime: &evidence::SourceIndex,
) -> Result<String, Box<dyn Error>> {
    let name = metadata["project"].as_str().unwrap_or("?");
    let registry = erabasic_compiler::default_host_registry();
    let functions = builtin_function_names();
    let instructions = builtin_instruction_names();
    let diagnostics = DiagnosticIndex::new(&pipeline.diagnostics);
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
            "migration": evidence::migration(&appearance.api, ""), "dynamic_verification": "not_run"
        }));
        let known = match appearance.form.as_str() {
            "expression" => {
                functions.contains(&appearance.api) || user_functions.contains(&appearance.api)
            }
            "instruction" => instructions.contains(&appearance.api),
            "declaration" | "operator" | "compound_assignment" => true,
            _ => false,
        };
        let analyzer = diagnostics.overlapping(&appearance.path, appearance.span, "analyzer");
        let compiler = diagnostics.overlapping(&appearance.path, appearance.span, "compiler");
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
    output.write_all(b"],\"api_evidence\":")?;
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
            &scan.rows,
            &scan.user_functions,
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
}
