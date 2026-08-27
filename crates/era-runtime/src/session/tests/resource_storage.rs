//! Execute text storage calls through the compiler, VM wait, and public runtime protocol.

use super::*;
use era_runtime_protocol::FrontendIoError;

struct StorageFixture {
    session: RuntimeSession,
    sequence: u64,
    messages: Vec<RuntimeMessage>,
}

impl StorageFixture {
    fn new(snake: bool, body: &str) -> Self {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "resource-storage-fixture".into(),
                features: vec![RuntimeFeature::Storage],
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["en".into()],
                configuration_profile: None,
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        let mut files = vec![SubmittedFile {
            relative_path: "resource-storage.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8(format!("@SYSTEM_TITLE\n{body}\nWAIT\nRETURN\n")),
            content_hash: None,
        }];
        let compatibility = if snake {
            files.push(SubmittedFile {
                relative_path: "reraconfig.toml".into(),
                category: FileCategory::Configuration,
                payload: FilePayload::Utf8("[meta]\nschema_version = 4\n[compatibility]\nprofile = \"emuera.skia.snake\"\n".into()),
                content_hash: None,
            });
            erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            )
        } else {
            erabasic_compat::CompatibilityIdentity::default()
        };
        submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                compatibility,
                project_revision: 1,
                files,
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let messages = drain(&mut session);
        assert_eq!(session.phase(), RuntimePhase::Ready, "{messages:#?}");
        submit(
            &mut session,
            2,
            RuntimeMessage::Start(StartRequest {
                mode: StartMode::NewGame { seed: Some(1) },
            }),
        );
        let mut fixture = Self {
            session,
            sequence: 3,
            messages: Vec::new(),
        };
        fixture.pump();
        fixture
    }

    fn pump(&mut self) {
        for _ in 0..32 {
            self.session.drive(RuntimeDriveBudget::default()).unwrap();
            self.messages.extend(drain(&mut self.session));
            if matches!(
                self.session.phase(),
                RuntimePhase::WaitingExternal | RuntimePhase::WaitingInput | RuntimePhase::Faulted
            ) {
                return;
            }
        }
        panic!("fixture made no bounded progress: {:#?}", self.messages);
    }

    fn request(&mut self, namespace: StorageNamespace) -> StorageRequest {
        assert_eq!(
            self.session.phase(),
            RuntimePhase::WaitingExternal,
            "{:#?}",
            self.messages
        );
        let requests = std::mem::take(&mut self.messages)
            .into_iter()
            .filter_map(|message| {
                if let RuntimeMessage::StorageRequest(request) = message {
                    Some(request)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 1, "{requests:#?}");
        let request = requests.into_iter().next().unwrap();
        assert_eq!(request.namespace, namespace);
        request
    }

    fn respond(&mut self, request: &StorageRequest, result: StorageResult) {
        submit(
            &mut self.session,
            self.sequence,
            RuntimeMessage::StorageResponse(StorageResponse {
                request_id: request.request_id,
                result,
            }),
        );
        self.sequence += 1;
        self.pump();
    }

    fn finished(&self) {
        assert_eq!(
            self.session.phase(),
            RuntimePhase::WaitingInput,
            "{:#?}",
            self.messages
        );
        assert!(
            !self
                .messages
                .iter()
                .any(|message| matches!(message, RuntimeMessage::StorageRequest(_))),
            "{:#?}",
            self.messages
        );
    }

    fn integer(&self, index: u64) -> i64 {
        read_runtime_integer(self.session.vm.as_ref().unwrap(), "RESULT", &[index], None).unwrap()
    }

    fn string(&self, index: u64) -> String {
        let vm = self.session.vm.as_ref().unwrap();
        let key = runtime_variable_key(vm, "RESULTS").unwrap();
        let VmValue::String(value) = vm.vm().read_variable(key, &[index], None).unwrap() else {
            panic!("RESULTS must be string");
        };
        value
    }
}

fn error(kind: FrontendIoErrorKind) -> StorageResult {
    StorageResult::Error {
        error: FrontendIoError {
            kind,
            message: "fixture storage error".into(),
            platform_code: None,
        },
    }
}

fn read(bytes: &[u8]) -> StorageResult {
    StorageResult::Read {
        data: ProtocolBytes::new(bytes.to_vec()),
        revision: None,
    }
}

fn listed(paths: &[&str]) -> StorageResult {
    StorageResult::Listed {
        entries: paths
            .iter()
            .map(|path| era_runtime_protocol::StorageEntry {
                relative_path: (*path).into(),
                byte_length: 0,
                revision: None,
                change_token: None,
            })
            .collect(),
    }
}

#[test]
fn snake_text_fallback_retains_vm_wait_and_rejects_the_previous_storage_reply() {
    let mut fixture = StorageFixture::new(
        true,
        "RESULT:11 += 1\nRESULTS:10 = %LOADTEXT(\"plugins/schema.xml\")%\nRESULT:11 += 1",
    );
    let data = fixture.request(StorageNamespace::Data);
    assert_eq!(data.relative_path, "plugins/schema.xml");
    assert!(matches!(data.operation, StorageOperation::Read));
    assert_eq!(fixture.integer(11), 1);
    fixture.respond(&data, error(FrontendIoErrorKind::NotFound));
    let resource = fixture.request(StorageNamespace::Resource);
    assert_ne!(data.request_id, resource.request_id);
    assert_eq!(data.relative_path, resource.relative_path);
    assert_eq!(fixture.integer(11), 1);
    fixture.respond(&data, read(b"late Data"));
    assert_eq!(fixture.session.phase(), RuntimePhase::WaitingExternal);
    assert!(fixture.messages.iter().any(|message| matches!(message, RuntimeMessage::CommandRejected(rejection) if rejection.code == CommandErrorCode::StaleRequest)));
    fixture.respond(&resource, read(b"\xef\xbb\xbf<schema>\r\n</schema>"));
    fixture.finished();
    assert_eq!(fixture.string(10), "<schema>\n</schema>");
    assert_eq!(fixture.integer(11), 2);
}

#[test]
fn successful_data_read_including_invalid_text_never_falls_back() {
    for (bytes, expected) in [(b"overlay".as_slice(), "overlay"), (&[0xff], "")] {
        let mut fixture =
            StorageFixture::new(true, "RESULTS:10 = %LOADTEXT(\"plugins/schema.xml\")%");
        let request = fixture.request(StorageNamespace::Data);
        fixture.respond(&request, read(bytes));
        fixture.finished();
        assert_eq!(fixture.string(10), expected);
    }
}

#[test]
fn original_text_and_snake_integer_text_keep_their_original_namespaces() {
    for (snake, expression, namespace, path) in [
        (
            false,
            "\"plugins/schema.xml\"",
            StorageNamespace::Data,
            "plugins/schema.xml",
        ),
        (true, "3", StorageNamespace::Save, "txt03.txt"),
    ] {
        let mut fixture =
            StorageFixture::new(snake, &format!("RESULTS:10 = %LOADTEXT({expression})%"));
        let request = fixture.request(namespace);
        assert_eq!(request.relative_path, path);
        fixture.respond(&request, error(FrontendIoErrorKind::NotFound));
        fixture.finished();
        assert_eq!(fixture.string(10), "");
    }
}

#[test]
fn data_failures_other_than_missing_do_not_start_resource_requests() {
    for kind in [
        FrontendIoErrorKind::PermissionDenied,
        FrontendIoErrorKind::InvalidData,
        FrontendIoErrorKind::Interrupted,
        FrontendIoErrorKind::ReadOnly,
        FrontendIoErrorKind::AlreadyExists,
        FrontendIoErrorKind::Other,
        FrontendIoErrorKind::Conflict,
    ] {
        let mut fixture = StorageFixture::new(
            true,
            "RESULTS:0 = unchanged\nRESULTS:1 = retained\nRESULTS:10 = %LOADTEXT(\"plugins/schema.xml\")%\nRESULT:10 = EXISTFILE(\"plugins/schema.xml\")\nRESULT:11 = ENUMFILES(\"plugins\")",
        );
        for operation in 0..3 {
            let request = fixture.request(StorageNamespace::Data);
            assert!(match operation {
                0 => matches!(request.operation, StorageOperation::Read),
                1 => matches!(request.operation, StorageOperation::Stat),
                _ => matches!(request.operation, StorageOperation::List { .. }),
            });
            fixture.respond(&request, error(kind));
        }
        fixture.finished();
        assert_eq!(fixture.string(10), "");
        assert_eq!(fixture.integer(10), 0);
        assert_eq!(fixture.integer(11), -1);
        assert_eq!(fixture.string(0), "unchanged");
        assert_eq!(fixture.string(1), "retained");
    }
}

#[test]
fn snake_existence_uses_resource_metadata_only_after_missing_data() {
    for exists in [false, true] {
        let mut fixture =
            StorageFixture::new(true, "RESULT:10 = EXISTFILE(\"plugins/schema.xml\")");
        let data = fixture.request(StorageNamespace::Data);
        fixture.respond(&data, error(FrontendIoErrorKind::NotFound));
        let resource = fixture.request(StorageNamespace::Resource);
        assert!(matches!(resource.operation, StorageOperation::Stat));
        fixture.respond(
            &resource,
            if exists {
                StorageResult::Metadata(era_runtime_protocol::StorageMetadata {
                    byte_length: 4,
                    revision: None,
                })
            } else {
                error(FrontendIoErrorKind::NotFound)
            },
        );
        fixture.finished();
        assert_eq!(fixture.integer(10), i64::from(exists));
    }
}

#[test]
fn snake_listing_unifies_namespaces_deterministically_with_data_spelling() {
    let mut fixture = StorageFixture::new(true, "RESULT:10 = ENUMFILES(\"plugins\", \"*.xml\", 1)");
    let data = fixture.request(StorageNamespace::Data);
    assert_eq!(data.relative_path, "plugins");
    assert!(
        matches!(&data.operation, StorageOperation::List { pattern: Some(pattern), recursive: true } if pattern == "*.xml")
    );
    fixture.respond(&data, listed(&["plugins/É.xml", "plugins/a.xml"]));
    let resource = fixture.request(StorageNamespace::Resource);
    assert_ne!(data.request_id, resource.request_id);
    assert_eq!(data.operation, resource.operation);
    fixture.respond(
        &resource,
        listed(&[
            "plugins/z.xml",
            "plugins/e\u{301}.xml",
            "plugins/deep/b.xml",
        ]),
    );
    fixture.finished();
    assert_eq!(fixture.integer(10), 4);
    assert_eq!(
        (0..4)
            .map(|index| fixture.string(index))
            .collect::<Vec<_>>(),
        [
            "plugins/a.xml",
            "plugins/deep/b.xml",
            "plugins/z.xml",
            "plugins/É.xml"
        ]
    );
}

#[test]
fn listing_not_found_is_empty_but_other_resource_failures_publish_no_partial_list() {
    for (data_result, resource_result, expected, first) in [
        (
            error(FrontendIoErrorKind::NotFound),
            listed(&["plugins/a.xml"]),
            1,
            "plugins/a.xml",
        ),
        (
            listed(&["plugins/a.xml"]),
            error(FrontendIoErrorKind::NotFound),
            1,
            "plugins/a.xml",
        ),
        (
            listed(&["plugins/a.xml"]),
            error(FrontendIoErrorKind::PermissionDenied),
            -1,
            "unchanged",
        ),
        (
            listed(&["plugins/a.xml"]),
            error(FrontendIoErrorKind::Conflict),
            -1,
            "unchanged",
        ),
    ] {
        let mut fixture = StorageFixture::new(
            true,
            "RESULTS:0 = unchanged\nRESULT:10 = ENUMFILES(\"plugins\")",
        );
        let data = fixture.request(StorageNamespace::Data);
        fixture.respond(&data, data_result);
        let resource = fixture.request(StorageNamespace::Resource);
        assert_eq!(fixture.string(0), "unchanged");
        fixture.respond(&resource, resource_result);
        fixture.finished();
        assert_eq!(fixture.integer(10), expected);
        assert_eq!(fixture.string(0), first);
    }
}

#[test]
fn invalid_listing_paths_or_namespace_collisions_fail_without_writing_results() {
    for invalid in [
        vec!["../secret"],
        vec!["/secret"],
        vec!["other/a.xml"],
        vec!["plugins/deep/a.xml"],
        vec!["plugins/A.xml", "plugins/a.xml"],
        vec!["plugins/é.xml", "plugins/e\u{301}.xml"],
    ] {
        for invalid_resource in [false, true] {
            let mut fixture = StorageFixture::new(
                true,
                "RESULTS:0 = unchanged\nRESULT:10 = ENUMFILES(\"plugins\")",
            );
            let mut request = fixture.request(StorageNamespace::Data);
            if invalid_resource {
                fixture.respond(&request, listed(&["plugins/overlay.xml"]));
                request = fixture.request(StorageNamespace::Resource);
            }
            fixture.respond(&request, listed(&invalid));
            fixture.finished();
            assert_eq!(fixture.integer(10), -1);
            assert_eq!(fixture.string(0), "unchanged");
        }
    }
}

#[test]
fn original_listing_and_character_search_do_not_merge_resources() {
    for (snake, call) in [
        (false, "ENUMFILES(\"plugins\")"),
        (true, "FIND_CHARADATA(\"*\")"),
    ] {
        let mut fixture = StorageFixture::new(snake, &format!("RESULT:10 = {call}"));
        let data = fixture.request(StorageNamespace::Data);
        fixture.respond(&data, error(FrontendIoErrorKind::NotFound));
        fixture.finished();
        assert_eq!(fixture.integer(10), -1);
    }
}

#[test]
fn snake_text_writes_only_data_or_save_and_uses_the_same_normalized_path() {
    let mut fixture = StorageFixture::new(
        true,
        "RESULT:10 = SAVETEXT(\"overlay\", \"plugins/e\u{301}.xml\")\nRESULT:11 = SAVETEXT(\"slot\", 3)",
    );
    for (namespace, path, bytes) in [
        (
            StorageNamespace::Data,
            "plugins/é.xml",
            b"overlay".as_slice(),
        ),
        (StorageNamespace::Save, "txt03.txt", b"slot".as_slice()),
    ] {
        let request = fixture.request(namespace);
        assert_eq!(request.relative_path, path);
        assert!(
            matches!(&request.operation, StorageOperation::Write { data, .. } if data.as_slice() == bytes)
        );
        fixture.respond(&request, StorageResult::Written { revision: None });
    }
    fixture.finished();
    assert_eq!(fixture.integer(10), 1);
    assert_eq!(fixture.integer(11), 1);
}
