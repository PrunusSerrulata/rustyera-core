#[allow(clippy::wildcard_imports)]
use super::*;

#[cfg(any(target_arch = "wasm32", test))]
impl CooperativeCompiledCacheEncoder {
    #[cfg(test)]
    pub(crate) fn new(
        manifest: Arc<ProjectManifest>,
        extensions: Vec<ExtensionDeclaration>,
        artifact: ValidatedArtifact,
        cache_keys: Vec<Digest>,
        snapshot: CompiledSnapshotMetadata,
        diagnostics: Vec<ProtocolDiagnostic>,
        cancelled: Option<Arc<AtomicBool>>,
    ) -> Self {
        Self::new_for_kind(CooperativeEncoderInput {
            kind: ProjectContainerKind::CompiledCache,
            manifest,
            extensions,
            artifact,
            cache_keys: CacheKeyPlanner::Ready(Some(cache_keys)),
            snapshot,
            diagnostics,
            cancelled,
            progress: None,
            trailing_data: Vec::new(),
        })
    }

    pub(super) fn new_for_kind(input: CooperativeEncoderInput) -> Self {
        Self {
            kind: input.kind,
            manifest: input.manifest,
            extensions: input.extensions,
            artifact: input.artifact,
            snapshot: input.snapshot,
            diagnostics: input.diagnostics,
            cancelled: input.cancelled,
            progress: input.progress,
            planner: Some(CacheLayoutPlanner::new(input.cache_keys)),
            plan: None,
            next_section: 0,
            manifest_encoder: None,
            pending_section: None,
            output: None,
            trailing_data: input.trailing_data,
            progress_completed: 0,
            progress_total: 1,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new_with_incremental(
        manifest: Arc<ProjectManifest>,
        extensions: Vec<ExtensionDeclaration>,
        artifact: ValidatedArtifact,
        incremental: Arc<IncrementalState>,
        snapshot: CompiledSnapshotMetadata,
        diagnostics: Vec<ProtocolDiagnostic>,
        cancelled: Option<Arc<AtomicBool>>,
    ) -> Self {
        Self::new_for_kind(CooperativeEncoderInput {
            kind: ProjectContainerKind::CompiledCache,
            manifest,
            extensions,
            artifact,
            cache_keys: CacheKeyPlanner::Incremental {
                state: incremental,
                keys: Vec::new(),
            },
            snapshot,
            diagnostics,
            cancelled,
            progress: None,
            trailing_data: Vec::new(),
        })
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new_full_project(
        plan: FullProjectEncodingPlan,
        cancelled: Option<Arc<AtomicBool>>,
        progress: Option<crate::ProjectProgressReporter>,
    ) -> Self {
        let FullProjectEncodingPlan {
            manifest,
            extensions,
            artifact,
            incremental,
            snapshot,
            diagnostics,
            configuration_journal,
        } = plan;
        Self::new_for_kind(CooperativeEncoderInput {
            kind: ProjectContainerKind::FullProject,
            manifest,
            extensions,
            artifact,
            cache_keys: CacheKeyPlanner::Incremental {
                state: incremental,
                keys: Vec::new(),
            },
            snapshot,
            diagnostics,
            cancelled,
            progress,
            trailing_data: configuration_journal,
        })
    }

    pub(crate) fn step(&mut self) -> Result<Option<Vec<u8>>, String> {
        if self
            .cancelled
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            return Err("compiled cache build cancelled".into());
        }
        if let Some((section, offset)) = self.pending_section.as_mut() {
            let end = offset
                .saturating_add(COOPERATIVE_MANIFEST_CHUNK_BYTES)
                .min(section.len());
            let chunk = &section[*offset..end];
            let (output, hasher) = self.output.as_mut().expect("cache output was initialized");
            output.extend_from_slice(chunk);
            hasher.update(chunk);
            *offset = end;
            if end == section.len() {
                self.pending_section = None;
                self.next_section += 1;
            }
            self.report_cooperative_progress();
            return Ok(None);
        }
        if self.plan.is_none() {
            self.poll_layout()?;
            self.report_cooperative_progress();
            return Ok(None);
        }
        let plan = self.plan.as_ref().expect("cache layout was planned");
        if self.next_section < plan.section_count() {
            let Some(section) = self.encode_next_section()? else {
                self.report_cooperative_progress();
                return Ok(None);
            };
            self.pending_section = Some((section, 0));
            self.report_cooperative_progress();
            return Ok(None);
        }
        let (mut output, hasher) = self.output.take().expect("cache output was initialized");
        output.extend_from_slice(hasher.finalize().as_bytes());
        output.append(&mut self.trailing_data);
        if let Some(reporter) = &self.progress {
            let completed = self.progress_completed.saturating_add(1);
            reporter.report(crate::ProjectProgress {
                stage: crate::ProjectProgressStage::Packaging,
                completed,
                total: completed,
            });
        }
        Ok(Some(output))
    }

    fn report_cooperative_progress(&mut self) {
        let Some(reporter) = &self.progress else {
            return;
        };
        self.progress_completed = self.progress_completed.saturating_add(1);
        self.progress_total = self
            .progress_total
            .max(self.progress_completed.saturating_add(1));
        reporter.report(crate::ProjectProgress {
            stage: crate::ProjectProgressStage::Packaging,
            completed: self.progress_completed,
            total: self.progress_total,
        });
    }

    fn encode_next_section(&mut self) -> Result<Option<Vec<u8>>, String> {
        let plan = self.plan.as_ref().expect("cache layout was planned");
        let function_start = FIXED_SECTION_COUNT;
        let source_start = function_start + plan.function_ranges.len();
        let cancelled = self.cancelled.as_deref();
        let section = match self.next_section {
            0 => encode_section(
                &CompiledCacheMetadataRef {
                    manifest: &self.artifact.artifact().manifest,
                    call_compatibility: &self.artifact.artifact().call_compatibility,
                    runtime_builtins: &self.artifact.artifact().runtime_builtins,
                    native_imports: &self.artifact.artifact().native_imports,
                    host_imports: &self.artifact.artifact().host_imports,
                    event_groups: &self.artifact.artifact().event_groups,
                },
                self.kind,
                cancelled,
            )?,
            1 => encode_section(&self.artifact.artifact().globals, self.kind, cancelled)?,
            2 => encode_incremental_section(&plan.cache_keys, self.kind, cancelled)?,
            3 => encode_section(&self.artifact.artifact().project_data, self.kind, cancelled)?,
            4 if self.kind == ProjectContainerKind::CompiledCache => {
                encode_compact_source_record_section(
                    &self.artifact.artifact().source_map.sources,
                    &self.manifest,
                    self.kind,
                    cancelled,
                )?
            }
            4 => encode_source_record_section(
                &self.artifact.artifact().source_map.sources,
                &self.manifest,
                self.kind,
                cancelled,
            )?,
            5 => encode_digest_section(
                &self.artifact.artifact().source_map.statement_fingerprints,
                self.kind,
                cancelled,
            )?,
            MANIFEST_SECTION_INDEX => {
                let encoder = self
                    .manifest_encoder
                    .get_or_insert(ManifestSectionEncoder::new(&self.manifest, self.kind)?);
                let Some(section) = encoder.step(&self.manifest)? else {
                    return Ok(None);
                };
                self.manifest_encoder = None;
                section
            }
            7 => encode_section(&self.snapshot, self.kind, cancelled)?,
            8 => super::sections::encode_diagnostic_templates(
                &self.diagnostics,
                self.kind,
                cancelled,
            )?,
            index if index < source_start => {
                let range = plan.function_ranges[index - function_start].clone();
                encode_section(
                    &self.artifact.artifact().functions[range],
                    self.kind,
                    cancelled,
                )?
            }
            index => {
                let range = plan.source_ranges[index - source_start].clone();
                encode_source_section(
                    &self.artifact.artifact().source_map.entries[range],
                    &plan.function_indices,
                    self.kind,
                    cancelled,
                )?
            }
        };
        Ok(Some(section))
    }

    fn poll_layout(&mut self) -> Result<(), String> {
        let Some(plan) = self
            .planner
            .as_mut()
            .expect("cache layout planner exists")
            .step(&self.manifest, &self.artifact)?
        else {
            return Ok(());
        };
        let mut output = Vec::new();
        encode_project_file_header(
            &mut output,
            self.kind,
            &plan.identity,
            &self.extensions,
            plan.function_ranges.len(),
            plan.source_ranges.len(),
        )?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(&output);
        self.output = Some((output, hasher));
        self.planner = None;
        self.progress_total = cooperative_work_estimate(&self.manifest, &self.artifact, &plan);
        self.plan = Some(plan);
        Ok(())
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn cooperative_work_estimate(
    manifest: &ProjectManifest,
    artifact: &ValidatedArtifact,
    plan: &CacheLayoutPlan,
) -> u64 {
    let payload_quanta = manifest.files.iter().fold(0_usize, |total, file| {
        let bytes = match &file.payload {
            FilePayload::Utf8(value) => value.len(),
            FilePayload::Bytes(value) => value.as_slice().len(),
            FilePayload::ExternalResource(_) => 0,
            FilePayload::IoError(error) => error.message.len(),
        };
        total.saturating_add(bytes.max(1).div_ceil(COOPERATIVE_MANIFEST_CHUNK_BYTES))
    });
    let planning_quanta = artifact
        .artifact()
        .functions
        .len()
        .saturating_add(artifact.artifact().source_map.entries.len())
        .div_ceil(COOPERATIVE_ITEM_QUANTUM);
    let estimate = plan
        .section_count()
        .saturating_mul(4)
        .saturating_add(manifest.files.len().saturating_mul(3))
        .saturating_add(payload_quanta.saturating_mul(3))
        .saturating_add(planning_quanta.saturating_mul(2))
        .saturating_add(32);
    u64::try_from(estimate).unwrap_or(u64::MAX)
}

pub(super) struct ManifestSectionEncoder {
    writer:
        Option<super::io::CountingWriter<'static, zstd::stream::write::Encoder<'static, Vec<u8>>>>,
    file_index: usize,
    payload_offset: usize,
    payload_hasher: Option<blake3::Hasher>,
    kind: ProjectContainerKind,
}

impl ManifestSectionEncoder {
    pub(super) fn new(
        manifest: &ProjectManifest,
        kind: ProjectContainerKind,
    ) -> Result<Self, String> {
        let encoder = zstd::stream::Encoder::new(Vec::new(), kind.compression_level())
            .map_err(|error| error.to_string())?;
        let mut writer = super::io::CountingWriter::new(encoder, None);
        writer
            .write_all(match kind {
                ProjectContainerKind::CompiledCache => COMPACT_MANIFEST_SECTION_MAGIC,
                ProjectContainerKind::FullProject => MANIFEST_SECTION_MAGIC,
            })
            .map_err(|error| error.to_string())?;
        manifest
            .compatibility
            .validate()
            .map_err(|error| error.to_string())?;
        write_bytes(
            &mut writer,
            &serde_json::to_vec(&manifest.compatibility).map_err(|error| error.to_string())?,
        )?;
        write_varint(
            &mut writer,
            u64::try_from(manifest.files.len())
                .map_err(|_| "project manifest has too many files")?,
        )?;
        Ok(Self {
            writer: Some(writer),
            file_index: 0,
            payload_offset: 0,
            payload_hasher: None,
            kind,
        })
    }

    pub(super) fn step(&mut self, manifest: &ProjectManifest) -> Result<Option<Vec<u8>>, String> {
        let Some(file) = manifest.files.get(self.file_index) else {
            return self.finish().map(Some);
        };
        let payload = match &file.payload {
            FilePayload::Utf8(text) => text.as_bytes(),
            FilePayload::Bytes(bytes) => bytes.as_slice(),
            FilePayload::ExternalResource(_)
                if self.kind == ProjectContainerKind::CompiledCache =>
            {
                &[]
            }
            FilePayload::ExternalResource(_) => {
                return Err("full project files cannot contain external resources".into());
            }
            FilePayload::IoError(_) => {
                return Err("project files with I/O errors cannot be cached".into());
            }
        };
        let writer = self
            .writer
            .as_mut()
            .expect("manifest encoder retains its writer");
        if self.payload_hasher.is_none() {
            write_bytes(writer, file.relative_path.as_bytes())?;
            if self.kind == ProjectContainerKind::CompiledCache {
                let hash = file.content_hash.as_ref().map_or_else(
                    || blake3::hash(payload).as_bytes().to_vec(),
                    |value| value.as_slice().to_vec(),
                );
                if hash.len() != blake3::OUT_LEN {
                    return Err("project manifest content hash is not 32 bytes".into());
                }
                let omitted = !matches!(
                    file.category,
                    FileCategory::Configuration | FileCategory::ResourceManifest
                );
                writer
                    .write_all(&[
                        file.category as u8,
                        u8::from(file.category == FileCategory::Resource),
                        u8::from(omitted),
                    ])
                    .map_err(|error| error.to_string())?;
                writer.write_all(&hash).map_err(|error| error.to_string())?;
                if omitted {
                    self.file_index += 1;
                    return Ok(None);
                }
            } else {
                writer
                    .write_all(&[
                        file.category as u8,
                        u8::from(file.content_hash.is_some()),
                        u8::from(matches!(&file.payload, FilePayload::Bytes(_))),
                    ])
                    .map_err(|error| error.to_string())?;
            }
            write_varint(
                writer,
                u64::try_from(payload.len())
                    .map_err(|_| "compiled cache byte string is too large")?,
            )?;
            self.payload_hasher = Some(blake3::Hasher::new());
            return Ok(None);
        }
        let end = self
            .payload_offset
            .saturating_add(COOPERATIVE_MANIFEST_CHUNK_BYTES)
            .min(payload.len());
        let chunk = &payload[self.payload_offset..end];
        writer.write_all(chunk).map_err(|error| error.to_string())?;
        self.payload_hasher
            .as_mut()
            .expect("payload hasher was initialized")
            .update(chunk);
        self.payload_offset = end;
        if end == payload.len() {
            let actual = self
                .payload_hasher
                .take()
                .expect("payload hasher was initialized")
                .finalize();
            if file
                .content_hash
                .as_ref()
                .is_some_and(|expected| expected.as_slice() != actual.as_bytes())
            {
                return Err("project manifest content hash differs from its payload".into());
            }
            self.file_index += 1;
            self.payload_offset = 0;
        }
        Ok(None)
    }

    fn finish(&mut self) -> Result<Vec<u8>, String> {
        let writer = self
            .writer
            .take()
            .expect("manifest encoder retains its writer");
        let decoded_length = writer.bytes;
        let compressed = writer
            .into_inner()
            .finish()
            .map_err(|error| error.to_string())?;
        let mut output = Vec::with_capacity(16 + compressed.len());
        output.extend_from_slice(&decoded_length.to_le_bytes());
        output.extend_from_slice(
            &u64::try_from(compressed.len())
                .map_err(|_| "compiled cache section is too large")?
                .to_le_bytes(),
        );
        output.extend_from_slice(&compressed);
        Ok(output)
    }
}
