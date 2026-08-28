//! Bind immutable compile diagnostics only at a successful project publication.

#[allow(clippy::wildcard_imports)]
use crate::session::*;

impl RuntimeSession {
    pub(in crate::session) fn emit_committed_project_report(
        &mut self,
        message_id: u64,
        mut report: ProjectLoadReport,
        generation: Option<u64>,
    ) -> Result<(), RuntimeError> {
        if !report.success {
            return self.emit(RuntimeMessage::ProjectLoadReport(report), Some(message_id));
        }
        let artifact = self
            .artifact
            .as_ref()
            .ok_or_else(|| {
                RuntimeError::Internal("committed project diagnostic has no artifact".into())
            })?
            .artifact()
            .manifest
            .artifact_id
            .0;
        let scope = ProjectDiagnosticScope {
            artifact,
            project_load_id: self.project_load_id,
            runtime_epoch: self.epoch.0,
            generation,
        };
        let mut publication = match &self.project_diagnostic_publication {
            Some(previous) if previous.scope == scope => previous.clone(),
            _ => ProjectDiagnosticPublication {
                scope,
                sites: BTreeSet::new(),
            },
        };
        let identity = report.compatibility.clone();
        report.diagnostics.retain_mut(|diagnostic| {
            if diagnostic.code != "compat.call.excess_arguments"
                || diagnostic.level != RuntimeLogLevel::Warning
            {
                return true;
            }
            let site = ProjectDiagnosticSite {
                code: diagnostic.code.clone(),
                source: diagnostic.source.as_ref().map(|source| {
                    (
                        source.relative_path.clone(),
                        source.byte_start,
                        source.byte_end,
                    )
                }),
            };
            if !publication.sites.insert(site) {
                return false;
            }
            let context = diagnostic.context.get_or_insert_with(|| {
                Box::new(era_runtime_protocol::CompatibilityDiagnosticContext {
                    identity: identity.clone(),
                    stage: "compat".into(),
                    api: Some("user_call".into()),
                    required_capability: None,
                    artifact: None,
                    project_load_id: None,
                    runtime_epoch: None,
                    generation: None,
                })
            });
            context.artifact = Some(ProtocolBytes::new(artifact.to_vec()));
            context.project_load_id = Some(publication.scope.project_load_id);
            context.runtime_epoch = Some(publication.scope.runtime_epoch);
            context.generation = publication.scope.generation;
            true
        });
        // Only successful serialization and journal insertion consume publication sites.
        // Old scopes are retired here; state is bounded by sites in the current project.
        self.emit(RuntimeMessage::ProjectLoadReport(report), Some(message_id))?;
        self.project_diagnostic_publication = Some(publication);
        Ok(())
    }
}
