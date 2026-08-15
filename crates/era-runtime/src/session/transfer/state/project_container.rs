#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    pub(in super::super::super) fn start_compiled_cache_build(&mut self) -> Result<(), String> {
        let artifact = self
            .artifact
            .clone()
            .ok_or_else(|| "compiled cache build has no loaded artifact".to_owned())?;
        self.compiled_cache_failure = None;
        let snapshot = self
            .project_snapshot
            .as_ref()
            .ok_or_else(|| "compiled cache build has no project snapshot".to_owned())?;
        if snapshot.configuration_snapshot().restart_pending {
            return Err(
                "compiled cache build requires restarting to apply pending configuration"
                    .to_owned(),
            );
        }
        let manifest = Arc::clone(&snapshot.manifest);
        let snapshot = crate::compiled_cache::CompiledSnapshotMetadata::from(snapshot);
        let extensions = self.extension_declarations.clone();
        let incremental = Arc::clone(&self.incremental);
        let diagnostics = self.compiled_cache_diagnostics.clone();
        #[cfg(not(target_arch = "wasm32"))]
        let cancelled = Arc::new(AtomicBool::new(false));
        #[cfg(not(target_arch = "wasm32"))]
        {
            let worker_cancelled = Arc::clone(&cancelled);
            let handle = std::thread::Builder::new()
                .name("rustyera-compiled-cache".into())
                .spawn(move || {
                    crate::compiled_cache::encode_cancellable(
                        manifest,
                        extensions,
                        artifact,
                        incremental,
                        snapshot,
                        diagnostics,
                        worker_cancelled,
                    )
                })
                .map_err(|error| format!("cannot start compiled cache worker: {error}"))?;
            self.compiled_cache_task = Some(ProjectContainerTask::Native {
                cancelled,
                handle: Some(handle),
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.compiled_cache_task = Some(ProjectContainerTask::Cooperative {
                encoder: Box::new(
                    crate::compiled_cache::CooperativeCompiledCacheEncoder::new_with_incremental(
                        manifest,
                        extensions,
                        artifact,
                        incremental,
                        snapshot,
                        diagnostics,
                        None,
                    ),
                ),
            });
        }
        Ok(())
    }

    pub(super) fn start_full_project_build(&mut self) -> Result<(), String> {
        let artifact = self
            .artifact
            .clone()
            .ok_or_else(|| "full project build has no loaded artifact".to_owned())?;
        let snapshot = self
            .project_snapshot
            .as_ref()
            .ok_or_else(|| "full project build has no project snapshot".to_owned())?;
        if snapshot.configuration_snapshot().restart_pending {
            return Err("full project export requires restarting pending configuration".into());
        }
        let manifest = self
            .staged_full_project_manifest
            .take()
            .unwrap_or_else(|| snapshot.manifest.as_ref().clone());
        crate::compiled_cache::validate_full_project_manifest(
            &manifest,
            &crate::compiled_cache::project_identity(&snapshot.manifest),
            &artifact.artifact().source_map.sources,
        )?;
        // A user-requested full export takes precedence over speculative cache preparation.
        // Dropping the cache task signals cancellation without coupling game interaction to it.
        self.compiled_cache_task = None;
        self.compiled_project_cache = None;
        self.compiled_cache_failure = None;
        let manifest = Arc::new(manifest);
        let snapshot = crate::compiled_cache::CompiledSnapshotMetadata::from(snapshot);
        let extensions = self.extension_declarations.clone();
        let incremental = Arc::clone(&self.incremental);
        let diagnostics = self.compiled_cache_diagnostics.clone();
        let progress = self.project_progress_reporter.clone();
        self.full_project_failure = None;
        if let Some(reporter) = &self.project_progress_reporter {
            reporter.report(ProjectProgress {
                stage: ProjectProgressStage::Packaging,
                completed: 0,
                total: 1,
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let cancelled = Arc::new(AtomicBool::new(false));
            let worker_cancelled = Arc::clone(&cancelled);
            let handle = std::thread::Builder::new()
                .name("rustyera-full-project".into())
                .spawn(move || {
                    crate::compiled_cache::encode_full_project_cancellable(
                        manifest,
                        extensions,
                        artifact,
                        incremental,
                        snapshot,
                        diagnostics,
                        crate::compiled_cache::ProjectContainerControl {
                            cancelled: worker_cancelled,
                            progress,
                        },
                    )
                })
                .map_err(|error| format!("cannot start full project worker: {error}"))?;
            self.full_project_task = Some(ProjectContainerTask::Native {
                cancelled,
                handle: Some(handle),
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.full_project_task = Some(ProjectContainerTask::Cooperative {
                encoder: Box::new(
                    crate::compiled_cache::CooperativeCompiledCacheEncoder::new_full_project(
                        manifest,
                        extensions,
                        artifact,
                        incremental,
                        snapshot,
                        diagnostics,
                        None,
                        progress,
                    ),
                ),
            });
        }
        Ok(())
    }

    pub(in super::super::super) fn poll_compiled_cache_task(
        &mut self,
    ) -> Result<bool, RuntimeError> {
        let (result, cooperative_work) = poll_project_container_task(
            &mut self.compiled_cache_task,
            "compiled cache worker panicked",
        );
        let Some(result) = result else {
            return Ok(cooperative_work);
        };
        match result {
            Ok(bytes) => {
                self.compiled_cache_failure = None;
                self.compiled_project_cache = Some(Arc::new(bytes));
                self.emit(
                    RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                        code: "runtime.compiled_cache_ready".into(),
                        level: RuntimeLogLevel::Info,
                        message: "compiled project cache is ready for frontend persistence".into(),
                        source: None,
                    }),
                    None,
                )?;
            }
            Err(error) => {
                self.compiled_cache_failure = Some(error.clone());
                self.emit(
                    RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                        code: "runtime.compiled_cache_failed".into(),
                        level: RuntimeLogLevel::Warning,
                        message: error,
                        source: None,
                    }),
                    None,
                )?;
            }
        }
        Ok(cooperative_work)
    }

    pub(in super::super::super) fn poll_full_project_task(&mut self) -> bool {
        let (result, cooperative_work) = poll_project_container_task(
            &mut self.full_project_task,
            "full project worker panicked",
        );
        let Some(result) = result else {
            return cooperative_work;
        };
        match result {
            Ok(bytes) => {
                self.full_project_failure = None;
                self.full_project_file = Some(Arc::new(bytes));
            }
            Err(error) => self.full_project_failure = Some(error),
        }
        cooperative_work
    }

    pub(in super::super::super) fn cancel_state_export(&mut self, cancel: StateExportCancel) {
        if self
            .outbound_transfer
            .as_ref()
            .is_some_and(|transfer| transfer.descriptor.kind == cancel.kind)
        {
            self.outbound_transfer = None;
        }
        match cancel.kind {
            StateExportKind::CompiledProjectCache => {
                self.compiled_cache_task = None;
                self.compiled_project_cache = None;
                self.compiled_cache_failure = None;
            }
            StateExportKind::FullProjectFile => {
                self.full_project_task = None;
                self.full_project_file = None;
                self.full_project_failure = None;
                self.staged_full_project_manifest = None;
            }
            StateExportKind::TraditionalSave
            | StateExportKind::VmSnapshot
            | StateExportKind::InputReplay
            | StateExportKind::FullProjectManifest => {}
        }
    }
}

fn poll_project_container_task(
    task: &mut Option<ProjectContainerTask>,
    #[cfg_attr(target_arch = "wasm32", allow(unused_variables))] panic_message: &'static str,
) -> (Option<Result<Vec<u8>, String>>, bool) {
    let Some(active) = task.as_mut() else {
        return (None, false);
    };
    match active {
        #[cfg(any(target_arch = "wasm32", test))]
        ProjectContainerTask::Cooperative { encoder } => match encoder.step() {
            Ok(None) => (None, true),
            result => {
                *task = None;
                (
                    Some(result.transpose().expect("completed container result")),
                    true,
                )
            }
        },
        #[cfg(not(target_arch = "wasm32"))]
        ProjectContainerTask::Native { handle, .. } => {
            if !handle.as_ref().is_some_and(JoinHandle::is_finished) {
                return (None, false);
            }
            let mut finished = task.take().expect("finished container task exists");
            let handle = match &mut finished {
                ProjectContainerTask::Native { handle, .. } => handle,
                #[cfg(test)]
                ProjectContainerTask::Cooperative { .. } => {
                    unreachable!("finished native container task changed variant")
                }
            }
            .take()
            .expect("finished container task has a join handle");
            drop(finished);
            let result = handle
                .join()
                .unwrap_or_else(|_| Err(panic_message.to_owned()));
            (Some(result), false)
        }
    }
}
