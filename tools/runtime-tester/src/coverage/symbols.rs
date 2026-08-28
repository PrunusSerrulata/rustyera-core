//! Compact authoritative symbol views; never serialize HIR bodies or deferred source text.

use erabasic_analyzer::AnalyzedProject;
use erabasic_ast::Span;
use erabasic_data::{ProjectData, VariableId};
use serde_json::{Value, json};

fn projection_progress(kind: &str, completed: usize, total: usize) {
    if completed.is_multiple_of(256) || completed == total {
        crate::watchdog::publish_or_exit(json!({"phase": "coverage_symbol_projection",
            "pending": kind, "symbols_completed": completed, "symbols_total": total}));
    }
}

pub(super) fn data(data: &ProjectData, phase: &str) -> Value {
    let users = data
        .schema
        .variables
        .values()
        .filter(|variable| matches!(variable.id, VariableId::User(_)))
        .map(|variable| json!({"name": variable.id.name(), "schema": variable}))
        .collect::<Vec<_>>();
    let catalog = &data.static_data.deferred_indices;
    let groups = catalog.groups.iter().map(|(stem, files)| json!({
        "stem": stem,
        "files": files.iter().map(|file| json!({"path": file.relative_path,
            "decoded_utf8_blake3": blake3::hash(file.content.as_bytes()).to_hex().to_string(),
            "alias_source": file.aliases.as_ref().map(|alias| json!({"path": alias.relative_path,
                "decoded_utf8_blake3": blake3::hash(alias.content.as_bytes()).to_hex().to_string()}))})).collect::<Vec<_>>(),
        "resolution": if catalog.resolved.contains_key(stem) { "resolved_in_reported_phase" } else { "unresolved_no_registered_dimension_or_disabled_by_policy" }
    })).collect::<Vec<_>>();
    let indices = catalog.resolved.iter().map(|(stem, table)| {
        // Resolved table names include the ERD data dimension; schema names do not.
        let (variable_name, dimension) = table.variable_name.rsplit_once('@')
            .and_then(|(name, suffix)| suffix.parse::<usize>().ok().map(|dimension| (name, dimension)))
            .unwrap_or((&table.variable_name, 1));
        let schema = data.schema.variable(variable_name);
        json!({"stem": stem, "variable_name": variable_name, "data_dimension": dimension,
            "data_dimension_length": schema.and_then(|schema| schema.dimensions.get(dimension.saturating_sub(1))),
            "character_storage_index_is_not_an_erd_dimension": true,
            "signed_name_lookup": table.entries, "reverse_names_in_primary_then_insertion_precedence": table.canonical_names,
            "entry_origin": "merged_lookup_not_reclassified_as_alias_from_spelling",
            "array_access_bounds": "checked_at_execution_not_proven_by_index_lookup"})
    }).collect::<Vec<_>>();
    json!({"phase": phase, "execution_status": "unverified", "user_variables": users,
        "user_variable_order": data.schema.user_variable_order, "index_sources": groups,
        "resolved_user_indices": indices, "builtin_name_tables": data.static_data.name_tables})
}

pub(super) fn analyzed(project: &AnalyzedProject, sources: &[(String, String)]) -> Value {
    projection_progress(
        "source_and_variable_indices",
        0,
        project.program.variables.len(),
    );
    let by_source = project
        .program
        .sources
        .iter()
        .map(|source| (source.id, source))
        .collect::<std::collections::BTreeMap<_, _>>();
    let by_path = sources
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let by_variable = project
        .program
        .variables
        .iter()
        .map(|variable| (variable.id, variable))
        .collect::<std::collections::BTreeMap<_, _>>();
    let source_location = |source_id, span: Span| {
        let source = by_source.get(&source_id).copied();
        let valid = source
            .and_then(|source| by_path.get(source.relative_path.as_str()))
            .is_some_and(|text| text.get(span.start..span.end).is_some());
        json!({"source_id": source_id, "path": source.map(|source| &source.relative_path), "span": span,
            "span_status": if valid { "valid_decoded_utf8" } else { "invalid_or_unavailable_source_span" },
            "decoded_utf8_blake3": source.map(|source| blake3::Hash::from(source.content_hash).to_hex().to_string())})
    };
    let variables = project.program.variables.iter().enumerate().map(|(index, variable)| {
        let value = json!({
        "id": variable.id, "name": variable.name, "value_type": variable.value_type,
        "dimensions": variable.dimensions, "storage": variable.storage, "persistence": variable.persistence,
        "reference": variable.reference, "mutable": variable.mutable, "static_lifetime": variable.static_lifetime,
        "scope": variable.scope, "owner": variable.owner,
        "source": variable.location.map(|location| source_location(location.source, location.span))
        });
        projection_progress("variables", index + 1, project.program.variables.len());
        value
    }).collect::<Vec<_>>();
    projection_progress("functions", 0, project.program.functions.len());
    let functions = project.program.functions.iter().enumerate().map(|(index, function)| {
        let value = json!({
        "id": function.id, "name": function.name, "kind": function.kind, "return_type": function.return_type,
        "definition_order": function.definition_order, "event_attributes": function.event_attributes,
        "source": source_location(function.location.source, function.location.span),
        "parameters": function.parameters.iter().map(|parameter| {
            let variable = by_variable.get(&parameter.target.variable).copied();
            json!({"variable": parameter.target.variable, "name": variable.map(|variable| &variable.name),
                "value_type": parameter.target.value_type, "target_index_count": parameter.target.indices.len(),
                "reference": variable.map(|variable| variable.reference), "dimensions": variable.map(|variable| &variable.dimensions),
                "optional": parameter.default.is_some(), "default_type": parameter.default.as_ref().map(|default| default.value_type),
                "default_constant": parameter.default.as_ref().and_then(|default| default.constant.as_ref())})
        }).collect::<Vec<_>>(),
        "reachability": "retained_even_if_uncalled_not_execution_proof"
        });
        projection_progress("functions", index + 1, project.program.functions.len());
        value
    }).collect::<Vec<_>>();
    json!({"phase": "analyzer_project", "status": "resolved_symbols_diagnostics_still_apply",
        "data": data(&project.data, "analyzer_project"), "variables": variables, "functions": functions})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "112727-function symbol projection under the production watchdog"]
    fn tw_scale_symbol_projection_reports_completed_symbols() {
        crate::watchdog::supervise("coverage_symbol_scale", run_symbol_scale).unwrap();
    }

    fn run_symbol_scale() -> Result<(), Box<dyn std::error::Error>> {
        const COUNT: usize = 112_727;
        let sources = vec![("ERB/symbols.erb".into(), "@SYSTEM_TITLE\nRETURN\n".into())];
        let mut project = erabasic_analyzer::analyze_project(
            erabasic_analyzer::AnalysisInput {
                project_data: erabasic_csv::load_project(&Default::default(), &Default::default())
                    .data
                    .expect("default CSV data"),
                sources: sources
                    .iter()
                    .map(
                        |(path, text): &(String, String)| erabasic_analyzer::ProjectSource {
                            relative_path: path.clone(),
                            payload: erabasic_analyzer::SourcePayload::Utf8(text.clone()),
                        },
                    )
                    .collect(),
            },
            &erabasic_analyzer::AnalyzerOptions::analysis_mode(),
            &Default::default(),
        )
        .project
        .expect("minimal analyzed project");
        let sample = project.program.functions[0].clone();
        project.program.functions.clear();
        for index in 0..COUNT {
            let mut function = sample.clone();
            function.name = format!("SCALE_{index}");
            project.program.functions.push(function);
            projection_progress("stress_allocate_functions", index + 1, COUNT);
        }
        let started = std::time::Instant::now();
        let mut value = analyzed(&project, &sources);
        let seconds = started.elapsed().as_secs_f64();
        let functions = value["functions"].as_array_mut().unwrap();
        assert_eq!(functions.len(), COUNT);
        assert_eq!(functions[0]["name"], "SCALE_0");
        assert_eq!(functions[COUNT - 1]["name"], format!("SCALE_{}", COUNT - 1));
        for (index, function) in functions.drain(..).enumerate() {
            assert_eq!(function["name"], format!("SCALE_{index}"));
            assert!(function.get("lines").is_none());
            drop(function);
            projection_progress("stress_verify_and_release_symbols", index + 1, COUNT);
        }
        while !project.program.functions.is_empty() {
            project
                .program
                .functions
                .truncate(project.program.functions.len().saturating_sub(256));
            projection_progress(
                "stress_release_hir",
                COUNT - project.program.functions.len(),
                COUNT,
            );
        }
        let summary = json!({"phase": "symbol_stress_complete", "functions": COUNT,
            "projection_seconds": seconds, "all_names_verified": true, "hir_bodies_omitted": true});
        crate::watchdog::publish(summary.clone())?;
        println!("{summary}");
        Ok(())
    }
}
