use super::*;

#[test]
fn loadtext_decodes_bom_marked_unicode_assets_and_removes_carriage_returns() {
    let source = "<data>温柔</data>\r\n";
    let mut little_endian = vec![0xff, 0xfe];
    little_endian.extend(
        source
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let mut big_endian = vec![0xfe, 0xff];
    big_endian.extend(
        source
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>(),
    );

    assert_eq!(
        decode_load_text(&little_endian),
        Some("<data>温柔</data>\n".into())
    );
    assert_eq!(
        decode_load_text(&big_endian),
        Some("<data>温柔</data>\n".into())
    );
    assert_eq!(
        decode_load_text(b"\xef\xbb\xbf<data>ok</data>\r\n"),
        Some("<data>ok</data>\n".into())
    );
    assert_eq!(decode_load_text(&[0xff, 0xfe, 0x3c]), None);
}

#[test]
#[allow(clippy::too_many_lines)]
fn generated_configuration_can_be_confirmed_before_the_next_edit() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "configuration-upgrade-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["zh-CN".into()],
            configuration_profile: Some(ConfigurationClientProfile::Tui),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let original = "[meta]\nschema_version = 1\n[text]\nfont_size = 20\n";
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "reraconfig.toml".into(),
                    category: FileCategory::Configuration,
                    payload: FilePayload::Utf8(original.into()),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let initial = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) => report.configuration,
            _ => None,
        })
        .expect("version 1 project publishes upgraded configuration");
    assert!(initial.generated_source.is_some());
    assert_eq!(
        initial.source_digest.as_slice(),
        blake3::hash(original.as_bytes()).as_bytes(),
        "an existing configuration upgrade must retain its overwrite precondition",
    );

    submit(
        &mut session,
        2,
        RuntimeMessage::PrepareConfigurationUpdate(PrepareConfigurationUpdate {
            project_revision: initial.project_revision,
            expected_source_digest: initial.source_digest,
            changes: Vec::new(),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let prepared = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ConfigurationUpdatePrepared(value) => Some(value),
            _ => None,
        })
        .expect("empty transaction confirms generated contents");
    assert!(prepared.contents.contains("schema_version = 5"));
    submit(
        &mut session,
        3,
        RuntimeMessage::FinalizeConfigurationUpdate(FinalizeConfigurationUpdate {
            preparation_message_id: 3,
            outcome: ConfigurationUpdateOutcome::Commit,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let committed = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ConfigurationUpdateCommitted(value) => Some(value.configuration),
            _ => None,
        })
        .expect("generated contents are committed to the runtime manifest");
    assert!(committed.generated_source.is_none());
    assert_eq!(committed.source_digest, prepared.prepared_source_digest);

    submit(
        &mut session,
        4,
        RuntimeMessage::PrepareConfigurationUpdate(PrepareConfigurationUpdate {
            project_revision: committed.project_revision,
            expected_source_digest: committed.source_digest,
            changes: vec![ConfigurationChange {
                code: "MaxLog".into(),
                value: "777".into(),
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::ConfigurationUpdatePrepared(value)
            if value.contents.contains("history_lines = 777")
    )));
}
