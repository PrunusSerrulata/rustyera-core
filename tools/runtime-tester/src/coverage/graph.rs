//! Conservative source-reference graph. No path in this graph is an execution claim.

use super::{
    evidence,
    pipeline::{DiagnosticIndex, Pipeline},
    scan::{Appearance, ParsedFunction},
    targets::{Segment, Target},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::{Duration, Instant},
};

pub(super) struct Graph {
    pub symbols: Vec<Value>,
    pub resolutions: Vec<Value>,
    pub candidate_sets: Vec<CandidateSet>,
    pub row_resolution: Vec<Option<usize>>,
    pub slices: Vec<Value>,
}

#[derive(Serialize)]
pub(super) struct CandidateSet {
    pub selector: &'static str,
    pub count: usize,
    pub symbol_ids: Vec<usize>,
}

impl CandidateSet {
    fn indices(&self) -> Box<dyn Iterator<Item = usize> + '_> {
        if self.selector == "all_function_symbols" {
            Box::new(0..self.count)
        } else {
            Box::new(self.symbol_ids.iter().copied())
        }
    }
}

fn same_name(left: &str, right: &str, ignore_case: bool) -> bool {
    if ignore_case {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn symbols(functions: &[ParsedFunction], pipeline: &Pipeline) -> Vec<Value> {
    let analyzed = pipeline.symbols["functions"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let key = |path: &str, start: u64, name: &str| {
        (
            path.to_owned(),
            start,
            if pipeline.analyzer_options.ignore_case {
                name.to_ascii_uppercase()
            } else {
                name.to_owned()
            },
        )
    };
    let mut by_location = BTreeMap::new();
    for (index, symbol) in analyzed.iter().enumerate() {
        if let (Some(path), Some(start), Some(name)) = (
            symbol["source"]["path"].as_str(),
            symbol["source"]["span"]["start"].as_u64(),
            symbol["name"].as_str(),
        ) {
            by_location.entry(key(path, start, name)).or_insert(index);
        }
    }
    let mut matched = BTreeSet::new();
    let mut result = Vec::new();
    for function in functions {
        let resolved = by_location
            .get(&key(
                &function.path,
                function.span.start as u64,
                &function.name,
            ))
            .map(|&index| (index, &analyzed[index]));
        if let Some((index, _)) = resolved {
            matched.insert(index);
        }
        let attributes = function
            .attributes
            .iter()
            .map(|name| name.to_ascii_uppercase())
            .collect::<BTreeSet<_>>();
        let (kind, returns) = if attributes.contains("FUNCTIONS") {
            ("method", "string")
        } else if attributes.contains("FUNCTION") {
            ("method", "integer")
        } else {
            ("unresolved", "unresolved")
        };
        result.push(json!({"id": result.len(), "name": function.name, "parser_function_id": function.id,
            "source": {"path": function.path, "span": function.span, "span_status": function.span_status, "decoded_utf8_blake3": function.decoded_utf8_blake3},
            "phase": if resolved.is_some() { "analyzer_project" } else { "parser_declaration_only" },
            "kind": resolved.map_or(json!(kind), |(_, symbol)| symbol["kind"].clone()),
            "return_type": resolved.map_or(json!(returns), |(_, symbol)| symbol["return_type"].clone()),
            "parameters": resolved.map_or_else(|| json!(function.parameters), |(_, symbol)| symbol["parameters"].clone()),
            "analyzer_symbol": resolved.map(|(_, symbol)| symbol), "execution_status": "unverified"}));
    }
    for (index, symbol) in analyzed
        .iter()
        .enumerate()
        .filter(|(index, _)| !matched.contains(index))
    {
        result.push(json!({"id": result.len(), "name": symbol["name"], "parser_function_id": null,
            "phase": "analyzer_project", "kind": symbol["kind"], "return_type": symbol["return_type"], "parameters": symbol["parameters"],
            "source": symbol["source"], "analyzer_symbol_index": index, "execution_status": "unverified"}));
    }
    result
}

fn candidates(
    target: &Target,
    symbols: &[Value],
    names: &BTreeMap<String, Vec<usize>>,
    ignore_case: bool,
) -> CandidateSet {
    if target.namespace != "function" {
        return CandidateSet {
            selector: "explicit_symbol_ids",
            count: 0,
            symbol_ids: Vec::new(),
        };
    }
    if target.pattern.segments.iter().all(|segment| match segment {
        Segment::Unknown(_) => true,
        Segment::Literal(value) => value.is_empty(),
    }) && target.pattern.exact().is_none()
    {
        return CandidateSet {
            selector: "all_function_symbols",
            count: symbols.len(),
            symbol_ids: Vec::new(),
        };
    }
    let exact = target.pattern.exact().map(|name| {
        if ignore_case {
            name.to_ascii_uppercase()
        } else {
            name
        }
    });
    let ids = if let Some(name) = exact {
        names.get(&name).cloned().unwrap_or_default()
    } else {
        symbols
            .iter()
            .enumerate()
            .filter(|(_, symbol)| {
                symbol["name"]
                    .as_str()
                    .is_some_and(|name| target.pattern.matches(name, ignore_case))
            })
            .map(|(id, _)| id)
            .collect()
    };
    let count = ids.len();
    if count == symbols.len() {
        CandidateSet {
            selector: "all_function_symbols",
            count,
            symbol_ids: Vec::new(),
        }
    } else {
        CandidateSet {
            selector: "explicit_symbol_ids",
            count,
            symbol_ids: ids,
        }
    }
}

fn resolution(target: &Target, candidate_set: usize, candidate_count: usize) -> Value {
    let mut reasons = target
        .pattern
        .segments
        .iter()
        .filter_map(|segment| match segment {
            Segment::Unknown(reason) => Some(reason.clone()),
            Segment::Literal(_) => None,
        })
        .collect::<BTreeSet<_>>();
    if target.namespace != "function" {
        reasons.insert("label_target_requires_function_local_label_resolution".into());
    }
    if candidate_count == 0 {
        reasons.insert("no_symbol_candidate_or_incomplete_symbol_inventory".into());
    }
    if !target.executes_body {
        reasons.insert("existmeth_resolution_does_not_execute_target_body".into());
    }
    json!({"target": target, "exact_name": target.pattern.exact(), "candidate_set_ref": candidate_set,
        "name_matching": if target.dispatch.starts_with("dynamic_") { "current_vm_ascii_case_insensitive_function_lookup" } else { "analyzer_ignore_case_option" },
        "candidate_checks": {"required_kind": if target.dispatch == "dynamic_method" { Some("method") } else { None },
            "required_return_type": target.expected_return,
            "kind_and_type_policy": "wrong_kind_and_return_type_symbols_are_retained_for_diagnostics",
            "signature": "argument_types_omitted_slots_ref_bindings_and_profile_arity_not_validated_by_name_graph"},
        "unresolved_reasons": reasons, "validity": "not_proven", "dynamic_expansion": "conservative_all_matching_symbols_retained"})
}

impl Graph {
    pub fn build(
        appearances: &[Appearance],
        functions: &[ParsedFunction],
        pipeline: &Pipeline,
    ) -> Self {
        let symbols = symbols(functions, pipeline);
        let mut names = BTreeMap::<String, Vec<usize>>::new();
        let mut folded_names = BTreeMap::<String, Vec<usize>>::new();
        for (id, symbol) in symbols.iter().enumerate() {
            if let Some(name) = symbol["name"].as_str() {
                names
                    .entry(if pipeline.analyzer_options.ignore_case {
                        name.to_ascii_uppercase()
                    } else {
                        name.into()
                    })
                    .or_default()
                    .push(id);
                folded_names
                    .entry(name.to_ascii_uppercase())
                    .or_default()
                    .push(id);
            }
        }
        let mut result = Self {
            symbols,
            resolutions: Vec::new(),
            candidate_sets: Vec::new(),
            row_resolution: Vec::with_capacity(appearances.len()),
            slices: Vec::new(),
        };
        let mut cache = BTreeMap::<String, usize>::new();
        let mut candidate_cache = BTreeMap::<String, usize>::new();
        let mut last_progress = None::<Instant>;
        for (row_index, row) in appearances.iter().enumerate() {
            let id = row.target.as_ref().map(|target| {
                // A compact structural DTO, never source/HIR bodies, is the cache key.
                let key =
                    serde_json::to_string(target).expect("target DTO serialization cannot fail");
                *cache.entry(key).or_insert_with(|| {
                    // VM late binding is ASCII-insensitive even when static analyzer
                    // lookup was configured otherwise; do not prune those candidates.
                    let ignore_case = target.dispatch.starts_with("dynamic_")
                        || pipeline.analyzer_options.ignore_case;
                    let candidate_key =
                        serde_json::to_string(&(&target.namespace, &target.pattern, ignore_case))
                            .expect("pattern DTO serialization cannot fail");
                    let candidate_set =
                        *candidate_cache.entry(candidate_key).or_insert_with(|| {
                            let id = result.candidate_sets.len();
                            result.candidate_sets.push(candidates(
                                target,
                                &result.symbols,
                                if ignore_case { &folded_names } else { &names },
                                ignore_case,
                            ));
                            id
                        });
                    let id = result.resolutions.len();
                    result.resolutions.push(resolution(
                        target,
                        candidate_set,
                        result.candidate_sets[candidate_set].count,
                    ));
                    id
                })
            });
            result.row_resolution.push(id);
            if last_progress.is_none_or(|time| time.elapsed() >= Duration::from_secs(1)) {
                crate::watchdog::publish_or_exit(
                    json!({"phase": "coverage_reference_graph", "pending": row.path,
                    "rows_completed": row_index + 1, "rows_total": appearances.len(), "resolutions_completed": result.resolutions.len(), "lastFullResponse": null}),
                );
                last_progress = Some(Instant::now());
            }
        }
        for (slice, names) in [
            ("title", &["SYSTEM_TITLE", "TITLE"][..]),
            ("GRAPH_DB_INIT", &["GRAPH_DB_INIT"][..]),
        ] {
            result
                .slices
                .push(result.slice(slice, names, appearances, functions, pipeline));
        }
        result
    }

    fn slice(
        &self,
        name: &str,
        seeds: &[&str],
        appearances: &[Appearance],
        functions: &[ParsedFunction],
        pipeline: &Pipeline,
    ) -> Value {
        let roots = functions
            .iter()
            .filter(|function| {
                seeds.iter().any(|seed| {
                    same_name(&function.name, seed, pipeline.analyzer_options.ignore_case)
                })
            })
            .map(|function| function.id)
            .collect::<BTreeSet<_>>();
        let mut queue = VecDeque::from_iter(roots.iter().copied());
        let mut visited = roots.clone();
        let mut rows_by_owner = BTreeMap::<usize, Vec<usize>>::new();
        for (row, appearance) in appearances.iter().enumerate() {
            if let Some(owner) = appearance.owning_function {
                rows_by_owner.entry(owner).or_default().push(row);
            }
            if row.is_multiple_of(16_384) || row + 1 == appearances.len() {
                crate::watchdog::publish_or_exit(json!({"phase": "coverage_slice_index",
                    "case": name, "rows_completed": row + 1, "rows_total": appearances.len(),
                    "owners_indexed": rows_by_owner.len(), "pending": appearance.path}));
            }
        }
        let mut references = BTreeSet::new();
        let mut static_edges = Vec::new();
        let mut candidates = BTreeSet::new();
        let diagnostics = DiagnosticIndex::new(&pipeline.diagnostics);
        let mut blockers = Vec::new();
        while let Some(owner) = queue.pop_front() {
            for row_id in rows_by_owner.get(&owner).into_iter().flatten().copied() {
                let row = &appearances[row_id];
                references.insert(row_id);
                if references.len() == 1 || references.len().is_multiple_of(4096) {
                    crate::watchdog::publish_or_exit(json!({"phase": "coverage_slice_references",
                        "case": name, "references_discovered": references.len(),
                        "functions_discovered": visited.len(), "functions_pending": queue.len(),
                        "pending": {"owner": owner, "row": row_id, "path": row.path}}));
                }
                let valid = row.activity == "active_ast"
                    && row.span_status == "valid_decoded_utf8"
                    && row.ownership_status == "parser_function_membership_not_execution"
                    && functions
                        .get(owner)
                        .is_some_and(|function| function.span_status == "valid_decoded_utf8");
                let language = if valid {
                    diagnostics
                        .overlapping(&row.path, row.span, "analyzer")
                        .into_iter()
                        .chain(diagnostics.overlapping(&row.path, row.span, "compiler"))
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let service = evidence::required_service(&row.api);
                if !language.is_empty()
                    || service.is_some()
                    || row.api.starts_with("SQL_")
                    || !valid
                {
                    blockers.push(json!({"row_id": row_id, "language_diagnostic_ids": language,
                        "language_feature": if row.api.starts_with("SQL_") { Some("C01") } else { None },
                        "required_service": service, "frontend_status": "unverified_without_capture",
                        "source_status": if valid { "static_reference_only" } else { "unverified_source_or_ownership" }}));
                }
                let Some(resolution) = self.row_resolution[row_id] else {
                    continue;
                };
                candidates.insert(resolution);
                let Some(target) = &row.target else {
                    continue;
                };
                if !valid
                    || !target.executes_body
                    || !target.dispatch.starts_with("direct_")
                    || target.pattern.exact().is_none()
                {
                    continue;
                }
                let Some(set) = self.resolutions[resolution]["candidate_set_ref"]
                    .as_u64()
                    .and_then(|id| self.candidate_sets.get(id as usize))
                else {
                    continue;
                };
                for candidate in set.indices() {
                    let Some(symbol) = self.symbols.get(candidate) else {
                        continue;
                    };
                    let Some(next) = symbol["parser_function_id"].as_u64().map(|id| id as usize)
                    else {
                        continue;
                    };
                    static_edges.push(json!({"from": owner, "to": next, "row_id": row_id,
                        "evidence": "active_ast_exact_name_reference_not_proven_runtime_binding"}));
                    if static_edges.len().is_multiple_of(16_384) {
                        crate::watchdog::publish_or_exit(json!({"phase": "coverage_slice_edges",
                            "case": name, "static_edges_completed": static_edges.len(),
                            "references_discovered": references.len(),
                            "pending": {"owner": owner, "next": next, "row": row_id}}));
                    }
                    if visited.insert(next) {
                        queue.push_back(next);
                    }
                }
            }
        }
        json!({"name": name, "root_functions": roots, "static_reference_closure": visited,
            "static_edges": static_edges, "outgoing_reference_rows": references, "target_resolution_ids": candidates,
            "blockers_and_unverified_requirements": blockers,
            "status": if roots.is_empty() { "unverified_no_root_symbol" } else { "static_slice_not_execution" },
            "dynamic_targets": "listed_conservatively_not_recursively_promoted_to_static_reachability",
            "unlocated_language_diagnostics": pipeline.diagnostics.iter().enumerate().filter(|(_, diagnostic)| diagnostic.error && (diagnostic.path.is_none() || diagnostic.span.is_none())).map(|(id, _)| id).collect::<Vec<_>>()})
    }
}
