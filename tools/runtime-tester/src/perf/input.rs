use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use era_protocol::ProtocolBytes;
use era_runtime_protocol::{
    ExternalResource, FileCategory, FilePayload, ProjectManifest, ResolveProjectCompatibility,
    RuntimeLogLevel, SubmittedFile,
};
use erabasic_compat::CompatibilityIdentity;
use sha2::{Digest, Sha256};

use super::{AuditResult, SNAKE_PROFILE};

pub(super) struct PreparedProject {
    pub(super) manifest: ProjectManifest,
    pub(super) digest: String,
    pub(super) input_count: usize,
    pub(super) source_bytes: u64,
    pub(super) resource_bytes: u64,
}

pub(super) fn prepare(
    root: &Path,
    progress: &mut dyn FnMut(&Path, usize),
) -> AuditResult<PreparedProject> {
    let mut paths = super::super::try_collect_project_files(root, progress)?;
    paths.sort();
    let inputs = super::super::project_inputs::ProjectInputs::new(root, &paths);
    let mut project_digest = Sha256::new();
    let mut files = Vec::with_capacity(paths.len());
    let mut configuration = None;
    let mut source_bytes = 0_u64;
    let mut resource_bytes = 0_u64;

    for relative_path in &paths {
        let category = inputs
            .classify(relative_path)
            .ok_or("inventoried project input lost its classification")?;
        let path = root.join(relative_path);
        let (payload, content_hash) = if category == FileCategory::Resource {
            let (blake, byte_length) =
                hash_resource(&path, relative_path, category, &mut project_digest)?;
            resource_bytes = resource_bytes.saturating_add(byte_length);
            (
                FilePayload::ExternalResource(ExternalResource {
                    byte_length,
                    image_metadata: None,
                }),
                ProtocolBytes::new(blake.as_bytes().to_vec()),
            )
        } else {
            let source = super::super::read_submitted_text(&path, category)?;
            let bytes = source.as_bytes();
            update_record_header(
                &mut project_digest,
                relative_path,
                category,
                bytes.len() as u64,
            )?;
            project_digest.update(bytes);
            let content_hash = blake3::hash(bytes);
            let byte_length = bytes.len() as u64;
            source_bytes = source_bytes.saturating_add(byte_length);
            (
                FilePayload::Utf8(source),
                ProtocolBytes::new(content_hash.as_bytes().to_vec()),
            )
        };
        let submitted = SubmittedFile {
            relative_path: relative_path.clone(),
            category,
            payload,
            content_hash: Some(content_hash),
        };
        if relative_path.eq_ignore_ascii_case("reraconfig.toml")
            && configuration.replace(submitted.clone()).is_some()
        {
            return Err("project contains multiple root reraconfig.toml inputs".into());
        }
        files.push(submitted);
    }

    let configuration = configuration.ok_or("perf-run requires root reraconfig.toml")?;
    let resolved = era_runtime::resolve_project_compatibility(&ResolveProjectCompatibility {
        request_id: 1,
        configuration: Some(configuration),
    });
    let snake = CompatibilityIdentity::for_profile(SNAKE_PROFILE);
    if resolved.identity.as_ref() != Some(&snake)
        || resolved
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level == RuntimeLogLevel::Error)
    {
        return Err(format!(
            "resolve_project_compatibility did not resolve strict snake identity: {:?}",
            resolved.diagnostics
        )
        .into());
    }

    let input_count = files.len();
    Ok(PreparedProject {
        manifest: ProjectManifest {
            project_revision: 1,
            files,
            compatibility: snake,
        },
        digest: format!("{:x}", project_digest.finalize()),
        input_count,
        source_bytes,
        resource_bytes,
    })
}

fn hash_resource(
    path: &Path,
    relative_path: &str,
    category: FileCategory,
    project_digest: &mut Sha256,
) -> AuditResult<(blake3::Hash, u64)> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("resource must be a regular file: {}", path.display()).into());
    }
    let byte_length = metadata.len();
    update_record_header(project_digest, relative_path, category, byte_length)?;
    let mut blake = blake3::Hasher::new();
    let mut file = File::open(path)?;
    let mut observed = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let bytes = &buffer[..read];
        observed = observed.saturating_add(read as u64);
        project_digest.update(bytes);
        blake.update(bytes);
    }
    if observed != byte_length {
        return Err(format!(
            "resource changed while hashing: {} expected={byte_length} observed={observed}",
            path.display()
        )
        .into());
    }
    Ok((blake.finalize(), byte_length))
}

fn update_record_header(
    digest: &mut Sha256,
    relative_path: &str,
    category: FileCategory,
    byte_length: u64,
) -> AuditResult<()> {
    let category = serde_json::to_vec(&category)?;
    digest.update((relative_path.len() as u64).to_le_bytes());
    digest.update(relative_path.as_bytes());
    digest.update((category.len() as u64).to_le_bytes());
    digest.update(category);
    digest.update(byte_length.to_le_bytes());
    Ok(())
}

pub(super) fn validate_isolated_project(project: &Path) -> AuditResult<()> {
    let canonical = project.canonicalize()?;
    if !canonical.is_dir() {
        return Err("--project must name a directory".into());
    }
    let repository = super::super::repository_root();
    let workspace = repository
        .ancestors()
        .find(|ancestor| ancestor.join("games/eratw-sub-modding").exists());
    if let Some(source) = workspace
        .map(|workspace| workspace.join("games/eratw-sub-modding"))
        .and_then(|source| source.canonicalize().ok())
        && (canonical == source || canonical.starts_with(&source))
    {
        return Err("perf-run rejects the shared snake-TW source; use an isolated copy".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedicated_fixture_resolves_strict_snake_identity() -> AuditResult<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixture-snake-perf");
        let project = prepare(&root, &mut |_, _| {})?;
        assert_eq!(
            project.manifest.compatibility.profile,
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake
        );
        assert_eq!(project.digest.len(), 64);
        assert!(project.manifest.files.iter().any(|file| {
            file.category == FileCategory::Resource
                && matches!(&file.payload, FilePayload::ExternalResource(_))
        }));
        Ok(())
    }

    #[test]
    fn project_digest_domain_separates_path_category_length_and_content() -> AuditResult<()> {
        fn digest(path: &str, category: FileCategory, content: &[u8]) -> AuditResult<String> {
            let mut digest = Sha256::new();
            update_record_header(&mut digest, path, category, content.len() as u64)?;
            digest.update(content);
            Ok(format!("{:x}", digest.finalize()))
        }
        let baseline = digest("ERB/main.erb", FileCategory::Erb, b"A")?;
        assert_ne!(baseline, digest("ERB/other.erb", FileCategory::Erb, b"A")?);
        assert_ne!(baseline, digest("ERB/main.erb", FileCategory::Erh, b"A")?);
        assert_ne!(baseline, digest("ERB/main.erb", FileCategory::Erb, b"AA")?);
        assert_ne!(baseline, digest("ERB/main.erb", FileCategory::Erb, b"B")?);
        Ok(())
    }
}
