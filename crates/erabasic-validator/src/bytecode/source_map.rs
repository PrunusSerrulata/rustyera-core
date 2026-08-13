use std::collections::BTreeMap;

use erabasic_bytecode::BytecodeArtifact;
use rayon::prelude::*;

use crate::{ValidationCode, ValidationDiagnostic};

pub(super) fn validate_source_map(
    artifact: &BytecodeArtifact,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    for source in &artifact.source_map.sources {
        if source.line_starts.first() != Some(&0)
            || !source.line_starts.windows(2).all(|pair| pair[0] < pair[1])
            || source
                .line_starts
                .last()
                .is_some_and(|offset| *offset > source.byte_len)
        {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::InvalidSourceMap,
                format!("source {} has an invalid line table", source.relative_path),
            ));
        }
    }
    let functions: BTreeMap<_, _> = artifact
        .functions
        .iter()
        .map(|function| {
            (
                function.key,
                function
                    .code
                    .iter()
                    .map(erabasic_bytecode::EncodedInstruction::encoded_len)
                    .sum::<u64>(),
            )
        })
        .collect();
    let entry_diagnostics = artifact
        .source_map
        .entries
        .par_chunks(65_536)
        .map(|entries| {
            let mut chunk_diagnostics = Vec::new();
            for entry in entries {
                let valid = functions.get(&entry.function).is_some_and(|length| {
                    entry.code_start < entry.code_end && entry.code_end <= *length
                }) && artifact
                    .source_map
                    .sources
                    .get(entry.source_index as usize)
                    .is_some_and(|source| {
                        entry.byte_start <= entry.byte_end && entry.byte_end <= source.byte_len
                    })
                    && artifact
                        .source_map
                        .statement_fingerprints
                        .get(entry.statement_fingerprint as usize)
                        .is_some();
                if !valid {
                    chunk_diagnostics.push(ValidationDiagnostic::project(
                        ValidationCode::InvalidSourceMap,
                        format!(
                            "source-map entry is outside its function or source \
                             (function={:?}, code={}..{} of {:?}, source={}, bytes={}..{} of {:?}, \
                             fingerprint={} of {})",
                            entry.function,
                            entry.code_start,
                            entry.code_end,
                            functions.get(&entry.function),
                            entry.source_index,
                            entry.byte_start,
                            entry.byte_end,
                            artifact
                                .source_map
                                .sources
                                .get(entry.source_index as usize)
                                .map(|source| source.byte_len),
                            entry.statement_fingerprint,
                            artifact.source_map.statement_fingerprints.len(),
                        ),
                    ));
                }
            }
            chunk_diagnostics
        })
        .collect::<Vec<_>>();
    for mut chunk in entry_diagnostics {
        diagnostics.append(&mut chunk);
    }
}
