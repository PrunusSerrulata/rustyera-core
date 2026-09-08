use super::*;
use era_runtime_protocol::{FrontendIoError, SqlResourceSeedV1, StorageEntry, StorageResponse};
use std::collections::BTreeMap;

const SEED_SHA: &str = "3eefa4c1f5e8eb01010ad3c3200364da0e506a639258062c0f7c52163eb0acd2";

// Exact 8192-byte SQLite 3.53.0 fixture, represented as nonzero spans to keep
// this crate self-contained without a sibling frontend checkout or generated DB.
pub(crate) fn old_seed() -> Vec<u8> {
    let mut bytes = vec![0; 8192];
    for (offset, text) in [
        (0, "53514c69746520666f726d61742033"),
        (16, "10"),
        (18, "0101"),
        (21, "402020"),
        (31, "02"),
        (43, "02"),
        (47, "04"),
        (59, "01"),
        (63, "01"),
        (100, "0d"),
        (104, "010fa9"),
        (108, "0fa9"),
        (
            4009,
            "55010617232301737461626c65736565645f6d61726b6572736565645f6d61726b657202435245415445205441424c4520736565645f6d61726b6572202876657273696f6e20494e5445474552204e4f54204e554c4c290d",
        ),
        (4100, "010ffc"),
        (4104, "0ffc"),
        (8188, "02010209"),
    ] {
        for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
            let digit = |byte: u8| {
                if byte <= b'9' {
                    byte - b'0'
                } else {
                    byte - b'a' + 10
                }
            };
            bytes[offset + index] = digit(pair[0]) * 16 + digit(pair[1]);
        }
    }
    assert_eq!(revision_hex(&hash(&bytes)).unwrap(), SEED_SHA);
    bytes
}

#[derive(Clone, Copy, Default)]
pub(crate) enum Ack {
    #[default]
    Normal,
    Lost,
    Omitted,
    OmittedUnreadable,
    Unresolved,
    Rejected,
}

#[derive(Default)]
pub(crate) struct MemoryIo {
    files: BTreeMap<String, Vec<u8>>,
    requests: Vec<StorageRequest>,
    seed: Vec<u8>,
    ack: Ack,
    quota_extra: u64,
    unreadable_pointer: bool,
    idempotent: BTreeMap<String, StorageResult>,
}

impl MemoryIo {
    fn conflicts(current: Option<&[u8]>, precondition: &StoragePrecondition) -> bool {
        match precondition {
            StoragePrecondition::Missing => current.is_some(),
            StoragePrecondition::Revision(revision) => {
                current.is_none_or(|bytes| opaque(bytes) != *revision)
            }
            StoragePrecondition::Any => panic!("unguarded SQL write"),
        }
    }

    fn cached(&self, request: &StorageRequest) -> Option<StorageResponse> {
        // Match StorageHost: an idempotency key alone selects an old ACK.
        if request.idempotency_key.is_empty() {
            return None;
        }
        self.idempotent
            .get(&request.idempotency_key)
            .map(|result| StorageResponse {
                request_id: request.request_id,
                result: result.clone(),
            })
    }

    pub(crate) fn set_ack(&mut self, ack: Ack) {
        self.ack = ack;
    }

    pub(crate) fn published_files(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.files
    }

    pub(crate) fn seed(seed: Vec<u8>) -> Self {
        Self {
            seed,
            ..Self::default()
        }
    }
}

fn opaque(bytes: &[u8]) -> String {
    format!("opaque/{}", hex(hash(bytes).sha256.as_slice()))
}
fn io_error(kind: FrontendIoErrorKind) -> StorageResult {
    StorageResult::Error {
        error: FrontendIoError {
            kind,
            message: "injected".into(),
            platform_code: None,
        },
    }
}

impl SqlStorage for MemoryIo {
    fn handle(&mut self, request: StorageRequest) -> StorageResponse {
        self.requests.push(request.clone());
        if let Some(response) = self.cached(&request) {
            return response;
        }
        let current = self.files.get(&request.relative_path);
        let result = match &request.operation {
            StorageOperation::Read if request.namespace == StorageNamespace::Resource => {
                StorageResult::Read {
                    data: ProtocolBytes::new(self.seed.clone()),
                    revision: None,
                }
            }
            StorageOperation::Read
                if self.unreadable_pointer && request.relative_path.ends_with("/current") =>
            {
                io_error(FrontendIoErrorKind::Interrupted)
            }
            StorageOperation::Read => current.map_or_else(
                || io_error(FrontendIoErrorKind::NotFound),
                |bytes| StorageResult::Read {
                    data: ProtocolBytes::new(bytes.clone()),
                    revision: Some(opaque(bytes)),
                },
            ),
            StorageOperation::List { .. } => {
                let mut entries: Vec<_> = self
                    .files
                    .iter()
                    .filter(|(path, _)| path.starts_with(&format!("{}/", request.relative_path)))
                    .map(|(path, bytes)| StorageEntry {
                        relative_path: path.clone(),
                        byte_length: bytes.len() as u64,
                        revision: None,
                        change_token: None,
                    })
                    .collect();
                if self.quota_extra > 0 {
                    entries.push(StorageEntry {
                        relative_path: "other.sqlite3".into(),
                        byte_length: self.quota_extra,
                        revision: None,
                        change_token: None,
                    });
                }
                StorageResult::Listed { entries }
            }
            StorageOperation::Write {
                data,
                precondition,
                atomic_replace,
            } => {
                assert!(*atomic_replace);
                assert_eq!(request.namespace, StorageNamespace::Data);
                let conflict = Self::conflicts(current.map(Vec::as_slice), precondition);
                if conflict {
                    io_error(FrontendIoErrorKind::Conflict)
                } else {
                    let pointer = request.relative_path.ends_with("/current");
                    let ack = if pointer { self.ack } else { Ack::Normal };
                    if matches!(ack, Ack::Rejected) {
                        io_error(FrontendIoErrorKind::PermissionDenied)
                    } else if matches!(ack, Ack::Unresolved) {
                        io_error(FrontendIoErrorKind::Interrupted)
                    } else {
                        self.files
                            .insert(request.relative_path.clone(), data.as_slice().to_vec());
                        match ack {
                            Ack::Lost => io_error(FrontendIoErrorKind::Interrupted),
                            Ack::Omitted => StorageResult::Written { revision: None },
                            Ack::OmittedUnreadable => {
                                self.unreadable_pointer = true;
                                StorageResult::Written { revision: None }
                            }
                            _ => StorageResult::Written {
                                revision: Some(opaque(data.as_slice())),
                            },
                        }
                    }
                }
            }
            _ => panic!("unexpected SQL storage operation"),
        };
        if !request.idempotency_key.is_empty()
            && matches!(
                request.operation,
                StorageOperation::Write { .. } | StorageOperation::Delete { .. }
            )
        {
            self.idempotent
                .insert(request.idempotency_key, result.clone());
        }
        StorageResponse {
            request_id: request.request_id,
            result,
        }
    }
}

fn fixture() -> (SqlDatabaseIdentityV1, MemoryIo) {
    let seed = old_seed();
    (
        SqlDatabaseIdentityV1 {
            sqlite_version: "3.53.4".into(),
            format_version: 1,
            source: SqlDatabaseSourceV1::ResourceSeed(SqlResourceSeedV1 {
                resource_id: "plugins/qol_data.db".into(),
                sha256: hash(&seed).sha256,
            }),
        },
        MemoryIo {
            seed,
            ..MemoryIo::default()
        },
    )
}

#[test]
fn host_cache_replays_written_by_key_without_writing_new_content() {
    let mut io = MemoryIo::default();
    let request = StorageRequest {
        request_id: 1,
        namespace: StorageNamespace::Data,
        relative_path: "first".into(),
        operation: write(b"first".to_vec(), StoragePrecondition::Missing),
        idempotency_key: "reused".into(),
        deadline_ns: None,
    };
    let first = io.handle(request.clone());
    assert!(matches!(first.result, StorageResult::Written { .. }));
    let replay = io.handle(StorageRequest {
        request_id: 2,
        relative_path: "second".into(),
        operation: write(b"second".to_vec(), StoragePrecondition::Missing),
        ..request
    });
    assert_eq!(replay.request_id, 2);
    assert_eq!(replay.result, first.result);
    assert!(!io.published_files().contains_key("second"));
}

#[test]
fn separate_and_reset_stores_publish_actual_blobs_and_pointers() {
    for kind in [ChainKind::Memory, ChainKind::Resource] {
        for different_identity in [false, true] {
            let mut io = MemoryIo::default();
            let mut first_store = RevisionStore::default();
            let mut second_store = RevisionStore::default();
            let mut chain = SqlChain {
                kind,
                identity: hex(hash(b"first identity").sha256.as_slice()),
                current_database_revision: None,
                current_storage_revision: None,
            };
            let mut expected_files = BTreeMap::new();
            for (index, bytes) in [b"first".as_slice(), b"second", b"reset"]
                .into_iter()
                .enumerate()
            {
                if different_identity && index > 0 {
                    chain = SqlChain {
                        kind,
                        identity: hex(hash(bytes).sha256.as_slice()),
                        current_database_revision: None,
                        current_storage_revision: None,
                    };
                }
                let store = if index == 1 {
                    &mut second_store
                } else {
                    if index == 2 {
                        first_store = RevisionStore::default();
                    }
                    &mut first_store
                };
                let expected = chain.current_database_revision.clone();
                let revision = hash(bytes);
                store
                    .publish(&mut chain, expected.as_ref(), bytes, &revision, &mut io)
                    .unwrap();
                let digest = revision_hex(&revision).unwrap();
                expected_files.insert(blob_path(&chain.identity, &digest), bytes.to_vec());
                assert_eq!(chain.current_database_revision, Some(revision));
                if kind == ChainKind::Resource {
                    let pointer = format!("{digest}\n").into_bytes();
                    assert_eq!(chain.current_storage_revision, Some(opaque(&pointer)));
                    expected_files.insert(current_path(&chain.identity), pointer);
                }
                assert_eq!(io.published_files(), &expected_files);
            }
            assert!(
                io.requests
                    .iter()
                    .all(|request| request.idempotency_key.is_empty())
            );
            assert!(io.idempotent.is_empty());
        }
    }
}

#[test]
fn reset_store_cannot_replay_written_to_bypass_pointer_cas() {
    let mut io = MemoryIo::default();
    let mut chain = SqlChain {
        kind: ChainKind::Resource,
        identity: hex(hash(b"identity").sha256.as_slice()),
        current_database_revision: None,
        current_storage_revision: None,
    };
    let mut stale = chain.clone();
    RevisionStore::default()
        .publish(&mut chain, None, b"winner", &hash(b"winner"), &mut io)
        .unwrap();
    let failure = RevisionStore::default()
        .publish(&mut stale, None, b"loser", &hash(b"loser"), &mut io)
        .unwrap_err();
    assert_eq!(failure.code, SqlErrorCodeV1::RevisionConflict);
    assert_eq!(failure.commit_outcome, CommitOutcome::NotCommitted);
    assert!(stale.current_database_revision.is_none());
    assert!(stale.current_storage_revision.is_none());
    assert_eq!(
        io.published_files()[&current_path(&chain.identity)],
        format!("{}\n", revision_hex(&hash(b"winner")).unwrap()).into_bytes()
    );
    assert_eq!(
        io.published_files()[&blob_path(&chain.identity, &revision_hex(&hash(b"loser")).unwrap())],
        b"loser"
    );
}

#[test]
fn fixed_legacy_chain_current_exact_and_stale_publication() {
    let (identity, mut io) = fixture();
    let mut store = RevisionStore::default();
    let mut validated = false;
    let mut opened = store
        .open(
            &identity,
            "db",
            &SqlOpenRevisionV1::Current,
            &mut io,
            |bytes| {
                assert_eq!(bytes, old_seed());
                validated = true;
                Ok(())
            },
        )
        .unwrap();
    assert!(validated);
    assert_eq!(
        opened.chain.identity,
        "9e9dbe1aab07adb1c94cd73b3dba219d8685098fd91fb388e628d3e018d8a100"
    );
    let old = hash(&io.seed);
    let seed_path = blob_path(&opened.chain.identity, SEED_SHA);
    assert_eq!(io.files[&seed_path], io.seed);
    let pointer = current_path(&opened.chain.identity);
    let old_pointer = io.files[&pointer].clone();
    let next = b"next immutable revision";
    store
        .publish(&mut opened.chain, Some(&old), next, &hash(next), &mut io)
        .unwrap();
    assert!(io.requests.iter().any(|request| matches!(&request.operation, StorageOperation::Write { precondition: StoragePrecondition::Revision(token), .. } if *token == opaque(&old_pointer))));
    let current = store
        .open(
            &identity,
            "other",
            &SqlOpenRevisionV1::Current,
            &mut io,
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(current.bytes.as_deref(), Some(next.as_slice()));
    let mut exact = store
        .open(
            &identity,
            "db",
            &SqlOpenRevisionV1::Exact(old.clone()),
            &mut io,
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(exact.bytes.as_deref(), Some(io.seed.as_slice()));
    let before = io.files.clone();
    let failure = store
        .publish(
            &mut exact.chain,
            Some(&old),
            b"stale",
            &hash(b"stale"),
            &mut io,
        )
        .unwrap_err();
    assert_eq!(failure.code, SqlErrorCodeV1::RevisionConflict);
    assert_eq!(failure.commit_outcome, CommitOutcome::NotCommitted);
    assert_eq!(io.files, before);
    assert_eq!(io.files[&seed_path], old_seed());
}

#[test]
fn memory_current_is_empty_and_exact_uses_casefolded_legacy_identity() {
    let identity = SqlDatabaseIdentityV1 {
        source: SqlDatabaseSourceV1::Memory,
        sqlite_version: "3.53.4".into(),
        format_version: 1,
    };
    let mut io = MemoryIo::default();
    let mut store = RevisionStore::default();
    let mut opened = store
        .open(
            &identity,
            "TR_DB",
            &SqlOpenRevisionV1::Current,
            &mut io,
            |_| panic!("memory seed validation"),
        )
        .unwrap();
    assert!(opened.bytes.is_none());
    assert!(io.requests.is_empty());
    assert_eq!(
        opened.chain.identity,
        "394ec555498abd84d9561be06d19d8aae6d430ca1e4686494535110a0fd7cb62"
    );
    let seed = old_seed();
    let revision = hash(&seed);
    store
        .publish(&mut opened.chain, None, &seed, &revision, &mut io)
        .unwrap();
    let exact = store
        .open(
            &identity,
            "tr_db",
            &SqlOpenRevisionV1::Exact(revision),
            &mut io,
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(exact.bytes, Some(seed));
    assert_eq!(exact.chain.identity, opened.chain.identity);
    assert!(!io.files.contains_key(&current_path(&opened.chain.identity)));
    assert!(
        store
            .open(
                &identity,
                "tr_db",
                &SqlOpenRevisionV1::Current,
                &mut io,
                |_| Ok(())
            )
            .unwrap()
            .bytes
            .is_none()
    );
}

#[test]
fn orphan_exact_can_create_missing_pointer() {
    let (identity, mut io) = fixture();
    let mut store = RevisionStore::default();
    let opened = store
        .open(
            &identity,
            "db",
            &SqlOpenRevisionV1::Current,
            &mut io,
            |_| Ok(()),
        )
        .unwrap();
    io.files.remove(&current_path(&opened.chain.identity));
    let old = hash(&io.seed);
    let mut orphan = store
        .open(
            &identity,
            "db",
            &SqlOpenRevisionV1::Exact(old.clone()),
            &mut io,
            |_| Ok(()),
        )
        .unwrap();
    assert!(orphan.chain.current_storage_revision.is_none());
    store
        .publish(
            &mut orphan.chain,
            Some(&old),
            b"orphan resumed",
            &hash(b"orphan resumed"),
            &mut io,
        )
        .unwrap();
    assert!(matches!(
        &io.requests.last().unwrap().operation,
        StorageOperation::Write {
            precondition: StoragePrecondition::Missing,
            ..
        }
    ));
}

#[test]
fn seed_validation_digest_paths_and_quota_fail_before_writes() {
    let (identity, mut io) = fixture();
    let mut store = RevisionStore::default();
    assert!(
        store
            .open(
                &identity,
                "db",
                &SqlOpenRevisionV1::Current,
                &mut io,
                |_| Err(error(SqlErrorCodeV1::InvalidSource, "invalid DB"))
            )
            .is_err()
    );
    assert!(io.files.is_empty());
    io.seed[0] ^= 1;
    assert_eq!(
        store
            .open(
                &identity,
                "db",
                &SqlOpenRevisionV1::Current,
                &mut io,
                |_| panic!("digest first")
            )
            .unwrap_err()
            .code,
        SqlErrorCodeV1::InvalidSource
    );
    for path in [
        "/root",
        "a/../b",
        "a//b",
        "a\\b",
        "a:b",
        "a\0b",
        "e\u{301}.db",
    ] {
        assert!(safe_resource(path).is_err());
    }
    io.seed = old_seed();
    io.quota_extra = MAX_BYTES;
    assert_eq!(
        store
            .open(
                &identity,
                "db",
                &SqlOpenRevisionV1::Current,
                &mut io,
                |_| Ok(())
            )
            .unwrap_err()
            .code,
        SqlErrorCodeV1::DatabaseTooLarge
    );
    assert!(io.files.is_empty());
    assert!(
        io.requests
            .iter()
            .all(|r| !matches!(r.operation, StorageOperation::Write { .. }))
    );
}

#[test]
fn pointer_ack_readback_preserves_commit_outcome() {
    for ack in [
        Ack::Lost,
        Ack::Omitted,
        Ack::OmittedUnreadable,
        Ack::Unresolved,
        Ack::Rejected,
    ] {
        let (identity, mut io) = fixture();
        let mut store = RevisionStore::default();
        let mut opened = store
            .open(
                &identity,
                "db",
                &SqlOpenRevisionV1::Current,
                &mut io,
                |_| Ok(()),
            )
            .unwrap();
        let old = opened.durable_revision.clone().unwrap();
        io.set_ack(ack);
        let result = store.publish(
            &mut opened.chain,
            Some(&old),
            b"published",
            &hash(b"published"),
            &mut io,
        );
        match ack {
            Ack::Lost | Ack::Omitted => {
                result.unwrap();
                assert_eq!(
                    opened.chain.current_database_revision,
                    Some(hash(b"published"))
                );
                assert!(opened.chain.current_storage_revision.is_some());
            }
            Ack::Unresolved => {
                assert_eq!(result.unwrap_err().commit_outcome, CommitOutcome::Unknown);
            }
            Ack::OmittedUnreadable => {
                assert_eq!(result.unwrap_err().commit_outcome, CommitOutcome::Committed);
                assert_eq!(
                    opened.chain.current_database_revision,
                    Some(hash(b"published"))
                );
            }
            Ack::Rejected => assert_eq!(
                result.unwrap_err().commit_outcome,
                CommitOutcome::NotCommitted
            ),
            Ack::Normal => unreachable!(),
        }
    }
}

#[test]
fn competing_pointer_cas_never_overwrites_new_current() {
    let (identity, mut io) = fixture();
    let mut store = RevisionStore::default();
    let mut opened = store
        .open(
            &identity,
            "db",
            &SqlOpenRevisionV1::Current,
            &mut io,
            |_| Ok(()),
        )
        .unwrap();
    let old = opened.durable_revision.clone().unwrap();
    let pointer = current_path(&opened.chain.identity);
    let competing = format!("{}\n", revision_hex(&hash(b"competitor")).unwrap()).into_bytes();
    io.files.insert(pointer.clone(), competing.clone());
    let failure = store
        .publish(
            &mut opened.chain,
            Some(&old),
            b"candidate",
            &hash(b"candidate"),
            &mut io,
        )
        .unwrap_err();
    assert_eq!(failure.code, SqlErrorCodeV1::RevisionConflict);
    assert_eq!(failure.commit_outcome, CommitOutcome::NotCommitted);
    assert_eq!(io.files[&pointer], competing);
    assert_eq!(opened.chain.current_database_revision, Some(old));
}

#[test]
fn corrupt_blob_pointer_and_invalid_publication_digest_are_rejected() {
    let (identity, mut io) = fixture();
    let mut store = RevisionStore::default();
    let mut opened = store
        .open(
            &identity,
            "db",
            &SqlOpenRevisionV1::Current,
            &mut io,
            |_| Ok(()),
        )
        .unwrap();
    let old = opened.durable_revision.clone().unwrap();
    let before = io.files.clone();
    let failure = store
        .publish(
            &mut opened.chain,
            Some(&old),
            b"wrong",
            &hash(b"different"),
            &mut io,
        )
        .unwrap_err();
    assert_eq!(failure.code, SqlErrorCodeV1::InvalidState);
    assert_eq!(io.files, before);
    io.files.insert(
        blob_path(&opened.chain.identity, SEED_SHA),
        b"corrupt".to_vec(),
    );
    assert_eq!(
        store
            .open(
                &identity,
                "db",
                &SqlOpenRevisionV1::Exact(old),
                &mut io,
                |_| Ok(())
            )
            .unwrap_err()
            .code,
        SqlErrorCodeV1::StorageFailure
    );
    io.files.insert(
        current_path(&opened.chain.identity),
        b"bad pointer".to_vec(),
    );
    assert_eq!(
        store
            .open(
                &identity,
                "db",
                &SqlOpenRevisionV1::Current,
                &mut io,
                |_| Ok(())
            )
            .unwrap_err()
            .code,
        SqlErrorCodeV1::StorageFailure
    );
}
