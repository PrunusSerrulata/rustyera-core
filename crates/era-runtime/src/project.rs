use era_runtime_protocol::{
    DiagnosticSeverity, FileCategory, FileChange, FilePayload, ProjectAnalysisReport,
    ProjectAnalysisRequest, ProjectLoadReport, ProjectManifest, ProtocolDiagnostic, ReloadProject,
    SourceLocation, validate_relative_path,
};
use erabasic_analyzer::{
    AnalysisInput, AnalyzerDiagnosticSeverity, AnalyzerOptions, ArgumentConstraint,
    CallableSignature, ExtensionRegistry, InstructionSignature, ProjectSource, SourceIoError,
    SourceIoErrorKind, SourcePayload, WarningPolicy, analyze_project, builtin_function_names,
    builtin_instruction_names,
};
use erabasic_bytecode::BytecodeArtifact;
use erabasic_compiler::{
    CompilerOptions, IncrementalState, compile_project_with_artifact, default_host_registry,
    extension_binding,
};
use erabasic_config::{ConfigStore, ConfigValue};
use erabasic_csv::{
    CsvDiagnosticSeverity, CsvLoadOptions, FilePayload as CsvFilePayload,
    FrontendFile as CsvFrontendFile, FrontendIoError as CsvIoError,
    FrontendIoErrorKind as CsvIoErrorKind, ProjectFiles, load_project,
};
use erabasic_data::LegacyEncoding;
use erabasic_hir::SemanticType;
use erabasic_parser::ArgumentStyle;
use erabasic_validator::{ValidatedArtifact, ValidationContext, validate_compiler_output};
use serde::{Deserialize, Serialize};

use crate::resource::ResourceGraph;

pub(crate) struct ProjectBuild {
    pub(crate) artifact: Option<ValidatedArtifact>,
    pub(crate) incremental: IncrementalState,
    pub(crate) report: ProjectLoadReport,
    pub(crate) snapshot: Option<NormalizedProjectSnapshot>,
}

#[derive(Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct NormalizedProjectSnapshot {
    pub(crate) manifest: ProjectManifest,
    pub(crate) project_identity: [u8; 32],
    pub(crate) resources: Vec<NormalizedResourceIdentity>,
    pub(crate) resource_graph: ResourceGraph,
    pub(crate) sort_with_filename: bool,
    pub(crate) use_new_random_ignored: bool,
    pub(crate) auto_save: bool,
    pub(crate) ctrl_z_enabled: bool,
    pub(crate) allow_long_input_by_activation: bool,
    pub(crate) save_in_binary: bool,
    pub(crate) compress_save: bool,
    pub(crate) save_slot_count: u32,
    pub(crate) money_label: String,
    pub(crate) money_first: bool,
    pub(crate) maximum_shop_items: u32,
    pub(crate) viewport_width: u32,
    pub(crate) _viewport_height: u32,
    pub(crate) font_size: u32,
    pub(crate) line_height: u32,
    pub(crate) print_c_per_line: u32,
    pub(crate) print_c_length: u32,
    /// Complete query-visible configuration, including client-only compatibility values.
    pub(crate) configuration: ConfigStore,
    pub(crate) extensions:
        std::collections::BTreeMap<String, era_runtime_protocol::ExtensionDeclaration>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct NormalizedResourceIdentity {
    pub(crate) relative_path: String,
    pub(crate) category: FileCategory,
    pub(crate) payload_digest: [u8; 32],
}

#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct SemanticConfig {
    values: ConfigStore,
    csv: CsvLoadOptions,
    analyzer: AnalyzerOptions,
    use_new_random: bool,
    auto_save: bool,
    ctrl_z_enabled: bool,
    allow_long_input_by_activation: bool,
    save_in_binary: bool,
    compress_save: bool,
    save_slot_count: u32,
    money_label: String,
    money_first: bool,
    maximum_shop_items: u32,
    viewport_width: u32,
    viewport_height: u32,
    font_size: u32,
    line_height: u32,
    print_c_per_line: u32,
    print_c_length: u32,
    legacy_encoding: LegacyEncoding,
}

impl Default for SemanticConfig {
    fn default() -> Self {
        Self {
            values: ConfigStore::default(),
            csv: CsvLoadOptions::default(),
            analyzer: AnalyzerOptions::default(),
            use_new_random: false,
            auto_save: true,
            ctrl_z_enabled: false,
            allow_long_input_by_activation: false,
            save_in_binary: false,
            compress_save: false,
            save_slot_count: 20,
            money_label: "$".into(),
            money_first: true,
            maximum_shop_items: 100,
            viewport_width: 760,
            viewport_height: 480,
            font_size: 18,
            line_height: 19,
            print_c_per_line: 3,
            print_c_length: 25,
            legacy_encoding: LegacyEncoding::Japanese,
        }
    }
}

// Keeping the pipeline in one function makes the atomic artifact/report outcome visible;
// conversion details live in the helpers below.
#[allow(clippy::too_many_lines)]
#[cfg(test)]
pub(crate) fn build_project(
    manifest: &ProjectManifest,
    previous: Option<&IncrementalState>,
) -> ProjectBuild {
    build_project_inner(manifest, previous, None)
}

pub(crate) fn build_project_with_extensions(
    manifest: &ProjectManifest,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    extensions: &[era_runtime_protocol::ExtensionDeclaration],
) -> ProjectBuild {
    build_project_inner_with_extensions(
        manifest,
        previous,
        previous_artifact,
        None,
        false,
        extensions,
    )
}

pub(crate) fn analyze_submitted_project_with_extensions(
    request: &ProjectAnalysisRequest,
    extensions: &[era_runtime_protocol::ExtensionDeclaration],
) -> ProjectAnalysisReport {
    let selected = request
        .selected_erb_paths
        .iter()
        .filter_map(|path| validate_relative_path(path).ok())
        .map(|path| path.to_ascii_lowercase())
        .collect::<std::collections::BTreeSet<_>>();
    let build = build_project_inner_with_extensions(
        &request.manifest,
        None,
        None,
        Some(&selected),
        request.debug_mode,
        extensions,
    );
    let mut analyzed_erb_paths = request
        .manifest
        .files
        .iter()
        .filter(|file| file.category == FileCategory::Erb)
        .filter_map(|file| validate_relative_path(&file.relative_path).ok())
        .filter(|path| selected.is_empty() || selected.contains(&path.to_ascii_lowercase()))
        .collect::<Vec<_>>();
    analyzed_erb_paths.sort();
    ProjectAnalysisReport {
        project_revision: request.manifest.project_revision,
        success: build.report.success,
        diagnostics: build.report.diagnostics,
        analyzed_erb_paths,
    }
}

#[cfg(test)]
fn build_project_inner(
    manifest: &ProjectManifest,
    previous: Option<&IncrementalState>,
    analysis_selection: Option<&std::collections::BTreeSet<String>>,
) -> ProjectBuild {
    build_project_inner_with_extensions(manifest, previous, None, analysis_selection, false, &[])
}

#[allow(clippy::too_many_lines)]
fn build_project_inner_with_extensions(
    manifest: &ProjectManifest,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    analysis_selection: Option<&std::collections::BTreeSet<String>>,
    analysis_debug_mode: bool,
    extension_declarations: &[era_runtime_protocol::ExtensionDeclaration],
) -> ProjectBuild {
    let mut diagnostics = Vec::new();
    let (extensions, host_registry, extension_map) =
        prepare_extensions(extension_declarations, &mut diagnostics);
    let mut files = manifest.files.clone();
    let mut config = parse_configuration(&files, &mut diagnostics);
    if config.csv.sort_with_filename {
        files.sort_by_key(|file| {
            (
                !path_has_priority_directory(&file.relative_path),
                file.relative_path.to_ascii_lowercase(),
                file.relative_path.clone(),
                file.category as u8,
            )
        });
    }
    let mut csv_files = ProjectFiles::default();
    let mut sources = Vec::new();
    let mut resources = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for mut file in files {
        let path = match validate_relative_path(&file.relative_path) {
            Ok(path) => path,
            Err(error) => {
                diagnostics.push(project_diagnostic(
                    "runtime.invalid_path",
                    DiagnosticSeverity::Error,
                    error.message,
                    Some(SourceLocation {
                        relative_path: file.relative_path,
                        byte_start: 0,
                        byte_end: 0,
                        line: None,
                        byte_column: None,
                    }),
                ));
                continue;
            }
        };
        file.relative_path.clone_from(&path);
        if !seen.insert((file.category as u8, path.to_ascii_lowercase())) {
            diagnostics.push(project_diagnostic(
                "runtime.duplicate_path",
                DiagnosticSeverity::Error,
                "duplicate normalized project path",
                Some(SourceLocation {
                    relative_path: path,
                    byte_start: 0,
                    byte_end: 0,
                    line: None,
                    byte_column: None,
                }),
            ));
            continue;
        }
        if let (Some(expected), Some(actual)) =
            (file.content_hash.as_ref(), payload_hash(&file.payload))
            && expected.as_slice() != actual.as_bytes()
        {
            diagnostics.push(project_diagnostic(
                "runtime.content_hash_mismatch",
                DiagnosticSeverity::Error,
                "submitted content hash does not match the payload",
                Some(SourceLocation {
                    relative_path: path.clone(),
                    byte_start: 0,
                    byte_end: 0,
                    line: None,
                    byte_column: None,
                }),
            ));
            continue;
        }
        match file.category {
            FileCategory::Csv => csv_files
                .csv
                .push(csv_file(category_relative_path(&path, "CSV"), file.payload)),
            FileCategory::Erh | FileCategory::Erb => {
                csv_files.erb.push(csv_file(
                    category_relative_path(&path, "ERB"),
                    file.payload.clone(),
                ));
                if file.category == FileCategory::Erh
                    || analysis_selection.is_none_or(|selection| {
                        selection.is_empty() || selection.contains(&path.to_ascii_lowercase())
                    })
                {
                    sources.push(analyzer_source(path, file.payload));
                }
            }
            FileCategory::Configuration => {}
            FileCategory::ResourceManifest | FileCategory::Resource => {
                if let Some(identity) =
                    normalize_resource(&mut diagnostics, path, file.category, &file.payload)
                {
                    resources.push(identity);
                }
            }
        }
    }

    let csv = load_project(&csv_files, &config.csv);
    diagnostics.extend(csv.diagnostics.iter().map(|diagnostic| ProtocolDiagnostic {
        code: format!("csv.{:?}", diagnostic.code).to_ascii_lowercase(),
        severity: match diagnostic.severity {
            CsvDiagnosticSeverity::Notice => DiagnosticSeverity::Information,
            CsvDiagnosticSeverity::Warning => DiagnosticSeverity::Warning,
            CsvDiagnosticSeverity::Error | CsvDiagnosticSeverity::Fatal => {
                DiagnosticSeverity::Error
            }
        },
        message: diagnostic.message.clone(),
        source: diagnostic.source.as_ref().map(|source| SourceLocation {
            relative_path: source.relative_path.clone(),
            byte_start: source.byte_start as u64,
            byte_end: source.byte_end as u64,
            line: Some(u64::from(source.physical_line)),
            byte_column: None,
        }),
    }));
    let Some(mut data) = csv.data else {
        return failed(manifest.project_revision, diagnostics, previous);
    };
    data.static_data.legacy_encoding = config.legacy_encoding;
    sync_replace_configuration(&mut config.values, &data.static_data.replace);
    config
        .money_label
        .clone_from(&data.static_data.replace.money_label);
    config.money_first = data.static_data.replace.money_first;
    config.maximum_shop_items = u32::try_from(data.static_data.replace.max_shop_item).unwrap_or(0);
    let mut analyzer_options = config.analyzer.clone();
    if analysis_selection.is_some() {
        analyzer_options.analysis_mode = true;
        analyzer_options.debug_mode = analysis_debug_mode;
        analyzer_options.ignore_uncalled_functions = false;
    }
    let analysis = analyze_project(
        AnalysisInput {
            project_data: data,
            sources,
        },
        &analyzer_options,
        &extensions,
    );
    diagnostics.extend(
        analysis
            .diagnostics
            .iter()
            .map(|diagnostic| ProtocolDiagnostic {
                code: format!("analyzer.{:?}", diagnostic.code).to_ascii_lowercase(),
                severity: match diagnostic.severity {
                    AnalyzerDiagnosticSeverity::Notice => DiagnosticSeverity::Information,
                    AnalyzerDiagnosticSeverity::Warning => DiagnosticSeverity::Warning,
                    AnalyzerDiagnosticSeverity::Error | AnalyzerDiagnosticSeverity::Fatal => {
                        DiagnosticSeverity::Error
                    }
                },
                message: diagnostic.message.clone(),
                source: diagnostic.source.as_ref().map(|source| SourceLocation {
                    relative_path: source.relative_path.clone(),
                    byte_start: source.byte_start as u64,
                    byte_end: source.byte_end as u64,
                    line: Some(u64::from(source.physical_line)),
                    byte_column: None,
                }),
            }),
    );
    let Some(project) = analysis.project else {
        return failed(manifest.project_revision, diagnostics, previous);
    };
    if analysis_selection.is_some() {
        let success = !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error);
        return ProjectBuild {
            artifact: None,
            incremental: IncrementalState::default(),
            report: ProjectLoadReport {
                project_revision: manifest.project_revision,
                success,
                diagnostics,
                payload_required: false,
            },
            snapshot: None,
        };
    }
    let compile = compile_project_with_artifact(
        &project,
        &CompilerOptions::default(),
        &host_registry,
        previous,
        previous_artifact,
    );
    diagnostics.extend(
        compile
            .diagnostics
            .iter()
            .map(|diagnostic| ProtocolDiagnostic {
                code: format!("compiler.{:?}", diagnostic.code).to_ascii_lowercase(),
                severity: match diagnostic.severity {
                    erabasic_compiler::CompilerDiagnosticSeverity::Notice => {
                        DiagnosticSeverity::Information
                    }
                    erabasic_compiler::CompilerDiagnosticSeverity::Warning => {
                        DiagnosticSeverity::Warning
                    }
                    erabasic_compiler::CompilerDiagnosticSeverity::Error => {
                        DiagnosticSeverity::Error
                    }
                },
                message: diagnostic.message.clone(),
                source: diagnostic.location.map(|location| SourceLocation {
                    relative_path: project
                        .program
                        .sources
                        .iter()
                        .find(|source| source.id == location.source)
                        .map_or_else(String::new, |source| source.relative_path.clone()),
                    byte_start: location.span.start as u64,
                    byte_end: location.span.end as u64,
                    line: None,
                    byte_column: None,
                }),
            }),
    );
    let incremental = compile.incremental_state;
    let Some(artifact) = compile.artifact else {
        return failed_with_incremental(manifest.project_revision, diagnostics, incremental);
    };
    // The compiler already assigned identities after validating this in-process
    // artifact. Re-run structural checks at the runtime boundary, but do not
    // serialize the entire artifact again solely to verify compiler-owned IDs.
    // Decoded or externally supplied bytecode still uses `validate_bytecode`.
    let validation_context = ValidationContext::for_artifact(&artifact);
    let validation = validate_compiler_output(artifact, &validation_context);
    diagnostics.extend(validation.diagnostics.iter().map(|diagnostic| {
        project_diagnostic(
            &format!("validator.{:?}", diagnostic.code).to_ascii_lowercase(),
            DiagnosticSeverity::Error,
            diagnostic.message.clone(),
            None,
        )
    }));
    let Some(artifact) = validation.value else {
        return failed_with_incremental(manifest.project_revision, diagnostics, incremental);
    };
    let success = !diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error);
    if !success {
        return failed_with_incremental(manifest.project_revision, diagnostics, incremental);
    }
    resources.sort_by(|left, right| {
        left.relative_path
            .cmp(&right.relative_path)
            .then_with(|| (left.category as u8).cmp(&(right.category as u8)))
    });
    let (resource_graph, resource_diagnostics) = ResourceGraph::from_manifest(manifest);
    diagnostics.extend(resource_diagnostics.into_iter().map(|diagnostic| {
        project_diagnostic(
            diagnostic.code,
            if diagnostic.error {
                DiagnosticSeverity::Error
            } else {
                DiagnosticSeverity::Warning
            },
            diagnostic.message,
            Some(SourceLocation {
                relative_path: diagnostic.path,
                byte_start: 0,
                byte_end: 0,
                line: diagnostic.line,
                byte_column: None,
            }),
        )
    }));
    let success = !diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error);
    if !success {
        return failed_with_incremental(manifest.project_revision, diagnostics, incremental);
    }
    let project_identity = project_identity(&artifact, &config, &resources, &extension_map);
    ProjectBuild {
        artifact: Some(artifact),
        incremental,
        report: ProjectLoadReport {
            project_revision: manifest.project_revision,
            success,
            diagnostics,
            payload_required: false,
        },
        snapshot: Some(NormalizedProjectSnapshot {
            manifest: manifest.clone(),
            project_identity,
            resources,
            resource_graph,
            sort_with_filename: config.csv.sort_with_filename,
            use_new_random_ignored: config.use_new_random,
            auto_save: config.auto_save,
            ctrl_z_enabled: config.ctrl_z_enabled,
            allow_long_input_by_activation: config.allow_long_input_by_activation,
            save_in_binary: config.save_in_binary,
            compress_save: config.compress_save,
            save_slot_count: config.save_slot_count,
            money_label: config.money_label,
            money_first: config.money_first,
            maximum_shop_items: config.maximum_shop_items,
            viewport_width: config.viewport_width,
            _viewport_height: config.viewport_height,
            font_size: config.font_size,
            line_height: config.line_height,
            print_c_per_line: config.print_c_per_line,
            print_c_length: config.print_c_length,
            configuration: config.values,
            extensions: extension_map,
        }),
    }
}

fn category_relative_path(path: &str, category: &str) -> String {
    let Some((first, remaining)) = path.split_once('/') else {
        return path.to_owned();
    };
    if first.eq_ignore_ascii_case(category) && !remaining.is_empty() {
        remaining.to_owned()
    } else {
        path.to_owned()
    }
}

#[allow(clippy::too_many_lines)]
fn prepare_extensions(
    declarations: &[era_runtime_protocol::ExtensionDeclaration],
    diagnostics: &mut Vec<ProtocolDiagnostic>,
) -> (
    ExtensionRegistry,
    erabasic_compiler::HostRegistry,
    std::collections::BTreeMap<String, era_runtime_protocol::ExtensionDeclaration>,
) {
    use era_runtime_protocol::{ExtensionArgumentStyle, ExtensionCallableKind, ExtensionValueType};
    let builtins = builtin_instruction_names()
        .into_iter()
        .chain(builtin_function_names())
        .collect::<std::collections::BTreeSet<_>>();
    let mut analyzer = ExtensionRegistry::default();
    let mut hosts = default_host_registry();
    let mut map = std::collections::BTreeMap::new();
    let mut ids = std::collections::BTreeSet::new();
    for declaration in declarations {
        let name = declaration.era_name.to_ascii_uppercase();
        let operation = declaration.operation.to_ascii_lowercase();
        let invalid = declaration.id.is_empty()
            || name.is_empty()
            || operation.is_empty()
            || builtins.contains(&name)
            || map.contains_key(&operation)
            || !ids.insert(declaration.id.clone())
            || declaration
                .arguments
                .iter()
                .any(|argument| argument.value_type == ExtensionValueType::Void)
            || declaration
                .arguments
                .windows(2)
                .any(|pair| pair[0].optional && !pair[1].optional)
            || declaration.variadic && declaration.arguments.is_empty()
            || matches!(
                (declaration.kind, declaration.return_type),
                (
                    ExtensionCallableKind::Instruction,
                    ExtensionValueType::Integer
                        | ExtensionValueType::String
                        | ExtensionValueType::Any
                ) | (
                    ExtensionCallableKind::Function,
                    ExtensionValueType::Void | ExtensionValueType::Any
                )
            )
            || declaration.kind == ExtensionCallableKind::Function
                && declaration.argument_style != ExtensionArgumentStyle::Normal;
        if invalid {
            diagnostics.push(project_diagnostic(
                "runtime.invalid_extension_declaration",
                DiagnosticSeverity::Error,
                format!(
                    "extension declaration {:?} is empty, duplicated, or conflicts with a built-in",
                    declaration.id
                ),
                None,
            ));
            continue;
        }
        let constraints = declaration
            .arguments
            .iter()
            .map(|argument| match (argument.mutable, argument.value_type) {
                (true, ExtensionValueType::Integer) => ArgumentConstraint::MutableInteger,
                (true, ExtensionValueType::String) => ArgumentConstraint::MutableString,
                (true, ExtensionValueType::Any | ExtensionValueType::Void) => {
                    ArgumentConstraint::MutableAny
                }
                (false, ExtensionValueType::Integer) => ArgumentConstraint::Integer,
                (false, ExtensionValueType::String) => ArgumentConstraint::String,
                (false, ExtensionValueType::Any | ExtensionValueType::Void) => {
                    ArgumentConstraint::Any
                }
            })
            .collect::<Vec<_>>();
        let minimum_arguments = declaration
            .arguments
            .iter()
            .take_while(|argument| !argument.optional)
            .count();
        let return_type = match declaration.return_type {
            ExtensionValueType::Integer => SemanticType::Integer,
            ExtensionValueType::String => SemanticType::String,
            ExtensionValueType::Void => SemanticType::Void,
            ExtensionValueType::Any => SemanticType::Error,
        };
        let registered = match declaration.kind {
            ExtensionCallableKind::Instruction => {
                analyzer.register_instruction(InstructionSignature {
                    name: name.clone(),
                    argument_style: match declaration.argument_style {
                        ExtensionArgumentStyle::Normal => ArgumentStyle::Expressions,
                        ExtensionArgumentStyle::Formatted => ArgumentStyle::Formatted,
                        ExtensionArgumentStyle::Raw => ArgumentStyle::Raw,
                    },
                    arguments: constraints,
                    minimum_arguments,
                    variadic: declaration.variadic,
                    allow_omitted: declaration
                        .arguments
                        .iter()
                        .any(|argument| argument.optional),
                })
            }
            ExtensionCallableKind::Function => analyzer.register_function(CallableSignature {
                name: name.clone(),
                return_type,
                arguments: constraints,
                minimum_arguments,
                variadic: declaration.variadic,
                allow_omitted: declaration
                    .arguments
                    .iter()
                    .any(|argument| argument.optional),
            }),
        };
        if !registered {
            diagnostics.push(project_diagnostic(
                "runtime.duplicate_extension_name",
                DiagnosticSeverity::Error,
                format!("duplicate extension callable {name}"),
                None,
            ));
            continue;
        }
        let mut binding = extension_binding(&name);
        binding.name.clone_from(&operation);
        binding.abi_version = u32::from(declaration.operation_version.major);
        hosts.register(name, binding);
        map.insert(operation, declaration.clone());
    }
    (analyzer, hosts, map)
}

pub(crate) fn apply_project_delta(
    current: &ProjectManifest,
    reload: &ReloadProject,
) -> Result<ProjectManifest, String> {
    if reload.base_revision != current.project_revision {
        return Err("reload base revision differs from the loaded project".into());
    }
    if reload.target_revision <= reload.base_revision {
        return Err("reload target revision must increase monotonically".into());
    }
    let mut files = std::collections::BTreeMap::new();
    for file in &current.files {
        let path = validate_relative_path(&file.relative_path).map_err(|error| error.message)?;
        files.insert(
            (file.category as u8, path.to_ascii_lowercase()),
            file.clone(),
        );
    }
    let mut changed = std::collections::BTreeSet::new();
    for change in &reload.changes {
        let (category, path) = match change {
            FileChange::Upsert { file } => (file.category, file.relative_path.as_str()),
            FileChange::Remove {
                category,
                relative_path,
            } => (*category, relative_path.as_str()),
        };
        let path = validate_relative_path(path).map_err(|error| error.message)?;
        let identity = (category as u8, path.to_ascii_lowercase());
        if !changed.insert(identity.clone()) {
            return Err("reload contains duplicate changes for one normalized path".into());
        }
        match change {
            FileChange::Upsert { file } => {
                let mut file = file.clone();
                file.relative_path = path;
                files.insert(identity, file);
            }
            FileChange::Remove { .. } => {
                files.remove(&identity);
            }
        }
    }
    Ok(ProjectManifest {
        project_revision: reload.target_revision,
        files: files.into_values().collect(),
    })
}

fn normalize_resource(
    diagnostics: &mut Vec<ProtocolDiagnostic>,
    relative_path: String,
    category: FileCategory,
    payload: &FilePayload,
) -> Option<NormalizedResourceIdentity> {
    let location = Some(SourceLocation {
        relative_path: relative_path.clone(),
        byte_start: 0,
        byte_end: 0,
        line: None,
        byte_column: None,
    });
    let bytes = match payload {
        FilePayload::Utf8(value) => value.as_bytes(),
        FilePayload::Bytes(_) if category == FileCategory::ResourceManifest => {
            diagnostics.push(project_diagnostic(
                "runtime.expected_utf8",
                DiagnosticSeverity::Error,
                "resource manifests must be submitted as UTF-8",
                location,
            ));
            return None;
        }
        FilePayload::Bytes(value) => value.as_slice(),
        FilePayload::IoError(error) => {
            diagnostics.push(project_diagnostic(
                "runtime.frontend_io_error",
                DiagnosticSeverity::Error,
                format!("frontend resource read failed: {:?}", error.kind),
                location,
            ));
            return None;
        }
    };
    Some(NormalizedResourceIdentity {
        relative_path,
        category,
        payload_digest: *blake3::hash(bytes).as_bytes(),
    })
}

fn project_identity(
    artifact: &ValidatedArtifact,
    config: &SemanticConfig,
    resources: &[NormalizedResourceIdentity],
    extensions: &std::collections::BTreeMap<String, era_runtime_protocol::ExtensionDeclaration>,
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key("rustyera.runtime.project.v2");
    hasher.update(&artifact.artifact().manifest.artifact_id.bytes());
    hasher.update(&[
        u8::from(config.auto_save),
        u8::from(config.save_in_binary),
        u8::from(config.compress_save),
        u8::from(config.money_first),
        u8::from(config.ctrl_z_enabled),
        u8::from(config.allow_long_input_by_activation),
    ]);
    hasher.update(&config.save_slot_count.to_le_bytes());
    hasher.update(&config.maximum_shop_items.to_le_bytes());
    hasher.update(&config.viewport_width.to_le_bytes());
    hasher.update(&config.viewport_height.to_le_bytes());
    hasher.update(&config.font_size.to_le_bytes());
    hasher.update(&config.line_height.to_le_bytes());
    hasher.update(&config.print_c_per_line.to_le_bytes());
    hasher.update(&config.print_c_length.to_le_bytes());
    hasher.update(&(config.money_label.len() as u64).to_le_bytes());
    hasher.update(config.money_label.as_bytes());
    // GETCONFIG exposes the complete catalog, including frontend-only preferences.
    // Hash every canonical entry so two projects with observably different query
    // results can never share a snapshot or hot-reload identity.
    for (code, value) in config.values.iter() {
        hasher.update(&(code.len() as u64).to_le_bytes());
        hasher.update(code.as_bytes());
        let encoded = serde_json::to_vec(value).expect("configuration value serializes");
        hasher.update(&(encoded.len() as u64).to_le_bytes());
        hasher.update(&encoded);
    }
    for resource in resources {
        hasher.update(&(resource.relative_path.len() as u64).to_le_bytes());
        hasher.update(resource.relative_path.as_bytes());
        hasher.update(&[resource.category as u8]);
        hasher.update(&resource.payload_digest);
    }
    for (operation, declaration) in extensions {
        hasher.update(&(operation.len() as u64).to_le_bytes());
        hasher.update(operation.as_bytes());
        let encoded = serde_json::to_vec(declaration).expect("extension declaration serializes");
        hasher.update(&(encoded.len() as u64).to_le_bytes());
        hasher.update(&encoded);
    }
    *hasher.finalize().as_bytes()
}

fn inspect_deferred_file(
    diagnostics: &mut Vec<ProtocolDiagnostic>,
    path: &str,
    payload: &FilePayload,
    require_utf8: bool,
    code: &str,
    message: &str,
) {
    let location = || SourceLocation {
        relative_path: path.into(),
        byte_start: 0,
        byte_end: 0,
        line: None,
        byte_column: None,
    };
    match payload {
        FilePayload::IoError(error) => diagnostics.push(project_diagnostic(
            "runtime.frontend_io_error",
            DiagnosticSeverity::Error,
            error.message.clone(),
            Some(location()),
        )),
        FilePayload::Bytes(_) if require_utf8 => diagnostics.push(project_diagnostic(
            "runtime.expected_utf8",
            DiagnosticSeverity::Error,
            "configuration and resource manifests must be submitted as UTF-8",
            Some(location()),
        )),
        FilePayload::Utf8(_) | FilePayload::Bytes(_) => diagnostics.push(project_diagnostic(
            code,
            DiagnosticSeverity::Warning,
            message,
            Some(location()),
        )),
    }
}

fn failed(
    revision: u64,
    diagnostics: Vec<ProtocolDiagnostic>,
    previous: Option<&IncrementalState>,
) -> ProjectBuild {
    failed_with_incremental(revision, diagnostics, previous.cloned().unwrap_or_default())
}

fn failed_with_incremental(
    revision: u64,
    diagnostics: Vec<ProtocolDiagnostic>,
    incremental: IncrementalState,
) -> ProjectBuild {
    ProjectBuild {
        artifact: None,
        incremental,
        report: ProjectLoadReport {
            project_revision: revision,
            success: false,
            diagnostics,
            payload_required: false,
        },
        snapshot: None,
    }
}

fn path_has_priority_directory(path: &str) -> bool {
    path.replace('\\', "/")
        .split('/')
        .rev()
        .skip(1)
        .any(|component| component.contains('#'))
}

#[allow(clippy::too_many_lines)]
fn parse_configuration(
    files: &[era_runtime_protocol::SubmittedFile],
    diagnostics: &mut Vec<ProtocolDiagnostic>,
) -> SemanticConfig {
    let mut config = SemanticConfig::default();
    let mut configuration_files = files
        .iter()
        .filter(|file| file.category == FileCategory::Configuration)
        .collect::<Vec<_>>();
    // Emuera has a semantic precedence independent of frontend submission order.
    configuration_files.sort_by_key(|file| configuration_precedence(&file.relative_path));
    for file in configuration_files {
        let FilePayload::Utf8(text) = &file.payload else {
            inspect_deferred_file(
                diagnostics,
                &file.relative_path,
                &file.payload,
                true,
                "runtime.configuration_ignored",
                "configuration payload was not UTF-8",
            );
            continue;
        };
        if parse_json_configuration(text, &file.relative_path, &mut config, diagnostics) {
            continue;
        }
        let fixed = is_fixed_configuration(&file.relative_path);
        let debug_configuration = file
            .relative_path
            .replace('\\', "/")
            .rsplit('/')
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case("debug.config"));
        for (line_index, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            let Some((name, value)) = line.split_once(':') else {
                diagnostics.push(project_diagnostic(
                    "runtime.invalid_configuration",
                    DiagnosticSeverity::Warning,
                    "configuration line has no ':' separator",
                    Some(SourceLocation {
                        relative_path: file.relative_path.clone(),
                        byte_start: 0,
                        byte_end: 0,
                        line: Some(line_index as u64 + 1),
                        byte_column: None,
                    }),
                ));
                continue;
            };
            let name = name.trim();
            let value = value.trim();
            let applied = if debug_configuration {
                config.values.apply(name, value, false)
            } else {
                config.values.apply_regular(name, value, fixed)
            };
            if let Err(error) = applied {
                diagnostics.push(project_diagnostic(
                    match error {
                        erabasic_config::ConfigParseError::UnknownKey => {
                            "runtime.unknown_configuration"
                        }
                        erabasic_config::ConfigParseError::InvalidValue => {
                            "runtime.invalid_configuration"
                        }
                    },
                    DiagnosticSeverity::Warning,
                    format!("configuration assignment {name:?} was not applied"),
                    Some(SourceLocation {
                        relative_path: file.relative_path.clone(),
                        byte_start: 0,
                        byte_end: 0,
                        line: Some(line_index as u64 + 1),
                        byte_column: None,
                    }),
                ));
            }
            match name {
                "表示するセーブデータ数" | "Save data count per page" => {
                    if let Ok(value) = value.parse::<u32>() {
                        config.save_slot_count = value.clamp(20, 80);
                    }
                    continue;
                }
                "ウィンドウ幅" | "Window width" => {
                    if let Ok(value) = value.parse::<u32>() {
                        config.viewport_width = value.max(128);
                    }
                    continue;
                }
                "ウィンドウ高さ" | "Window height" => {
                    if let Ok(value) = value.parse::<u32>() {
                        config.viewport_height = value.max(128);
                    }
                    continue;
                }
                "フォントサイズ" | "Font size" => {
                    if let Ok(value) = value.parse::<u32>() {
                        config.font_size = value.max(8);
                    }
                    continue;
                }
                "一行の高さ" | "Line height" => {
                    if let Ok(value) = value.parse::<u32>() {
                        config.line_height = value.max(config.font_size);
                    }
                    continue;
                }
                "PRINTCを並べる数" | "Items per line for PRINTC" => {
                    if let Ok(value) = value.parse::<u32>() {
                        config.print_c_per_line = value.max(1);
                    }
                    continue;
                }
                "PRINTCの文字数" | "Number of Item characters for PRINTC" => {
                    if let Ok(value) = value.parse::<u32>() {
                        config.print_c_length = value.max(1);
                    }
                    continue;
                }
                _ => {}
            }
            let Some(boolean) = parse_bool(value) else {
                continue;
            };
            match name {
                "大文字小文字の違いを無視する" | "Ignore case" => {
                    config.csv.ignore_case = boolean;
                    config.analyzer.ignore_case = boolean;
                }
                "_Rename.csvを利用する" | "Use _Rename.csv file" => {
                    config.csv.use_rename_file = boolean;
                }
                "_Replace.csvを利用する" | "Use _Replace.csv file" => {
                    config.csv.use_replace_file = boolean;
                }
                "サブディレクトリを検索する" | "Search subfolders" => {
                    config.csv.search_subdirectories = boolean;
                }
                "読み込み順をファイル名順にソートする" | "Sort filenames" => {
                    config.csv.sort_with_filename = boolean;
                    config.analyzer.sort_with_filename = boolean;
                }
                "全角スペースをホワイトスペースに含める"
                | "Whitespace includes full-width space" => {
                    config.csv.allow_full_width_space = boolean;
                    config.analyzer.allow_full_width_space = boolean;
                }
                "SPキャラを使用する" | "Allow SP characters" => {
                    config.csv.compatible_sp_character = boolean;
                }
                "ERD機能を利用する" | "Use ERD" => {
                    config.csv.use_erd = boolean;
                    config.analyzer.use_erd = boolean;
                }
                "イベント関数のCALLを許可する" | "Allow CALL on event functions" => {
                    config.analyzer.compatible_call_event = boolean;
                }
                "ユーザー関数の全ての引数の省略を許可する"
                | "Allow arguments omission for user functions" => {
                    config.analyzer.compatible_function_argument_optional = boolean;
                }
                "ユーザー関数の引数に自動的にTOSTRを補完する"
                | "Auto TOSTR conversion for user function arguments" => {
                    config.analyzer.compatible_function_argument_auto_convert = boolean;
                }
                "UseNewRandom" | "新しい高速な乱数アルゴリズムを使う" => {
                    config.use_new_random = boolean;
                }
                "オートセーブを行なう" | "Make autosaves" => config.auto_save = boolean,
                "Ctrl-Zで元に戻す機能を有効にする" | "Enable undo with ctrl-z" => {
                    config.ctrl_z_enabled = boolean;
                }
                "ONEINPUT系命令でマウスによる2文字以上の入力を許可する"
                | "Allow long input by mouse for ONEINPUT" => {
                    config.allow_long_input_by_activation = boolean;
                }
                "セーブデータをバイナリ形式で保存する"
                | "Use the binary format for saving data" => config.save_in_binary = boolean,
                "セーブデータを圧縮して保存する" | "Compress save data" => {
                    config.compress_save = boolean;
                }
                _ => {}
            }
        }
    }
    if config.use_new_random {
        diagnostics.push(project_diagnostic(
            "runtime.use_new_random_ignored",
            DiagnosticSeverity::Warning,
            "UseNewRandom=true is ignored; the pinned SFMT implementation is always used",
            None,
        ));
    }
    apply_catalog_semantics(&mut config);
    config
}

#[allow(clippy::too_many_lines)]
fn apply_catalog_semantics(config: &mut SemanticConfig) {
    let boolean = |code| match config.values.get_code(code) {
        Some(ConfigValue::Boolean(value)) => Some(*value),
        _ => None,
    };
    let integer = |code| match config.values.get_code(code) {
        Some(ConfigValue::Integer(value)) => Some(*value),
        _ => None,
    };
    let string = |code| match config.values.get_code(code) {
        Some(ConfigValue::String(value) | ConfigValue::Enum { value, .. }) => Some(value.as_str()),
        _ => None,
    };
    if let Some(value) = boolean("IgnoreCase") {
        config.csv.ignore_case = value;
        config.analyzer.ignore_case = value;
    }
    if let Some(value) = boolean("UseRenameFile") {
        config.csv.use_rename_file = value;
    }
    if let Some(value) = boolean("UseReplaceFile") {
        config.csv.use_replace_file = value;
    }
    if let Some(value) = boolean("SearchSubdirectory") {
        config.csv.search_subdirectories = value;
    }
    if let Some(value) = boolean("SortWithFilename") {
        config.csv.sort_with_filename = value;
        config.analyzer.sort_with_filename = value;
    }
    if let Some(value) = boolean("CompatiCALLNAME") {
        config.csv.compatible_call_name = value;
    }
    if let Some(value) = boolean("CompatiSPChara") {
        config.csv.compatible_sp_character = value;
    }
    if let Some(value) = boolean("UseERD") {
        config.csv.use_erd = value;
        config.analyzer.use_erd = value;
    }
    if let Some(value) = boolean("SystemAllowFullSpace") {
        config.csv.allow_full_width_space = value;
        config.analyzer.allow_full_width_space = value;
    }
    if let Some(value) = boolean("SystemIgnoreTripleSymbol") {
        config.analyzer.ignore_triple_symbols = value;
    }
    if let Some(value) = string("useLanguage") {
        config.legacy_encoding = match value.to_ascii_uppercase().as_str() {
            "KOREAN" => LegacyEncoding::Korean,
            "CHINESE_HANS" => LegacyEncoding::ChineseHans,
            "CHINESE_HANT" => LegacyEncoding::ChineseHant,
            _ => LegacyEncoding::Japanese,
        };
    }
    if let Some(value) = string("ReplaceContinuationBR") {
        let value = value.trim_matches('"').to_owned();
        config.csv.continuation_separator.clone_from(&value);
        config.analyzer.continuation_separator = value;
    }

    if let Some(value) = boolean("AllowFunctionOverloading") {
        config.analyzer.allow_function_overloading = value;
    }
    if let Some(value) = boolean("WarnFunctionOverloading") {
        config.analyzer.warn_function_overloading = value;
    }
    if let Some(value) = integer("DisplayWarningLevel").and_then(|value| u8::try_from(value).ok()) {
        config.analyzer.display_warning_level = value;
    }
    if let Some(value) = boolean("IgnoreUncalledFunction") {
        config.analyzer.ignore_uncalled_functions = value;
    }
    if let Some(value) = string("FunctionNotFoundWarning").and_then(parse_warning_policy) {
        config.analyzer.function_not_found = value;
    }
    if let Some(value) = string("FunctionNotCalledWarning").and_then(parse_warning_policy) {
        config.analyzer.function_not_called = value;
    }
    if let Some(value) = boolean("CompatiFuncArgAutoConvert") {
        config.analyzer.compatible_function_argument_auto_convert = value;
    }
    if let Some(value) = boolean("CompatiFuncArgOptional") {
        config.analyzer.compatible_function_argument_optional = value;
    }
    if let Some(value) = boolean("CompatiCallEvent") {
        config.analyzer.compatible_call_event = value;
    }
    if let Some(value) = boolean("SystemSaveInBinary") {
        config.analyzer.system_save_in_binary = value;
        config.save_in_binary = value;
    }

    if let Some(value) = boolean("AutoSave") {
        config.auto_save = value;
    }
    if let Some(value) = boolean("Ctrl_Z_Enabled") {
        config.ctrl_z_enabled = value;
    }
    if let Some(value) = boolean("AllowLongInputByMouse") {
        config.allow_long_input_by_activation = value;
    }
    if let Some(value) = boolean("ZipSaveData") {
        config.compress_save = value;
    }
    if let Some(value) = integer("SaveDataNos").and_then(|value| u32::try_from(value).ok()) {
        config.save_slot_count = value.clamp(20, 80);
    }
    if let Some(value) = integer("WindowX").and_then(|value| u32::try_from(value).ok()) {
        config.viewport_width = value.max(128);
    }
    if let Some(value) = integer("WindowY").and_then(|value| u32::try_from(value).ok()) {
        config.viewport_height = value.max(128);
    }
    if let Some(value) = integer("FontSize").and_then(|value| u32::try_from(value).ok()) {
        config.font_size = value.max(8);
    }
    if let Some(value) = integer("LineHeight").and_then(|value| u32::try_from(value).ok()) {
        config.line_height = value.max(config.font_size);
    }
    if let Some(value) = integer("PrintCPerLine").and_then(|value| u32::try_from(value).ok()) {
        config.print_c_per_line = value.max(1);
    }
    if let Some(value) = integer("PrintCLength").and_then(|value| u32::try_from(value).ok()) {
        config.print_c_length = value.max(1);
    }
}

fn parse_warning_policy(value: &str) -> Option<WarningPolicy> {
    match value.to_ascii_uppercase().as_str() {
        "IGNORE" => Some(WarningPolicy::Ignore),
        "DISPLAY" => Some(WarningPolicy::Display),
        "ONCE" | "ONCEPERFILE" | "ONCE_PER_FILE" => Some(WarningPolicy::OncePerFile),
        "LATER" => Some(WarningPolicy::Later),
        _ => None,
    }
}

fn parse_json_configuration(
    text: &str,
    path: &str,
    config: &mut SemanticConfig,
    diagnostics: &mut Vec<ProtocolDiagnostic>,
) -> bool {
    let text = text.trim_start_matches('\u{feff}');
    if !text.trim_start().starts_with('{') {
        return false;
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => {
            if let Some(boolean) = value
                .get("UseNewRandom")
                .and_then(serde_json::Value::as_bool)
            {
                config.use_new_random = boolean;
            }
        }
        Err(error) => diagnostics.push(project_diagnostic(
            "runtime.invalid_json_configuration",
            DiagnosticSeverity::Warning,
            error.to_string(),
            Some(SourceLocation {
                relative_path: path.into(),
                byte_start: 0,
                byte_end: u64::try_from(text.len()).unwrap_or(u64::MAX),
                line: None,
                byte_column: None,
            }),
        )),
    }
    true
}

fn configuration_precedence(path: &str) -> (u8, String) {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let name = normalized.rsplit('/').next().unwrap_or(&normalized);
    let rank = match name {
        "_default.config" | "default.config" => 0,
        "setting.json" => 2,
        "_fixed.config" | "fixed.config" => 3,
        "debug.config" => 4,
        _ => 1,
    };
    (rank, normalized)
}

fn is_fixed_configuration(path: &str) -> bool {
    matches!(
        path.replace('\\', "/")
            .to_ascii_lowercase()
            .rsplit('/')
            .next(),
        Some("_fixed.config" | "fixed.config")
    )
}

fn sync_replace_configuration(store: &mut ConfigStore, replace: &erabasic_data::ReplaceSettings) {
    // Replace.csv is parsed by erabasic-csv, then mirrored into the unified script
    // query catalog. This avoids treating replace keys as emuera.config settings.
    let values = [
        ("MoneyLabel", replace.money_label.clone()),
        (
            "MoneyFirst",
            if replace.money_first {
                "YES".into()
            } else {
                "NO".into()
            },
        ),
        ("LoadLabel", replace.load_label.clone()),
        ("MaxShopItem", replace.max_shop_item.to_string()),
        ("DrawLineString", replace.draw_line_string.clone()),
        ("BarChar1", replace.bar_char_1.to_string()),
        ("BarChar2", replace.bar_char_2.to_string()),
        ("TitleMenuString0", replace.title_menu_string_0.clone()),
        ("TitleMenuString1", replace.title_menu_string_1.clone()),
        ("ComAbleDefault", replace.com_able_default.to_string()),
        (
            "StainDefault",
            replace
                .stain_default
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join("/"),
        ),
        ("TimeupLabel", replace.timeup_label.clone()),
        (
            "ExpLvDef",
            replace
                .exp_lv_default
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join("/"),
        ),
        (
            "PalamLvDef",
            replace
                .palam_lv_default
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join("/"),
        ),
        ("pbandDef", replace.pband_default.to_string()),
        ("RelationDef", replace.relation_default.to_string()),
    ];
    for (name, value) in values {
        let _ = store.apply(name, &value, false);
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_uppercase().as_str() {
        "YES" | "TRUE" | "1" => Some(true),
        "NO" | "FALSE" | "0" => Some(false),
        _ => None,
    }
}

fn csv_file(path: String, payload: FilePayload) -> CsvFrontendFile {
    CsvFrontendFile {
        relative_path: path,
        payload: match payload {
            FilePayload::Utf8(value) => CsvFilePayload::Utf8(value),
            FilePayload::Bytes(_) => CsvFilePayload::IoError(CsvIoError {
                kind: CsvIoErrorKind::InvalidData,
                message: "CSV and EraBasic sources must be submitted as UTF-8".into(),
            }),
            FilePayload::IoError(error) => CsvFilePayload::IoError(CsvIoError {
                kind: csv_error_kind(error.kind),
                message: error.message,
            }),
        },
    }
}

fn analyzer_source(path: String, payload: FilePayload) -> ProjectSource {
    ProjectSource {
        relative_path: path,
        payload: match payload {
            FilePayload::Utf8(value) => SourcePayload::Utf8(value),
            FilePayload::Bytes(_) => SourcePayload::IoError(SourceIoError {
                kind: SourceIoErrorKind::InvalidData,
                message: "EraBasic sources must be submitted as UTF-8".into(),
            }),
            FilePayload::IoError(error) => SourcePayload::IoError(SourceIoError {
                kind: analyzer_error_kind(error.kind),
                message: error.message,
            }),
        },
    }
}

fn csv_error_kind(kind: era_runtime_protocol::FrontendIoErrorKind) -> CsvIoErrorKind {
    match kind {
        era_runtime_protocol::FrontendIoErrorKind::NotFound => CsvIoErrorKind::NotFound,
        era_runtime_protocol::FrontendIoErrorKind::PermissionDenied => {
            CsvIoErrorKind::PermissionDenied
        }
        era_runtime_protocol::FrontendIoErrorKind::InvalidData => CsvIoErrorKind::InvalidData,
        era_runtime_protocol::FrontendIoErrorKind::Interrupted => CsvIoErrorKind::Interrupted,
        era_runtime_protocol::FrontendIoErrorKind::ReadOnly
        | era_runtime_protocol::FrontendIoErrorKind::AlreadyExists
        | era_runtime_protocol::FrontendIoErrorKind::Conflict
        | era_runtime_protocol::FrontendIoErrorKind::Other => CsvIoErrorKind::Other,
    }
}

fn analyzer_error_kind(kind: era_runtime_protocol::FrontendIoErrorKind) -> SourceIoErrorKind {
    match kind {
        era_runtime_protocol::FrontendIoErrorKind::NotFound => SourceIoErrorKind::NotFound,
        era_runtime_protocol::FrontendIoErrorKind::PermissionDenied => {
            SourceIoErrorKind::PermissionDenied
        }
        era_runtime_protocol::FrontendIoErrorKind::InvalidData => SourceIoErrorKind::InvalidData,
        era_runtime_protocol::FrontendIoErrorKind::Interrupted => SourceIoErrorKind::Interrupted,
        era_runtime_protocol::FrontendIoErrorKind::ReadOnly
        | era_runtime_protocol::FrontendIoErrorKind::AlreadyExists
        | era_runtime_protocol::FrontendIoErrorKind::Conflict
        | era_runtime_protocol::FrontendIoErrorKind::Other => SourceIoErrorKind::Other,
    }
}

fn payload_hash(payload: &FilePayload) -> Option<blake3::Hash> {
    match payload {
        FilePayload::Utf8(value) => Some(blake3::hash(value.as_bytes())),
        FilePayload::Bytes(value) => Some(blake3::hash(value.as_slice())),
        FilePayload::IoError(_) => None,
    }
}

fn project_diagnostic(
    code: &str,
    severity: DiagnosticSeverity,
    message: impl Into<String>,
    source: Option<SourceLocation>,
) -> ProtocolDiagnostic {
    ProtocolDiagnostic {
        code: code.into(),
        severity,
        message: message.into(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use era_protocol::ProtocolVersion;
    use era_runtime_protocol::{
        ExtensionArgument, ExtensionArgumentStyle, ExtensionCallableKind, ExtensionDeclaration,
        ExtensionValueType, FileCategory, FileChange, FilePayload, ProjectAnalysisRequest,
        ReloadProject, SubmittedFile,
    };

    use super::*;

    fn configuration(text: &str) -> SubmittedFile {
        SubmittedFile {
            relative_path: "emuera.config".into(),
            category: FileCategory::Configuration,
            payload: FilePayload::Utf8(text.into()),
            content_hash: None,
        }
    }

    #[test]
    fn semantic_configuration_is_applied_and_new_random_warns_once() {
        let mut diagnostics = Vec::new();
        let config = parse_configuration(
            &[configuration(
                "\u{feff}Sort filenames:YES\nIgnore case:NO\nUseNewRandom:TRUE\nMake autosaves:NO\nEnable undo with ctrl-z:YES\nAllow long input by mouse for ONEINPUT:YES\nUse the binary format for saving data:YES\nCompress save data:YES\nSave data count per page:30\nFont size:20\nLine height:22\nAllow CALL on event functions:YES\nAllow arguments omission for user functions:YES\nAuto TOSTR conversion for user function arguments:YES\nDo not process triple symbols inside FORM:YES\nDefault ANSI encoding:KOREAN\nフォント名:Test\n",
            )],
            &mut diagnostics,
        );
        assert!(config.csv.sort_with_filename);
        assert!(!config.csv.ignore_case);
        assert!(!config.analyzer.ignore_case);
        assert!(config.use_new_random);
        assert!(!config.auto_save);
        assert!(config.ctrl_z_enabled);
        assert!(config.allow_long_input_by_activation);
        assert!(config.save_in_binary);
        assert!(config.compress_save);
        assert_eq!(config.save_slot_count, 30);
        assert_eq!(config.money_label, "$");
        assert!(config.money_first);
        assert_eq!(config.maximum_shop_items, 100);
        assert_eq!(config.font_size, 20);
        assert_eq!(config.line_height, 22);
        assert!(config.analyzer.compatible_call_event);
        assert!(config.analyzer.compatible_function_argument_optional);
        assert!(config.analyzer.compatible_function_argument_auto_convert);
        assert!(config.analyzer.ignore_triple_symbols);
        assert_eq!(config.legacy_encoding, LegacyEncoding::Korean);
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "runtime.use_new_random_ignored")
                .count(),
            1
        );
    }

    #[test]
    fn setting_json_only_applies_reference_setting_fields() {
        let mut diagnostics = Vec::new();
        let config = parse_configuration(
            &[configuration(
                r#"{"UseNewRandom":true,"UseMouse":false,"AllowLongInputByMouse":true,"WindowWidth":1200,"FontSize":21,"LineHeight":19,"CompatiCallEvent":true,"CompatiFuncArgOptional":true,"CompatiFuncArgAutoConvert":true}"#,
            )],
            &mut diagnostics,
        );
        assert!(config.use_new_random);
        assert!(!config.allow_long_input_by_activation);
        assert_eq!(config.font_size, 18);
        assert_eq!(config.line_height, 19);
        assert!(!config.analyzer.compatible_call_event);
        assert!(!config.analyzer.compatible_function_argument_optional);
        assert!(!config.analyzer.compatible_function_argument_auto_convert);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "runtime.use_new_random_ignored")
        );
    }

    #[test]
    fn only_directory_components_with_hash_receive_priority() {
        assert!(path_has_priority_directory("ERB/#boot/first.erb"));
        assert!(path_has_priority_directory("ERB/a#early/first.erb"));
        assert!(!path_has_priority_directory("ERB/ordinary/#function.erb"));
        assert!(!path_has_priority_directory("root.erb"));
    }

    #[test]
    fn category_root_prefix_is_removed_only_at_internal_loader_boundary() {
        assert_eq!(
            category_relative_path("CSV/_Rename.csv", "CSV"),
            "_Rename.csv"
        );
        assert_eq!(
            category_relative_path("csv/sub/data.csv", "CSV"),
            "sub/data.csv"
        );
        assert_eq!(category_relative_path("ERB/main.erb", "ERB"), "main.erb");
        assert_eq!(
            category_relative_path("scripts/main.erb", "ERB"),
            "scripts/main.erb"
        );
        assert_eq!(category_relative_path("CSV.csv", "CSV"), "CSV.csv");
    }

    #[test]
    fn focused_eratw_system_slices_exercise_runtime_owned_save_flows() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/eraTW/ERB");
        for (relative, required) in [
            ("TITLE.ERB", &["@SYSTEM_TITLE", "LOADGAME"][..]),
            ("SHOP関連/SHOP.ERB", &["SAVEGAME", "LOADGAME"]),
            ("SYSTEM.ERB", &["@EVENTLOAD"]),
            ("ステータス表示関連/INFO.ERB", &["@SAVEINFO", "PUTFORM"]),
        ] {
            // This is a read-only corpus audit; functional behavior is covered by the small
            // controller fixtures so the 80+ MiB real project is never a default test input.
            let source = std::fs::read_to_string(root.join(relative)).expect("UTF-8 eraTW slice");
            for needle in required {
                assert!(
                    source.contains(needle),
                    "{relative} no longer contains {needle}"
                );
            }
        }
    }

    #[test]
    fn project_delta_is_monotonic_normalized_and_unique() {
        let current = ProjectManifest {
            project_revision: 4,
            files: vec![SubmittedFile {
                relative_path: "ERB\\main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("old".into()),
                content_hash: None,
            }],
        };
        let updated = apply_project_delta(
            &current,
            &ReloadProject {
                base_revision: 4,
                target_revision: 5,
                changes: vec![FileChange::Upsert {
                    file: SubmittedFile {
                        relative_path: "ERB/./main.erb".into(),
                        category: FileCategory::Erb,
                        payload: FilePayload::Utf8("new".into()),
                        content_hash: None,
                    },
                }],
            },
        )
        .unwrap();
        assert_eq!(updated.project_revision, 5);
        assert_eq!(updated.files.len(), 1);
        assert_eq!(updated.files[0].relative_path, "ERB/main.erb");
        assert!(matches!(updated.files[0].payload, FilePayload::Utf8(ref value) if value == "new"));

        let duplicate = ReloadProject {
            base_revision: 4,
            target_revision: 5,
            changes: vec![
                FileChange::Remove {
                    category: FileCategory::Erb,
                    relative_path: "ERB/main.erb".into(),
                },
                FileChange::Remove {
                    category: FileCategory::Erb,
                    relative_path: "erb\\MAIN.erb".into(),
                },
            ],
        };
        assert!(apply_project_delta(&current, &duplicate).is_err());
    }

    #[test]
    fn analysis_selection_checks_unreachable_code_without_loading_a_project() {
        let manifest = ProjectManifest {
            project_revision: 9,
            files: vec![
                SubmittedFile {
                    relative_path: "good.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@UNUSED\nRETURN\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "bad.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("this is not valid at top level".into()),
                    content_hash: None,
                },
            ],
        };
        let report = analyze_submitted_project_with_extensions(
            &ProjectAnalysisRequest {
                manifest,
                selected_erb_paths: vec!["good.erb".into()],
                debug_mode: true,
            },
            &[],
        );
        assert!(report.success, "{:?}", report.diagnostics);
        assert_eq!(report.analyzed_erb_paths, vec!["good.erb"]);
    }

    #[test]
    fn portable_extensions_participate_in_analysis_and_deterministic_host_lowering() {
        let declaration = ExtensionDeclaration {
            id: "example.echo.v1".into(),
            era_name: "EXT_ECHO".into(),
            kind: ExtensionCallableKind::Function,
            arguments: vec![ExtensionArgument {
                value_type: ExtensionValueType::String,
                mutable: false,
                optional: false,
            }],
            variadic: false,
            return_type: ExtensionValueType::String,
            argument_style: ExtensionArgumentStyle::Normal,
            operation: "example.echo".into(),
            operation_version: ProtocolVersion::new(1, 0),
        };
        let manifest = ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nRESULTS '= EXT_ECHO(\"ok\")\nRETURN\n".into(),
                ),
                content_hash: None,
            }],
        };
        let build = build_project_with_extensions(&manifest, None, None, &[declaration]);
        assert!(build.report.success, "{:?}", build.report.diagnostics);
        let artifact = build.artifact.unwrap();
        assert!(
            artifact.artifact().host_imports.iter().any(|import| {
                import.import.namespace == "rustyera.extension"
                    && import.import.name == "example.echo"
            }),
            "{:#?}",
            artifact.artifact().host_imports
        );
    }

    #[test]
    fn query_visible_configuration_participates_in_project_identity() {
        let manifest = |font_size| ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "emuera.config".into(),
                    category: FileCategory::Configuration,
                    payload: FilePayload::Utf8(format!("Font size:{font_size}\n")),
                    content_hash: None,
                },
            ],
        };
        let first = build_project(&manifest(18), None);
        let second = build_project(&manifest(19), None);
        assert!(first.report.success, "{:?}", first.report.diagnostics);
        assert!(second.report.success, "{:?}", second.report.diagnostics);
        assert_ne!(
            first.snapshot.unwrap().project_identity,
            second.snapshot.unwrap().project_identity
        );
    }

    #[test]
    fn runtime_project_build_retains_a_compact_serializable_incremental_cache() {
        use std::fmt::Write as _;

        let mut source = String::new();
        for index in 0..128 {
            write!(source, "@FUNCTION_{index}\nRESULT = {index}\nRETURN\n").unwrap();
        }
        let build = build_project(
            &ProjectManifest {
                project_revision: 1,
                files: vec![SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(source),
                    content_hash: None,
                }],
            },
            None,
        );
        assert!(build.report.success, "{:?}", build.report.diagnostics);
        let encoded = serde_json::to_vec(&build.incremental).unwrap();
        let decoded: IncrementalState = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(decoded, build.incremental);
        assert_eq!(decoded.cached_function_count(), 128);
        let encoded = String::from_utf8(encoded).unwrap();
        assert!(!encoded.contains("\"opcode\""));
        assert!(!encoded.contains("\"project_data\""));
    }
}
