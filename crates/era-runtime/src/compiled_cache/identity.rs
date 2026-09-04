#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn project_key(
    identity: &ProjectIdentity,
    extensions: &[ExtensionDeclaration],
) -> [u8; 32] {
    let mut writer = HashWriter::new("rustyera.compiled-project-key.v4");
    serde_json::to_writer(
        &mut writer,
        &(
            identity.source_digest.as_slice(),
            &identity.compatibility,
            &identity.configuration_digest,
            extensions,
        ),
    )
    .expect("project cache identity values are serializable");
    writer.finish()
}

pub(crate) fn project_identity(manifest: &ProjectManifest) -> ProjectIdentity {
    let mut files = manifest
        .files
        .iter()
        .map(|file| {
            (
                file.relative_path.to_lowercase(),
                file.relative_path.as_str(),
                file,
            )
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(right.1)));
    let mut hasher = blake3::Hasher::new_derive_key("rustyera.project-source-identity.v1");
    for (_, _, file) in files {
        let path = file.relative_path.as_bytes();
        hasher.update(&(path.len() as u64).to_le_bytes());
        hasher.update(path);
        hasher.update(&[file.category as u8]);
        let digest = file.content_hash.as_ref().map_or_else(
            || match &file.payload {
                FilePayload::Utf8(text) => *blake3::hash(text.as_bytes()).as_bytes(),
                FilePayload::Bytes(bytes) => *blake3::hash(bytes.as_slice()).as_bytes(),
                FilePayload::ExternalResource(resource) => {
                    *blake3::hash(&resource.byte_length.to_le_bytes()).as_bytes()
                }
                FilePayload::IoError(error) => *blake3::hash(error.message.as_bytes()).as_bytes(),
            },
            |value| {
                value
                    .as_slice()
                    .try_into()
                    .unwrap_or_else(|_| *blake3::hash(value.as_slice()).as_bytes())
            },
        );
        hasher.update(&digest);
    }
    ProjectIdentity {
        project_revision: manifest.project_revision,
        compatibility: manifest.compatibility.clone(),
        configuration_digest: crate::compatibility_configuration_digest(manifest),
        source_digest: ProtocolBytes::new(hasher.finalize().as_bytes().to_vec()),
    }
}

pub(crate) fn validate_full_project_manifest(
    manifest: &ProjectManifest,
    expected_identity: &ProjectIdentity,
    sources: &[SourceRecord],
) -> Result<(), String> {
    if &project_identity(manifest) != expected_identity {
        return Err("project files changed after the active project was loaded".into());
    }
    validate_full_project_sources(manifest, sources)
}

pub(crate) fn validate_full_project_sources(
    manifest: &ProjectManifest,
    sources: &[SourceRecord],
) -> Result<(), String> {
    let mut paths = BTreeSet::new();
    for file in &manifest.files {
        let path =
            validate_relative_path(&file.relative_path).map_err(|error| error.to_string())?;
        if !paths.insert(path.to_lowercase()) {
            return Err("full project manifest contains duplicate paths".into());
        }
        let payload = match &file.payload {
            FilePayload::Utf8(text) => text.as_bytes(),
            FilePayload::Bytes(bytes) => bytes.as_slice(),
            FilePayload::ExternalResource(_) => {
                return Err("full project manifest contains an external resource".into());
            }
            FilePayload::IoError(_) => {
                return Err("full project manifest contains an unreadable file".into());
            }
        };
        if file
            .content_hash
            .as_ref()
            .is_some_and(|expected| expected.as_slice() != blake3::hash(payload).as_bytes())
        {
            return Err("full project manifest content hash differs from its payload".into());
        }
    }
    let files = manifest
        .files
        .iter()
        .map(|file| {
            validate_relative_path(&file.relative_path)
                .map(|path| (path, file))
                .map_err(|error| error.to_string())
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
    for source in sources {
        let file = files
            .get(&source.relative_path)
            .ok_or("full project manifest is missing a compiled source")?;
        if source_record_from_file(file)? != *source {
            return Err("full project manifest source differs from the active artifact".into());
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn encode_full_project_for_test(
    manifest: &ProjectManifest,
    extensions: &[ExtensionDeclaration],
    artifact: &ValidatedArtifact,
    incremental: &IncrementalState,
    snapshot: &NormalizedProjectSnapshot,
    diagnostics: &[ProtocolDiagnostic],
) -> Result<Vec<u8>, String> {
    let snapshot = CompiledSnapshotMetadata::from(snapshot);
    let cache_keys = incremental.compact_cache_keys(artifact.artifact())?;
    let mut encoder = CooperativeCompiledCacheEncoder::new_for_kind(CooperativeEncoderInput {
        kind: ProjectContainerKind::FullProject,
        manifest: Arc::new(manifest.clone()),
        extensions: extensions.to_vec(),
        artifact: artifact.clone(),
        cache_keys: CacheKeyPlanner::Ready(Some(cache_keys)),
        snapshot,
        diagnostics: diagnostics.to_vec(),
        cancelled: None,
        progress: None,
        trailing_data: Vec::new(),
    });
    loop {
        if let Some(bytes) = encoder.step()? {
            return Ok(bytes.into_vec());
        }
    }
}

#[cfg(test)]
pub(crate) fn encode_compiled_cache_for_test(
    manifest: &ProjectManifest,
    extensions: &[ExtensionDeclaration],
    artifact: &ValidatedArtifact,
    incremental: &IncrementalState,
    snapshot: &NormalizedProjectSnapshot,
    diagnostics: &[ProtocolDiagnostic],
) -> Result<Vec<u8>, String> {
    let snapshot = CompiledSnapshotMetadata::from(snapshot);
    let cache_keys = incremental.compact_cache_keys(artifact.artifact())?;
    let mut encoder = CooperativeCompiledCacheEncoder::new(
        Arc::new(manifest.clone()),
        extensions.to_vec(),
        artifact.clone(),
        cache_keys,
        snapshot,
        diagnostics.to_vec(),
        None,
    );
    loop {
        if let Some(bytes) = encoder.step()? {
            return Ok(bytes.into_vec());
        }
    }
}
