//! Exact extraction of project files embedded in compiled `RustyEra` project caches.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use era_runtime_protocol::{FileCategory, FilePayload, ProjectManifest, validate_relative_path};

/// Maximum input accepted by the standalone tool, matching the runtime's default transfer cap.
pub const MAXIMUM_CACHE_BYTES: usize = 1024 * 1024 * 1024;

/// Command-line usage shown by the standalone extractor.
pub const USAGE: &str =
    "Usage: rustyera-project-extractor [--force] <compiled-project-v8.bin.zst> [OUTPUT_DIR]";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractOptions {
    pub input: PathBuf,
    pub output: PathBuf,
    pub force: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExtractSummary {
    pub extracted_files: usize,
    pub extracted_binary_assets: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Help,
    Extract(ExtractOptions),
}

#[derive(Debug)]
pub struct ExtractError {
    message: String,
}

impl ExtractError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ExtractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ExtractError {}

#[derive(Clone, Debug)]
struct PreparedFile {
    relative_path: String,
    contents: Vec<u8>,
    binary_asset: bool,
}

/// Parse command-line arguments after the executable name.
///
/// # Errors
///
/// Returns an error for unknown flags or an invalid positional argument count.
pub fn parse_arguments(
    arguments: impl IntoIterator<Item = OsString>,
    current_directory: &Path,
) -> Result<Command, ExtractError> {
    let mut force = false;
    let mut positional = Vec::new();
    let mut options = true;
    for argument in arguments {
        if options && argument == "--" {
            options = false;
        } else if options && (argument == "--help" || argument == "-h") {
            return Ok(Command::Help);
        } else if options && (argument == "--force" || argument == "-f") {
            force = true;
        } else if options && argument.to_string_lossy().starts_with('-') {
            return Err(ExtractError::new(format!(
                "unknown option {}; {USAGE}",
                argument.to_string_lossy()
            )));
        } else {
            positional.push(PathBuf::from(argument));
        }
    }
    if !(1..=2).contains(&positional.len()) {
        return Err(ExtractError::new(USAGE));
    }
    let output = positional
        .get(1)
        .cloned()
        .unwrap_or_else(|| current_directory.to_owned());
    Ok(Command::Extract(ExtractOptions {
        input: positional.remove(0),
        output,
        force,
    }))
}

/// Decode one compiled-project cache and extract every embedded project input.
///
/// # Errors
///
/// Returns an error for unreadable or invalid caches and for unsafe or failed output writes.
pub fn extract_cache(options: &ExtractOptions) -> Result<ExtractSummary, ExtractError> {
    let metadata = fs::metadata(&options.input).map_err(|error| {
        ExtractError::new(format!(
            "cannot inspect input {}: {error}",
            options.input.display()
        ))
    })?;
    if metadata.len() > u64::try_from(MAXIMUM_CACHE_BYTES).unwrap_or(u64::MAX) {
        return Err(ExtractError::new(format!(
            "compiled project cache exceeds the {MAXIMUM_CACHE_BYTES} byte extraction limit"
        )));
    }
    let bytes = fs::read(&options.input).map_err(|error| {
        ExtractError::new(format!(
            "cannot read input {}: {error}",
            options.input.display()
        ))
    })?;
    let manifest = era_runtime::decode_compiled_project_manifest(&bytes, MAXIMUM_CACHE_BYTES)
        .map_err(|error| {
            ExtractError::new(format!("cannot decode compiled project cache: {error}"))
        })?;
    extract_manifest(&manifest, &options.output, options.force)
}

/// Extract project files from an already decoded project manifest.
///
/// # Errors
///
/// Returns an error before writing file contents if the manifest or destination layout is unsafe.
pub fn extract_manifest(
    manifest: &ProjectManifest,
    output: &Path,
    force: bool,
) -> Result<ExtractSummary, ExtractError> {
    let files = prepare_files(manifest)?;
    prepare_output_root(output)?;
    preflight_destinations(output, &files, force)?;
    for file in &files {
        write_project_file(output, file, force)?;
    }
    Ok(ExtractSummary {
        extracted_files: files.len(),
        extracted_binary_assets: files.iter().filter(|file| file.binary_asset).count(),
    })
}

fn prepare_files(manifest: &ProjectManifest) -> Result<Vec<PreparedFile>, ExtractError> {
    let mut files = Vec::new();
    let mut normalized_paths = BTreeSet::new();
    for file in &manifest.files {
        let relative_path = validate_relative_path(&file.relative_path).map_err(|_| {
            ExtractError::new(format!("unsafe project file path {:?}", file.relative_path))
        })?;
        let collision_key = relative_path.to_lowercase();
        if !normalized_paths.insert(collision_key) {
            return Err(ExtractError::new(format!(
                "duplicate project file destination {relative_path:?}"
            )));
        }
        let (contents, binary_asset) = match (&file.category, &file.payload) {
            (FileCategory::Resource, FilePayload::Bytes(contents)) => {
                (contents.as_slice().to_vec(), true)
            }
            (
                FileCategory::Resource
                | FileCategory::Csv
                | FileCategory::Erh
                | FileCategory::Erb
                | FileCategory::Configuration
                | FileCategory::ResourceManifest,
                FilePayload::Utf8(contents),
            ) => (contents.as_bytes().to_vec(), false),
            (FileCategory::Resource, FilePayload::IoError(_)) => {
                return Err(ExtractError::new(format!(
                    "project asset {relative_path:?} contains an I/O error"
                )));
            }
            (_, FilePayload::Bytes(_)) => {
                return Err(ExtractError::new(format!(
                    "project source {relative_path:?} does not contain UTF-8 text"
                )));
            }
            (_, FilePayload::IoError(_)) => {
                return Err(ExtractError::new(format!(
                    "project source {relative_path:?} contains an I/O error"
                )));
            }
        };
        if let Some(expected) = &file.content_hash
            && expected.as_slice() != blake3::hash(&contents).as_bytes()
        {
            return Err(ExtractError::new(format!(
                "project file {relative_path:?} does not match its content hash"
            )));
        }
        files.push(PreparedFile {
            relative_path,
            contents,
            binary_asset,
        });
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let paths = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect::<BTreeSet<_>>();
    for path in &paths {
        let mut prefix = String::new();
        for part in path
            .split('/')
            .take(path.split('/').count().saturating_sub(1))
        {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if paths.contains(prefix.as_str()) {
                return Err(ExtractError::new(format!(
                    "project file path {path:?} is nested below file {prefix:?}"
                )));
            }
        }
    }
    Ok(files)
}

fn prepare_output_root(output: &Path) -> Result<(), ExtractError> {
    match fs::symlink_metadata(output) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(ExtractError::new(format!(
            "output directory {} must not be a symbolic link",
            output.display()
        ))),
        Ok(metadata) if !metadata.is_dir() => Err(ExtractError::new(format!(
            "output path {} is not a directory",
            output.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(output)
            .map_err(|error| {
                ExtractError::new(format!(
                    "cannot create output directory {}: {error}",
                    output.display()
                ))
            }),
        Err(error) => Err(ExtractError::new(format!(
            "cannot inspect output directory {}: {error}",
            output.display()
        ))),
    }
}

fn preflight_destinations(
    output: &Path,
    files: &[PreparedFile],
    force: bool,
) -> Result<(), ExtractError> {
    for file in files {
        let target = output.join(&file.relative_path);
        inspect_existing_ancestors(output, Path::new(&file.relative_path))?;
        match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ExtractError::new(format!(
                    "refusing symbolic-link destination {}",
                    target.display()
                )));
            }
            Ok(metadata) if !metadata.is_file() => {
                return Err(ExtractError::new(format!(
                    "destination {} is not a regular file",
                    target.display()
                )));
            }
            Ok(_) if !force => {
                return Err(ExtractError::new(format!(
                    "destination {} already exists; pass --force to overwrite it",
                    target.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ExtractError::new(format!(
                    "cannot inspect destination {}: {error}",
                    target.display()
                )));
            }
        }
    }
    Ok(())
}

fn inspect_existing_ancestors(output: &Path, relative: &Path) -> Result<(), ExtractError> {
    let Some(parent) = relative.parent() else {
        return Ok(());
    };
    let mut current = output.to_owned();
    for component in parent.components() {
        let Component::Normal(part) = component else {
            return Err(ExtractError::new(
                "normalized project path has an invalid component",
            ));
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ExtractError::new(format!(
                    "refusing symbolic-link output ancestor {}",
                    current.display()
                )));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(ExtractError::new(format!(
                    "output ancestor {} is not a directory",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(ExtractError::new(format!(
                    "cannot inspect output ancestor {}: {error}",
                    current.display()
                )));
            }
        }
    }
    Ok(())
}

fn write_project_file(
    output: &Path,
    project_file: &PreparedFile,
    force: bool,
) -> Result<(), ExtractError> {
    let relative = Path::new(&project_file.relative_path);
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let mut current = output.to_owned();
    for component in parent.components() {
        let Component::Normal(part) = component else {
            return Err(ExtractError::new(
                "normalized project path has an invalid component",
            ));
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(ExtractError::new(format!(
                    "unsafe output ancestor {}",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| {
                    ExtractError::new(format!(
                        "cannot create output directory {}: {error}",
                        current.display()
                    ))
                })?;
            }
            Err(error) => {
                return Err(ExtractError::new(format!(
                    "cannot inspect output directory {}: {error}",
                    current.display()
                )));
            }
        }
    }
    let target = output.join(relative);
    let mut open = OpenOptions::new();
    open.write(true);
    if force {
        open.create(true).truncate(true);
    } else {
        open.create_new(true);
    }
    let mut file = open.open(&target).map_err(|error| {
        ExtractError::new(format!(
            "cannot create output file {}: {error}",
            target.display()
        ))
    })?;
    file.write_all(&project_file.contents).map_err(|error| {
        ExtractError::new(format!(
            "cannot write output file {}: {error}",
            target.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use era_protocol::ProtocolBytes;
    use era_runtime_protocol::{FileCategory, FilePayload, SubmittedFile};

    use super::*;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rustyera-project-extractor-{}-{name}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn submitted(path: &str, category: FileCategory, payload: FilePayload) -> SubmittedFile {
        SubmittedFile {
            relative_path: path.into(),
            category,
            payload,
            content_hash: None,
        }
    }

    #[test]
    fn arguments_default_to_current_directory_and_accept_force() {
        let current = Path::new("/current");
        assert_eq!(
            parse_arguments([OsString::from("cache.bin")], current).unwrap(),
            Command::Extract(ExtractOptions {
                input: PathBuf::from("cache.bin"),
                output: current.into(),
                force: false,
            })
        );
        assert_eq!(
            parse_arguments(
                [
                    OsString::from("--force"),
                    OsString::from("cache.bin"),
                    OsString::from("output"),
                ],
                current,
            )
            .unwrap(),
            Command::Extract(ExtractOptions {
                input: PathBuf::from("cache.bin"),
                output: PathBuf::from("output"),
                force: true,
            })
        );
    }

    #[test]
    fn extracts_text_and_binary_project_files_exactly() {
        let directory = TestDirectory::new("exact");
        let text = "  ; comment\r\n@TEST\r\n\tPRINTL 界\r\n";
        let manifest = ProjectManifest {
            project_revision: 1,
            files: vec![
                submitted(
                    "ERB/nested/main.erb",
                    FileCategory::Erb,
                    FilePayload::Utf8(text.into()),
                ),
                submitted(
                    "CSV/_default.config",
                    FileCategory::Configuration,
                    FilePayload::Utf8("UseRenameFile=YES\n".into()),
                ),
                submitted(
                    "CSV/gamebase.csv",
                    FileCategory::Csv,
                    FilePayload::Utf8("Game,Test\n".into()),
                ),
                submitted(
                    "ERB/common.erh",
                    FileCategory::Erh,
                    FilePayload::Utf8("#DEFINE VALUE 1\n".into()),
                ),
                submitted(
                    "resources.csv",
                    FileCategory::ResourceManifest,
                    FilePayload::Utf8("sprite,image.png\n".into()),
                ),
                submitted(
                    "resources/nested/image.png",
                    FileCategory::Resource,
                    FilePayload::Bytes(ProtocolBytes::new(vec![0, 0xff, 1, 2])),
                ),
            ],
        };

        let summary = extract_manifest(&manifest, &directory.0, false).unwrap();

        assert_eq!(summary.extracted_files, 6);
        assert_eq!(summary.extracted_binary_assets, 1);
        assert_eq!(
            fs::read(directory.0.join("ERB/nested/main.erb")).unwrap(),
            text.as_bytes()
        );
        assert_eq!(
            fs::read(directory.0.join("resources/nested/image.png")).unwrap(),
            vec![0, 0xff, 1, 2]
        );
    }

    #[test]
    fn refuses_collisions_before_writing_and_force_overwrites_regular_files() {
        let directory = TestDirectory::new("force");
        fs::create_dir(directory.0.join("ERB")).unwrap();
        fs::write(directory.0.join("ERB/main.erb"), "old").unwrap();
        let manifest = ProjectManifest {
            project_revision: 1,
            files: vec![
                submitted(
                    "CSV/new.csv",
                    FileCategory::Csv,
                    FilePayload::Utf8("new csv".into()),
                ),
                submitted(
                    "ERB/main.erb",
                    FileCategory::Erb,
                    FilePayload::Utf8("new".into()),
                ),
                submitted(
                    "resources/icon.png",
                    FileCategory::Resource,
                    FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
                ),
            ],
        };

        assert!(extract_manifest(&manifest, &directory.0, false).is_err());
        assert_eq!(
            fs::read_to_string(directory.0.join("ERB/main.erb")).unwrap(),
            "old"
        );
        assert!(!directory.0.join("CSV/new.csv").exists());
        assert!(!directory.0.join("resources/icon.png").exists());
        extract_manifest(&manifest, &directory.0, true).unwrap();
        assert_eq!(
            fs::read_to_string(directory.0.join("ERB/main.erb")).unwrap(),
            "new"
        );
        assert_eq!(
            fs::read(directory.0.join("resources/icon.png")).unwrap(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn rejects_unsafe_duplicate_and_hash_mismatched_project_files() {
        let directory = TestDirectory::new("invalid");
        let invalid_path = ProjectManifest {
            project_revision: 1,
            files: vec![submitted(
                "../outside.erb",
                FileCategory::Erb,
                FilePayload::Utf8("@TEST\n".into()),
            )],
        };
        assert!(extract_manifest(&invalid_path, &directory.0, false).is_err());

        let duplicate = ProjectManifest {
            project_revision: 1,
            files: vec![
                submitted(
                    "ERB/Main.erb",
                    FileCategory::Erb,
                    FilePayload::Utf8(String::new()),
                ),
                submitted(
                    "erb/main.erb",
                    FileCategory::Erb,
                    FilePayload::Utf8(String::new()),
                ),
            ],
        };
        assert!(extract_manifest(&duplicate, &directory.0, false).is_err());

        let mut mismatched = submitted(
            "ERB/main.erb",
            FileCategory::Erb,
            FilePayload::Utf8("@TEST\n".into()),
        );
        mismatched.content_hash = Some(ProtocolBytes::new(vec![0; 32]));
        let mismatched = ProjectManifest {
            project_revision: 1,
            files: vec![mismatched],
        };
        assert!(extract_manifest(&mismatched, &directory.0, false).is_err());

        let mut mismatched_asset = submitted(
            "resources/image.png",
            FileCategory::Resource,
            FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
        );
        mismatched_asset.content_hash = Some(ProtocolBytes::new(vec![0; 32]));
        let mismatched_asset = ProjectManifest {
            project_revision: 1,
            files: vec![mismatched_asset],
        };
        assert!(extract_manifest(&mismatched_asset, &directory.0, false).is_err());

        let non_text = ProjectManifest {
            project_revision: 1,
            files: vec![submitted(
                "CSV/data.csv",
                FileCategory::Csv,
                FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
            )],
        };
        assert!(extract_manifest(&non_text, &directory.0, false).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symbolic_link_output_ancestors() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new("symlink");
        let outside = TestDirectory::new("outside");
        symlink(&outside.0, directory.0.join("ERB")).unwrap();
        let manifest = ProjectManifest {
            project_revision: 1,
            files: vec![submitted(
                "ERB/main.erb",
                FileCategory::Erb,
                FilePayload::Utf8("@TEST\n".into()),
            )],
        };

        assert!(extract_manifest(&manifest, &directory.0, false).is_err());
        assert!(!outside.0.join("main.erb").exists());
    }
}
