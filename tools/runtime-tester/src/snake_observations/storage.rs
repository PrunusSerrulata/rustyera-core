//! An explicitly enabled, case-local storage host for the owned COLUMNS fixture.
//! No operation reads or writes the filesystem, and no reply consults expected observations.
//! Original Data listings select one existing directory, as the real hosts do, but
//! filename matching deliberately uses the bounded snake matcher for this fixture's
//! limited patterns. Case folding, in-memory range tokens and absent filesystem
//! permissions/symlinks are simulations, not full original-host equivalence.

use std::collections::{BTreeMap, BTreeSet};

use era_protocol::ProtocolBytes;
use era_runtime_protocol::storage_pattern::{
    StoragePatternError, matches_snake_storage_pattern, validate_snake_storage_pattern,
};
use era_runtime_protocol::{
    FileCategory, FilePayload, FrontendIoError, FrontendIoErrorKind, ProjectManifest, StorageEntry,
    StorageMetadata, StorageNamespace, StorageOperation, StoragePrecondition, StorageRequest,
    StorageResponse, StorageResult,
};
use erabasic_compat::CompatibilityProfileId;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use super::AuditResult;

const FILE_LIMIT: usize = 64 * 1024 * 1024;
const RANGE_LIMIT: usize = 4 * 1024 * 1024;
const STORAGE_LIMIT: usize = 128 * 1024 * 1024;
const PATH_LIMIT: usize = 4096;
const LIST_LIMIT: usize = 100_000;
const LIST_BYTES_LIMIT: usize = 8 * 1024 * 1024;

struct File {
    path: String,
    bytes: Vec<u8>,
    revision: String,
    token: String,
}

impl File {
    fn new(path: String, bytes: Vec<u8>, generation: u64) -> Self {
        let revision = format!("sha256:{:x}", Sha256::digest(&bytes));
        let token = format!(
            "memory:{generation}:{:x}:{revision}",
            Sha256::digest(path.as_bytes())
        );
        Self {
            path,
            bytes,
            revision,
            token,
        }
    }

    fn entry(&self) -> StorageEntry {
        StorageEntry {
            relative_path: self.path.clone(),
            byte_length: self.bytes.len() as u64,
            revision: Some(self.revision.clone()),
            change_token: Some(self.token.clone()),
        }
    }
}

pub(super) struct FixtureStorage {
    snake: bool,
    resources: BTreeMap<String, File>,
    writable: BTreeMap<(StorageNamespace, String), File>,
    // Deleting a file never removes its parent directories. Keeping this separately
    // from writable files prevents original Data from falling back after deletion.
    directories: BTreeSet<(StorageNamespace, String)>,
    retained_directory_bytes: usize,
    retained_bytes: usize,
    retained_path_bytes: usize,
    generation: u64,
    mutations: BTreeMap<String, ([u8; 32], StorageResult)>,
}

impl FixtureStorage {
    pub(super) fn from_manifest(manifest: &ProjectManifest) -> AuditResult<Self> {
        let mut storage = Self {
            snake: manifest.compatibility.profile == CompatibilityProfileId::EmueraSkiaSnake,
            resources: BTreeMap::new(),
            writable: BTreeMap::new(),
            retained_bytes: 0,
            directories: BTreeSet::new(),
            retained_directory_bytes: 0,
            retained_path_bytes: 0,
            generation: 0,
            mutations: BTreeMap::new(),
        };
        for source in manifest
            .files
            .iter()
            .filter(|file| file.category == FileCategory::Resource)
        {
            let path = normalize(&source.relative_path, false).map_err(|error| error.message)?;
            let bytes = match &source.payload {
                FilePayload::Utf8(text) => text.as_bytes(),
                FilePayload::Bytes(bytes) => bytes.as_slice(),
                _ => {
                    return Err(format!(
                        "fixture resource {} must contain owned inline bytes",
                        source.relative_path
                    )
                    .into());
                }
            };
            if bytes.len() > FILE_LIMIT
                || storage.retained_bytes.saturating_add(bytes.len()) > STORAGE_LIMIT
                || storage.retained_path_bytes.saturating_add(path.len()) > LIST_BYTES_LIMIT
            {
                return Err("fixture resources exceed their memory budget".into());
            }
            if source
                .content_hash
                .as_ref()
                .is_some_and(|hash| hash.as_slice() != blake3::hash(bytes).as_bytes())
            {
                return Err(format!(
                    "fixture resource {} differs from its manifest hash",
                    source.relative_path
                )
                .into());
            }
            let key = path.to_lowercase();
            if storage.resources.contains_key(&key) {
                return Err(format!("fixture resource normalization collision: {path}").into());
            }
            if storage.resources.len() >= LIST_LIMIT {
                return Err("fixture resource count exceeds its limit".into());
            }
            storage.retained_bytes += bytes.len();
            storage.retained_path_bytes += path.len();
            storage
                .resources
                .insert(key, File::new(path, bytes.to_vec(), 0));
        }
        Ok(storage)
    }

    pub(super) fn respond(&mut self, request: &StorageRequest) -> StorageResponse {
        if matches!(
            request.operation,
            StorageOperation::Write { .. } | StorageOperation::Delete { .. }
        ) && request.namespace != StorageNamespace::Resource
        {
            return self.respond_mutation(request);
        }
        let result = self
            .handle(request)
            .unwrap_or_else(|error| StorageResult::Error { error });
        StorageResponse {
            request_id: request.request_id,
            result,
        }
    }

    fn respond_mutation(&mut self, request: &StorageRequest) -> StorageResponse {
        let result =
            if request.idempotency_key.len() > PATH_LIMIT || request.idempotency_key.is_empty() {
                StorageResult::Error {
                    error: io(
                        FrontendIoErrorKind::InvalidData,
                        "invalid fixture idempotency key",
                    ),
                }
            } else {
                let fingerprint = mutation_fingerprint(request);
                if let Some((previous, result)) = self.mutations.get(&request.idempotency_key) {
                    if previous == &fingerprint {
                        result.clone()
                    } else {
                        StorageResult::Error {
                            error: io(
                                FrontendIoErrorKind::Conflict,
                                "idempotency key was reused for a different operation",
                            ),
                        }
                    }
                } else if self.mutations.len() >= 1024 {
                    StorageResult::Error {
                        error: io(
                            FrontendIoErrorKind::InvalidData,
                            "fixture mutation count exceeds its limit",
                        ),
                    }
                } else {
                    let result = self
                        .handle(request)
                        .unwrap_or_else(|error| StorageResult::Error { error });
                    self.mutations.insert(
                        request.idempotency_key.clone(),
                        (fingerprint, result.clone()),
                    );
                    result
                }
            };
        StorageResponse {
            request_id: request.request_id,
            result,
        }
    }

    fn handle(&mut self, request: &StorageRequest) -> Result<StorageResult, FrontendIoError> {
        let mutation = matches!(
            request.operation,
            StorageOperation::Write { .. } | StorageOperation::Delete { .. }
        );
        if request.namespace == StorageNamespace::Resource && mutation {
            return Err(io(FrontendIoErrorKind::ReadOnly, "Resource is read-only"));
        }
        if !matches!(
            request.namespace,
            StorageNamespace::Resource
                | StorageNamespace::Data
                | StorageNamespace::Save
                | StorageNamespace::GlobalSave
        ) {
            return Err(io(
                FrontendIoErrorKind::PermissionDenied,
                "namespace is not enabled for this fixture",
            ));
        }
        let path = normalize(
            &request.relative_path,
            matches!(request.operation, StorageOperation::List { .. }),
        )?;
        let key = path.to_lowercase();
        match &request.operation {
            StorageOperation::Write {
                data, precondition, ..
            } => {
                let address = (request.namespace, key);
                let previous = self.writable.get(&address);
                check_precondition(previous, precondition)?;
                let retained_bytes = self
                    .retained_bytes
                    .saturating_sub(previous.map_or(0, |file| file.bytes.len()))
                    .saturating_add(data.as_slice().len());
                let retained_path_bytes = self
                    .retained_path_bytes
                    .saturating_sub(previous.map_or(0, |file| file.path.len()))
                    .saturating_add(path.len());
                if data.as_slice().len() > FILE_LIMIT
                    || retained_bytes > STORAGE_LIMIT
                    || retained_path_bytes > LIST_BYTES_LIMIT
                    || (previous.is_none() && self.writable.len() >= LIST_LIMIT)
                {
                    return Err(io(
                        FrontendIoErrorKind::InvalidData,
                        "fixture write exceeds its memory limit",
                    ));
                }
                let (directories, directory_bytes) =
                    self.prepare_parent_directories(request.namespace, &address.1)?;
                self.generation = self.generation.checked_add(1).ok_or_else(|| {
                    io(
                        FrontendIoErrorKind::InvalidData,
                        "fixture generation exhausted",
                    )
                })?;
                let file = File::new(path, data.as_slice().to_vec(), self.generation);
                let revision = Some(file.revision.clone());
                self.writable.insert(address, file);
                self.retained_bytes = retained_bytes;
                self.retained_path_bytes = retained_path_bytes;
                self.directories.extend(
                    directories
                        .into_iter()
                        .map(|path| (request.namespace, path)),
                );
                self.retained_directory_bytes = directory_bytes;
                Ok(StorageResult::Written { revision })
            }
            StorageOperation::Delete { precondition } => {
                let address = (request.namespace, key);
                check_precondition(self.writable.get(&address), precondition)?;
                let file = self
                    .writable
                    .remove(&address)
                    .ok_or_else(|| io(FrontendIoErrorKind::NotFound, "file does not exist"))?;
                self.retained_bytes -= file.bytes.len();
                self.retained_path_bytes -= file.path.len();
                Ok(StorageResult::Deleted)
            }
            StorageOperation::List { pattern, recursive } => {
                self.list(request.namespace, &key, pattern.as_deref(), *recursive)
            }
            operation => {
                let file = self.lookup(request.namespace, &key)?;
                match operation {
                    StorageOperation::Read => Ok(StorageResult::Read {
                        data: ProtocolBytes::new(file.bytes.clone()),
                        revision: Some(file.revision.clone()),
                    }),
                    StorageOperation::Stat => Ok(StorageResult::Metadata(StorageMetadata {
                        byte_length: file.bytes.len() as u64,
                        revision: Some(file.revision.clone()),
                    })),
                    StorageOperation::ReadRange {
                        offset,
                        maximum_bytes,
                        change_token,
                    } => {
                        if *maximum_bytes == 0 || *maximum_bytes as usize > RANGE_LIMIT {
                            return Err(io(
                                FrontendIoErrorKind::InvalidData,
                                "read range exceeds its limit",
                            ));
                        }
                        if change_token
                            .as_ref()
                            .is_some_and(|token| token != &file.token)
                        {
                            return Err(io(
                                FrontendIoErrorKind::Conflict,
                                "read range token changed",
                            ));
                        }
                        let start = usize::try_from(*offset)
                            .ok()
                            .filter(|offset| *offset <= file.bytes.len())
                            .ok_or_else(|| {
                                io(
                                    FrontendIoErrorKind::InvalidData,
                                    "read range offset is outside the file",
                                )
                            })?;
                        let end = start
                            .saturating_add(*maximum_bytes as usize)
                            .min(file.bytes.len());
                        Ok(StorageResult::ReadChunk {
                            data: ProtocolBytes::new(file.bytes[start..end].to_vec()),
                            offset: *offset,
                            complete: end == file.bytes.len(),
                            change_token: file.token.clone(),
                        })
                    }
                    _ => unreachable!("mutations and listings handled above"),
                }
            }
        }
    }

    fn lookup(&self, namespace: StorageNamespace, key: &str) -> Result<&File, FrontendIoError> {
        if namespace == StorageNamespace::Resource {
            return self.resources.get(key).ok_or_else(|| {
                io(
                    FrontendIoErrorKind::PermissionDenied,
                    "resource is not authorized by the fixture manifest",
                )
            });
        }
        self.writable
            .get(&(namespace, key.into()))
            .or_else(|| {
                (!self.snake && namespace == StorageNamespace::Data)
                    .then(|| self.resources.get(key))
                    .flatten()
            })
            .ok_or_else(|| io(FrontendIoErrorKind::NotFound, "file does not exist"))
    }

    fn prepare_parent_directories(
        &self,
        namespace: StorageNamespace,
        key: &str,
    ) -> Result<(Vec<String>, usize), FrontendIoError> {
        let mut directories = Vec::new();
        let mut bytes = self.retained_directory_bytes;
        for end in std::iter::once(0).chain(key.match_indices('/').map(|(offset, _)| offset)) {
            let directory = &key[..end];
            if self.directories.contains(&(namespace, directory.into())) {
                continue;
            }
            bytes = bytes.saturating_add(directory.len());
            if bytes > LIST_BYTES_LIMIT
                || self.directories.len().saturating_add(directories.len()) >= LIST_LIMIT
            {
                return Err(io(
                    FrontendIoErrorKind::InvalidData,
                    "fixture directory state exceeds its memory limit",
                ));
            }
            directories.push(directory.to_owned());
        }
        Ok((directories, bytes))
    }

    fn list(
        &self,
        namespace: StorageNamespace,
        directory: &str,
        pattern: Option<&str>,
        recursive: bool,
    ) -> Result<StorageResult, FrontendIoError> {
        validate_snake_storage_pattern(pattern).map_err(pattern_error)?;
        let prefix = if directory.is_empty() {
            String::new()
        } else {
            format!("{directory}/")
        };
        let use_resources = namespace == StorageNamespace::Resource
            || (!self.snake
                && namespace == StorageNamespace::Data
                && !self.directories.contains(&(namespace, directory.into())));
        // Snake Data never falls back here: runtime issues its own Resource request.
        // Original Data selects either its existing directory or project resources.
        let visible = self.resources.iter().filter(|_| use_resources).chain(
            self.writable
                .iter()
                .filter(|((file_namespace, _), _)| !use_resources && *file_namespace == namespace)
                .map(|((_, key), file)| (key, file)),
        );
        let mut entries = Vec::new();
        let mut retained_bytes = 0_usize;
        for (key, file) in visible {
            let Some(tail) = key.strip_prefix(&prefix) else {
                continue;
            };
            if tail.is_empty() || (!recursive && tail.contains('/')) {
                continue;
            }
            let name = tail.rsplit('/').next().unwrap_or(tail);
            if !matches_snake_storage_pattern(pattern, name).map_err(pattern_error)? {
                continue;
            }
            retained_bytes = retained_bytes.saturating_add(file.path.len());
            if entries.len() >= LIST_LIMIT || retained_bytes > LIST_BYTES_LIMIT {
                return Err(io(
                    FrontendIoErrorKind::InvalidData,
                    "fixture listing exceeds its limit",
                ));
            }
            entries.push(file.entry());
        }
        entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        Ok(StorageResult::Listed { entries })
    }
}

fn io(kind: FrontendIoErrorKind, message: &str) -> FrontendIoError {
    FrontendIoError {
        kind,
        message: message.into(),
        platform_code: None,
    }
}

fn pattern_error(error: StoragePatternError) -> FrontendIoError {
    io(FrontendIoErrorKind::InvalidData, &error.to_string())
}

fn mutation_fingerprint(request: &StorageRequest) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(format!(
        "{:?}\0{}\0",
        request.namespace, request.relative_path
    ));
    match &request.operation {
        StorageOperation::Write {
            data,
            atomic_replace,
            precondition,
        } => {
            hasher.update(format!("write:{atomic_replace}:{precondition:?}\0"));
            hasher.update(data.as_slice());
        }
        StorageOperation::Delete { precondition } => {
            hasher.update(format!("delete:{precondition:?}"))
        }
        _ => unreachable!("only mutations have idempotency fingerprints"),
    }
    hasher.finalize().into()
}

fn normalize(path: &str, directory: bool) -> Result<String, FrontendIoError> {
    if path.len() > PATH_LIMIT || path.contains('\0') {
        return Err(io(FrontendIoErrorKind::InvalidData, "invalid fixture path"));
    }
    if directory && matches!(path, "" | ".") {
        return Ok(String::new());
    }
    let path: String = era_runtime_protocol::validate_relative_path(path)
        .map_err(|_| io(FrontendIoErrorKind::InvalidData, "unsafe fixture path"))?
        .nfc()
        .collect();
    if path.len() > PATH_LIMIT {
        return Err(io(
            FrontendIoErrorKind::InvalidData,
            "fixture path exceeds its limit",
        ));
    }
    Ok(path)
}

fn check_precondition(
    file: Option<&File>,
    condition: &StoragePrecondition,
) -> Result<(), FrontendIoError> {
    let accepted = match condition {
        StoragePrecondition::Any => true,
        StoragePrecondition::Missing => file.is_none(),
        StoragePrecondition::Revision(revision) => {
            file.is_some_and(|file| &file.revision == revision)
        }
    };
    if accepted {
        Ok(())
    } else {
        Err(io(
            FrontendIoErrorKind::Conflict,
            "storage precondition failed",
        ))
    }
}

pub(super) fn evidence(request: &StorageRequest, response: &StorageResponse) -> Value {
    // Complete envelopes (including bytes) are in rawMessages. Keep this index bounded to
    // metadata/digests so a saved snapshot is not duplicated in every watchdog publication.
    let operation = match &request.operation {
        StorageOperation::Write {
            data,
            precondition,
            atomic_replace,
        } => {
            json!({"type":"write", "bytes":data.as_slice().len(), "sha256":format!("{:x}", Sha256::digest(data.as_slice())), "precondition":precondition, "atomicReplace":atomic_replace})
        }
        operation => serde_json::to_value(operation).expect("storage operation serializes"),
    };
    let result = match &response.result {
        StorageResult::Read { data, revision } => {
            json!({"type":"read", "bytes":data.as_slice().len(), "sha256":format!("{:x}", Sha256::digest(data.as_slice())), "revision":revision})
        }
        StorageResult::ReadChunk {
            data,
            offset,
            complete,
            change_token,
        } => {
            json!({"type":"read_chunk", "bytes":data.as_slice().len(), "sha256":format!("{:x}", Sha256::digest(data.as_slice())), "offset":offset, "complete":complete, "changeToken":change_token})
        }
        result => serde_json::to_value(result).expect("storage result serializes"),
    };
    json!({"requestId":request.request_id, "namespace":request.namespace, "path":request.relative_path, "operation":operation, "result":result})
}

#[cfg(test)]
mod tests {
    use super::*;
    use era_runtime_protocol::SubmittedFile;
    use erabasic_compat::CompatibilityIdentity;

    fn manifest(snake: bool, paths: &[&str]) -> ProjectManifest {
        ProjectManifest {
            project_revision: 1,
            compatibility: CompatibilityIdentity::for_profile(if snake {
                CompatibilityProfileId::EmueraSkiaSnake
            } else {
                CompatibilityProfileId::EmueraEm
            }),
            files: paths
                .iter()
                .map(|path| SubmittedFile {
                    relative_path: (*path).into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Utf8("resource".into()),
                    content_hash: None,
                })
                .collect(),
        }
    }

    fn request(
        id: u64,
        namespace: StorageNamespace,
        path: &str,
        operation: StorageOperation,
    ) -> StorageRequest {
        StorageRequest {
            request_id: id,
            namespace,
            relative_path: path.into(),
            operation,
            idempotency_key: format!("fixture:{id}"),
            deadline_ns: None,
        }
    }

    fn write(
        id: u64,
        namespace: StorageNamespace,
        path: &str,
        bytes: &[u8],
        precondition: StoragePrecondition,
    ) -> StorageRequest {
        request(
            id,
            namespace,
            path,
            StorageOperation::Write {
                data: ProtocolBytes::new(bytes.to_vec()),
                atomic_replace: true,
                precondition,
            },
        )
    }

    fn assert_error(response: StorageResponse, kind: FrontendIoErrorKind) {
        assert!(
            matches!(&response.result, StorageResult::Error { error } if error.kind == kind),
            "{response:?}"
        );
    }

    fn listed_paths(
        storage: &mut FixtureStorage,
        namespace: StorageNamespace,
        directory: &str,
        recursive: bool,
    ) -> Vec<String> {
        let reply = storage.respond(&request(
            90,
            namespace,
            directory,
            StorageOperation::List {
                pattern: Some("?.xml".into()),
                recursive,
            },
        ));
        let StorageResult::Listed { entries } = reply.result else {
            panic!("{reply:?}");
        };
        entries
            .into_iter()
            .map(|entry| entry.relative_path)
            .collect()
    }

    #[test]
    fn only_manifest_resources_are_readable_and_never_mutable() {
        let mut storage =
            FixtureStorage::from_manifest(&manifest(true, &["plugins/é.xml"])).unwrap();
        let read = request(
            1,
            StorageNamespace::Resource,
            "PLUGINS/e\u{301}.xml",
            StorageOperation::Read,
        );
        assert!(
            matches!(storage.respond(&read).result, StorageResult::Read { data, .. } if data.as_slice() == b"resource")
        );
        assert_error(
            storage.respond(&request(
                2,
                StorageNamespace::Data,
                "plugins/é.xml",
                StorageOperation::Read,
            )),
            FrontendIoErrorKind::NotFound,
        );
        assert_error(
            storage.respond(&request(
                3,
                StorageNamespace::Resource,
                "plugins/secret.xml",
                StorageOperation::Read,
            )),
            FrontendIoErrorKind::PermissionDenied,
        );
        assert_error(
            storage.respond(&write(
                4,
                StorageNamespace::Resource,
                "../outside",
                b"x",
                StoragePrecondition::Any,
            )),
            FrontendIoErrorKind::ReadOnly,
        );
        assert_error(
            storage.respond(&request(
                5,
                StorageNamespace::Resource,
                "plugins/é.xml",
                StorageOperation::Delete {
                    precondition: StoragePrecondition::Any,
                },
            )),
            FrontendIoErrorKind::ReadOnly,
        );
        assert!(storage.writable.is_empty());
    }

    #[test]
    fn original_data_fallback_and_case_local_namespace_isolation() {
        for snake in [false, true] {
            let manifest = manifest(snake, &["plugins/a.xml"]);
            let mut storage = FixtureStorage::from_manifest(&manifest).unwrap();
            let read = request(
                1,
                StorageNamespace::Data,
                "plugins/a.xml",
                StorageOperation::Read,
            );
            if snake {
                assert_error(storage.respond(&read), FrontendIoErrorKind::NotFound);
            } else {
                assert!(
                    matches!(storage.respond(&read).result, StorageResult::Read { data, .. } if data.as_slice() == b"resource")
                );
            }
            for (index, namespace) in [
                StorageNamespace::Save,
                StorageNamespace::GlobalSave,
                StorageNamespace::Data,
            ]
            .into_iter()
            .enumerate()
            {
                let bytes = [index as u8];
                assert!(matches!(
                    storage
                        .respond(&write(
                            10 + index as u64,
                            namespace,
                            "same.dat",
                            &bytes,
                            StoragePrecondition::Missing
                        ))
                        .result,
                    StorageResult::Written { .. }
                ));
            }
            for (index, namespace) in [
                StorageNamespace::Save,
                StorageNamespace::GlobalSave,
                StorageNamespace::Data,
            ]
            .into_iter()
            .enumerate()
            {
                assert!(
                    matches!(storage.respond(&request(20, namespace, "SAME.dat", StorageOperation::Read)).result, StorageResult::Read { data, .. } if data.as_slice() == [index as u8])
                );
            }
            let mut fresh = FixtureStorage::from_manifest(&manifest).unwrap();
            assert_error(
                fresh.respond(&request(
                    21,
                    StorageNamespace::GlobalSave,
                    "same.dat",
                    StorageOperation::Read,
                )),
                FrontendIoErrorKind::NotFound,
            );
        }
    }

    #[test]
    fn writes_deletes_and_retries_enforce_their_actual_preconditions() {
        let mut storage = FixtureStorage::from_manifest(&manifest(true, &[])).unwrap();
        let first = write(
            1,
            StorageNamespace::GlobalSave,
            "global.sav",
            b"first",
            StoragePrecondition::Missing,
        );
        let written = storage.respond(&first);
        let StorageResult::Written {
            revision: Some(revision),
        } = &written.result
        else {
            panic!("{written:?}");
        };
        assert_eq!(storage.respond(&first), written);
        assert_error(
            storage.respond(&write(
                1,
                StorageNamespace::GlobalSave,
                "global.sav",
                b"different",
                StoragePrecondition::Any,
            )),
            FrontendIoErrorKind::Conflict,
        );
        assert_error(
            storage.respond(&write(
                2,
                StorageNamespace::GlobalSave,
                "global.sav",
                b"different",
                StoragePrecondition::Missing,
            )),
            FrontendIoErrorKind::Conflict,
        );
        assert_error(
            storage.respond(&request(
                3,
                StorageNamespace::GlobalSave,
                "global.sav",
                StorageOperation::Delete {
                    precondition: StoragePrecondition::Revision("stale".into()),
                },
            )),
            FrontendIoErrorKind::Conflict,
        );
        let overwrite = storage.respond(&write(
            4,
            StorageNamespace::GlobalSave,
            "global.sav",
            b"second",
            StoragePrecondition::Revision(revision.clone()),
        ));
        let StorageResult::Written {
            revision: Some(revision),
        } = overwrite.result
        else {
            panic!("{overwrite:?}");
        };
        assert!(matches!(
            storage
                .respond(&request(
                    5,
                    StorageNamespace::GlobalSave,
                    "global.sav",
                    StorageOperation::Delete {
                        precondition: StoragePrecondition::Revision(revision)
                    }
                ))
                .result,
            StorageResult::Deleted
        ));
        assert_error(
            storage.respond(&request(
                6,
                StorageNamespace::GlobalSave,
                "global.sav",
                StorageOperation::Read,
            )),
            FrontendIoErrorKind::NotFound,
        );
    }

    #[test]
    fn ranges_track_content_changes_and_reject_out_of_bounds_requests() {
        let mut storage = FixtureStorage::from_manifest(&manifest(true, &[])).unwrap();
        storage.respond(&write(
            1,
            StorageNamespace::Data,
            "file.txt",
            b"abcdef",
            StoragePrecondition::Any,
        ));
        let first = storage.respond(&request(
            2,
            StorageNamespace::Data,
            "file.txt",
            StorageOperation::ReadRange {
                offset: 1,
                maximum_bytes: 2,
                change_token: None,
            },
        ));
        let StorageResult::ReadChunk {
            data,
            complete,
            change_token,
            ..
        } = first.result
        else {
            panic!("{first:?}");
        };
        assert_eq!(data.as_slice(), b"bc");
        assert!(!complete);
        storage.respond(&write(
            3,
            StorageNamespace::Data,
            "file.txt",
            b"abcdef",
            StoragePrecondition::Any,
        ));
        assert_error(
            storage.respond(&request(
                4,
                StorageNamespace::Data,
                "file.txt",
                StorageOperation::ReadRange {
                    offset: 3,
                    maximum_bytes: 2,
                    change_token: Some(change_token),
                },
            )),
            FrontendIoErrorKind::Conflict,
        );
        for (offset, maximum_bytes) in [(7, 1), (0, 0), (0, RANGE_LIMIT as u32 + 1)] {
            assert_error(
                storage.respond(&request(
                    5,
                    StorageNamespace::Data,
                    "file.txt",
                    StorageOperation::ReadRange {
                        offset,
                        maximum_bytes,
                        change_token: None,
                    },
                )),
                FrontendIoErrorKind::InvalidData,
            );
        }
        assert!(
            matches!(storage.respond(&request(6, StorageNamespace::Data, "file.txt", StorageOperation::ReadRange { offset: 6, maximum_bytes: 1, change_token: None })).result, StorageResult::ReadChunk { data, complete: true, .. } if data.as_slice().is_empty())
        );
    }

    #[test]
    fn listings_keep_absent_existing_and_emptied_data_directories_distinct() {
        for snake in [false, true] {
            let manifest = manifest(
                snake,
                &[
                    "plugins/z.xml",
                    "plugins/a.xml",
                    "plugins/deep/b.xml",
                    "plugins/c.txt",
                ],
            );
            let mut storage = FixtureStorage::from_manifest(&manifest).unwrap();
            for state in 0..3 {
                if state == 1 {
                    assert!(matches!(
                        storage
                            .respond(&write(
                                1,
                                StorageNamespace::Data,
                                "plugins/A.xml",
                                b"overlay",
                                StoragePrecondition::Any
                            ))
                            .result,
                        StorageResult::Written { .. }
                    ));
                } else if state == 2 {
                    assert!(matches!(
                        storage
                            .respond(&request(
                                2,
                                StorageNamespace::Data,
                                "plugins/A.xml",
                                StorageOperation::Delete {
                                    precondition: StoragePrecondition::Any
                                }
                            ))
                            .result,
                        StorageResult::Deleted
                    ));
                }
                for recursive in [false, true] {
                    let resource_paths = if recursive {
                        vec!["plugins/a.xml", "plugins/deep/b.xml", "plugins/z.xml"]
                    } else {
                        vec!["plugins/a.xml", "plugins/z.xml"]
                    };
                    let data_paths = match state {
                        0 if !snake => resource_paths.clone(),
                        1 => vec!["plugins/A.xml"],
                        _ => Vec::new(),
                    };
                    assert_eq!(
                        listed_paths(&mut storage, StorageNamespace::Data, "plugins", recursive),
                        data_paths,
                        "snake={snake} state={state}"
                    );
                    // These are separate host replies, never an emulation of runtime's union.
                    assert_eq!(
                        listed_paths(
                            &mut storage,
                            StorageNamespace::Resource,
                            "plugins",
                            recursive
                        ),
                        resource_paths
                    );
                }
                // The root is also retained after deletion, while a sibling descendant
                // which was never created still permits original directory fallback.
                let root_paths = match state {
                    0 if !snake => vec!["plugins/a.xml", "plugins/deep/b.xml", "plugins/z.xml"],
                    1 => vec!["plugins/A.xml"],
                    _ => Vec::new(),
                };
                assert_eq!(
                    listed_paths(&mut storage, StorageNamespace::Data, "", true),
                    root_paths
                );
                assert_eq!(
                    listed_paths(&mut storage, StorageNamespace::Data, "plugins/deep", true),
                    if snake {
                        Vec::<&str>::new()
                    } else {
                        vec!["plugins/deep/b.xml"]
                    }
                );
            }
        }
    }

    #[test]
    fn data_and_resource_lists_use_shared_pattern_vectors() {
        let vectors: Value =
            serde_json::from_str(include_str!("../../fixtures/snake-storage-patterns.json"))
                .unwrap();
        assert_eq!(vectors["version"], 1);
        for case in vectors["cases"].as_array().unwrap() {
            let pattern = case["pattern"].as_str();
            let name = case["name"].as_str().unwrap();
            let expected = case["expected"].as_bool();
            let matched = matches_snake_storage_pattern(pattern, name);
            if let Some(expected) = expected {
                assert_eq!(matched, Ok(expected), "{}", case["id"]);
            } else {
                assert_eq!(case["error"], "invalid_data");
                assert!(matched.is_err(), "{}", case["id"]);
            }
            // Empty basenames are useful matcher vectors, but are not filesystem
            // entries. Do not invent an authorized Resource with an empty path.
            if name.is_empty() {
                continue;
            }
            for snake in [false, true] {
                if normalize(name, false).is_err() {
                    assert!(
                        FixtureStorage::from_manifest(&manifest(snake, &[name])).is_err(),
                        "{}",
                        case["id"]
                    );
                    let mut storage = FixtureStorage::from_manifest(&manifest(snake, &[])).unwrap();
                    assert_error(
                        storage.respond(&write(
                            1,
                            StorageNamespace::Data,
                            name,
                            b"overlay",
                            StoragePrecondition::Any,
                        )),
                        FrontendIoErrorKind::InvalidData,
                    );
                    continue;
                }
                let mut storage = FixtureStorage::from_manifest(&manifest(snake, &[name])).unwrap();
                assert!(matches!(
                    storage
                        .respond(&write(
                            1,
                            StorageNamespace::Data,
                            name,
                            b"overlay",
                            StoragePrecondition::Any
                        ))
                        .result,
                    StorageResult::Written { .. }
                ));
                for namespace in [StorageNamespace::Data, StorageNamespace::Resource] {
                    let reply = storage.respond(&request(
                        2,
                        namespace,
                        "",
                        StorageOperation::List {
                            pattern: pattern.map(str::to_owned),
                            recursive: false,
                        },
                    ));
                    if let Some(expected) = expected {
                        let StorageResult::Listed { entries } = reply.result else {
                            panic!("{} {reply:?}", case["id"]);
                        };
                        assert_eq!(
                            entries.len(),
                            usize::from(expected),
                            "{} snake={snake} namespace={namespace:?}",
                            case["id"]
                        );
                    } else {
                        assert_error(reply, FrontendIoErrorKind::InvalidData);
                    }
                }
            }
        }
    }

    #[test]
    fn empty_lists_validate_patterns_and_failed_writes_do_not_create_directories() {
        for snake in [false, true] {
            let mut storage =
                FixtureStorage::from_manifest(&manifest(snake, &["plugins/a.xml"])).unwrap();
            for namespace in [StorageNamespace::Data, StorageNamespace::Resource] {
                assert_error(
                    storage.respond(&request(
                        1,
                        namespace,
                        "absent",
                        StorageOperation::List {
                            pattern: Some("bad\0*".into()),
                            recursive: true,
                        },
                    )),
                    FrontendIoErrorKind::InvalidData,
                );
            }
            storage.retained_directory_bytes = LIST_BYTES_LIMIT;
            assert_error(
                storage.respond(&write(
                    2,
                    StorageNamespace::Data,
                    "plugins/a.xml",
                    b"overlay",
                    StoragePrecondition::Any,
                )),
                FrontendIoErrorKind::InvalidData,
            );
            assert!(storage.directories.is_empty());
            assert!(storage.writable.is_empty());
            assert_eq!(
                listed_paths(&mut storage, StorageNamespace::Data, "plugins", true),
                if snake {
                    Vec::<&str>::new()
                } else {
                    vec!["plugins/a.xml"]
                }
            );
        }
    }

    #[test]
    fn manifest_collisions_hash_mismatch_and_limits_fail_explicitly() {
        assert!(
            FixtureStorage::from_manifest(&manifest(
                true,
                &["plugins/é.xml", "PLUGINS/e\u{301}.xml"]
            ))
            .is_err()
        );
        let mut invalid = manifest(true, &["plugins/a.xml"]);
        invalid.files[0].content_hash = Some(ProtocolBytes::new(vec![0; 32]));
        assert!(FixtureStorage::from_manifest(&invalid).is_err());
        let mut storage = FixtureStorage::from_manifest(&manifest(true, &[])).unwrap();
        for path in ["../outside".to_owned(), "x".repeat(PATH_LIMIT + 1)] {
            assert_error(
                storage.respond(&request(
                    1,
                    StorageNamespace::Data,
                    &path,
                    StorageOperation::Read,
                )),
                FrontendIoErrorKind::InvalidData,
            );
        }
        storage.retained_bytes = STORAGE_LIMIT;
        assert_error(
            storage.respond(&write(
                2,
                StorageNamespace::Data,
                "a",
                b"x",
                StoragePrecondition::Any,
            )),
            FrontendIoErrorKind::InvalidData,
        );
        assert!(storage.writable.is_empty());
    }
}
