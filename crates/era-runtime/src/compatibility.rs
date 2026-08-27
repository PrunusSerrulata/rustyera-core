//! Compatibility resolution is independent of session state and frontend I/O.

use era_config::{ReraConfigDocument, normalize_line_endings};
use era_runtime_protocol::{
    CompatibilityDiagnosticContext, CompatibilityIdentity, FileCategory, FilePayload,
    ProjectCompatibilityResolved, ProjectManifest, ProtocolBytes, ProtocolDiagnostic,
    ResolveProjectCompatibility, RuntimeLogLevel, SourceLocation, SubmittedFile,
    validate_relative_path,
};

/// Parse only the submitted root configuration, without loading a project or changing a session.
#[must_use]
pub fn resolve_project_compatibility(
    request: &ResolveProjectCompatibility,
) -> ProjectCompatibilityResolved {
    let mut report = ProjectCompatibilityResolved {
        request_id: request.request_id,
        identity: None,
        configuration_digest: None,
        diagnostics: Vec::new(),
    };
    let identity = match request.configuration.as_ref() {
        None => CompatibilityIdentity::reference(),
        Some(file) => match resolve_configuration(file) {
            Ok((identity, digest)) => {
                report.configuration_digest = Some(digest);
                identity
            }
            Err(error) => {
                report.diagnostics.push(*error);
                return report;
            }
        },
    };
    if identity.is_experimental() {
        report
            .diagnostics
            .push(experimental_profile_diagnostic(&identity));
    }
    report.identity = Some(identity);
    report
}

/// Hash the submitted root configuration without interpreting it.
///
/// Loading and resolution must separately validate configuration before trusting an identity.
/// This helper is suitable for cache keys even when reporting an invalid project.
#[must_use]
pub fn compatibility_configuration_digest(manifest: &ProjectManifest) -> Option<ProtocolBytes> {
    manifest.files.iter().find_map(|file| {
        if !validate_relative_path(&file.relative_path)
            .is_ok_and(|path| path.eq_ignore_ascii_case("reraconfig.toml"))
        {
            return None;
        }
        let FilePayload::Utf8(source) = &file.payload else {
            return None;
        };
        Some(ProtocolBytes::new(
            blake3::hash(normalize_line_endings(source).as_bytes())
                .as_bytes()
                .to_vec(),
        ))
    })
}

pub(crate) fn resolve_manifest_compatibility(
    manifest: &ProjectManifest,
) -> Result<(CompatibilityIdentity, Option<ProtocolBytes>), Box<ProtocolDiagnostic>> {
    let mut roots = manifest.files.iter().filter(|file| {
        validate_relative_path(&file.relative_path)
            .is_ok_and(|path| path.eq_ignore_ascii_case("reraconfig.toml"))
    });
    let Some(file) = roots.next() else {
        return Ok((CompatibilityIdentity::reference(), None));
    };
    if roots.next().is_some() {
        return Err(configuration_error(
            "runtime.duplicate_compatibility_configuration",
            "project contains duplicate root reraconfig.toml files",
            None,
        ));
    }
    resolve_configuration(file).map(|(identity, digest)| (identity, Some(digest)))
}

fn resolve_configuration(
    file: &SubmittedFile,
) -> Result<(CompatibilityIdentity, ProtocolBytes), Box<ProtocolDiagnostic>> {
    if file.category != FileCategory::Configuration
        || !validate_relative_path(&file.relative_path)
            .is_ok_and(|path| path.eq_ignore_ascii_case("reraconfig.toml"))
    {
        return Err(configuration_error(
            "runtime.invalid_compatibility_configuration",
            "compatibility resolution accepts only the root reraconfig.toml configuration",
            None,
        ));
    }
    let FilePayload::Utf8(source) = &file.payload else {
        return Err(configuration_error(
            "runtime.invalid_compatibility_configuration",
            "root reraconfig.toml requires a readable UTF-8 payload",
            None,
        ));
    };
    if file
        .content_hash
        .as_ref()
        .is_some_and(|digest| digest.as_slice() != blake3::hash(source.as_bytes()).as_bytes())
    {
        return Err(configuration_error(
            "runtime.compatibility_configuration_digest_mismatch",
            "root configuration content hash differs from its payload",
            None,
        ));
    }
    let document = ReraConfigDocument::parse(source).map_err(|error| {
        configuration_error(
            "runtime.invalid_reraconfig",
            error.to_string(),
            error.span.map(|span| (span.start, span.end)),
        )
    })?;
    let values = document.values().map_err(|error| {
        configuration_error(
            "runtime.invalid_reraconfig",
            error.to_string(),
            error.span.map(|span| (span.start, span.end)),
        )
    })?;
    let digest = ProtocolBytes::new(
        blake3::hash(normalize_line_endings(source).as_bytes())
            .as_bytes()
            .to_vec(),
    );
    Ok((
        CompatibilityIdentity::for_profile(values.compatibility_profile()),
        digest,
    ))
}

pub(crate) fn experimental_profile_diagnostic(
    identity: &CompatibilityIdentity,
) -> ProtocolDiagnostic {
    let mut diagnostic = configuration_error(
        "runtime.experimental_compatibility_profile",
        format!(
            "profile {} is experimental: arithmetic={}, RNG={}/state{}, layout={}; snake arithmetic, RNG, REF/extra arguments, TOINT, GETKEY and pixel layout differences remain unimplemented",
            identity.profile,
            identity.arithmetic,
            identity.rng_algorithm,
            identity.rng_state_version,
            identity.layout
        ),
        None,
    );
    diagnostic.level = RuntimeLogLevel::Warning;
    diagnostic
        .context
        .as_mut()
        .expect("configuration context exists")
        .identity = Some(identity.clone());
    *diagnostic
}

pub(crate) fn configuration_error(
    code: &str,
    message: impl Into<String>,
    span: Option<(usize, usize)>,
) -> Box<ProtocolDiagnostic> {
    Box::new(ProtocolDiagnostic {
        code: code.into(),
        level: RuntimeLogLevel::Error,
        message: message.into(),
        source: Some(SourceLocation {
            relative_path: "reraconfig.toml".into(),
            byte_start: span.map_or(0, |span| span.0 as u64),
            byte_end: span.map_or(0, |span| span.1 as u64),
            line: None,
            byte_column: None,
        }),
        notification: era_runtime_protocol::DiagnosticNotification::default(),
        context: Some(Box::new(CompatibilityDiagnosticContext {
            identity: None,
            stage: "configuration".into(),
            api: None,
            required_capability: None,
        })),
    })
}

/// Attach structured identity without replacing an existing capability or precise stage.
pub(crate) fn attach_diagnostic_identity(
    diagnostic: &mut ProtocolDiagnostic,
    identity: &CompatibilityIdentity,
) {
    let stage = diagnostic
        .code
        .split('.')
        .next()
        .unwrap_or("runtime")
        .to_owned();
    let context = diagnostic.context.get_or_insert_with(|| {
        Box::new(CompatibilityDiagnosticContext {
            identity: None,
            stage,
            api: None,
            required_capability: None,
        })
    });
    if context.identity.is_none() {
        context.identity = Some(identity.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(source: &str) -> ResolveProjectCompatibility {
        ResolveProjectCompatibility {
            request_id: 7,
            configuration: Some(SubmittedFile {
                relative_path: "reraconfig.toml".into(),
                category: FileCategory::Configuration,
                payload: FilePayload::Utf8(source.into()),
                content_hash: None,
            }),
        }
    }

    #[test]
    fn resolution_is_strict_and_normalizes_only_configuration_bytes() {
        let source =
            "[meta]\nschema_version = 4\n[compatibility]\nprofile = \"emuera.skia.snake\"\n";
        let resolved = resolve_project_compatibility(&request(source));
        assert_eq!(resolved.request_id, 7);
        assert!(resolved.identity.as_ref().unwrap().is_experimental());
        assert_eq!(
            resolved.configuration_digest,
            resolve_project_compatibility(&request(&source.replace('\n', "\r\n")))
                .configuration_digest
        );
        let invalid =
            resolve_project_compatibility(&request(&source.replace("emuera.skia.snake", "snake")));
        assert!(invalid.identity.is_none());
        assert!(invalid.configuration_digest.is_none());
        assert_eq!(invalid.diagnostics[0].code, "runtime.invalid_reraconfig");
        let default = resolve_project_compatibility(&ResolveProjectCompatibility {
            request_id: 8,
            configuration: None,
        });
        assert_eq!(default.identity, Some(CompatibilityIdentity::reference()));
        assert!(default.configuration_digest.is_none());
    }
}
