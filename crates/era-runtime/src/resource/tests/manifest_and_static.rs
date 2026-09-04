use era_protocol::ProtocolBytes;
use era_runtime_protocol::{
    LogicalLength, ProjectManifest, SceneAnchorV1, SceneLayerV1, SceneOffsetV1,
    SceneScrollPolicyV1, SceneSizeV1, SceneSourceV1, SceneStateV1, SubmittedFile,
};

#[test]
fn compact_snapshot_preserves_and_validates_static_resource_identities() {
    let manifest = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "resources/opaque.bin".into(),
            category: FileCategory::Resource,
            payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3, 4])),
            content_hash: None,
        }],
    };
    let (project, diagnostics) = ResourceGraph::from_manifest(&manifest);
    assert!(diagnostics.is_empty());
    assert_eq!(project.embedded_project_bytes(), 0);
    let mut snapshot = project.compact_snapshot();
    assert_eq!(snapshot.embedded_project_bytes(), 0);
    snapshot
        .images
        .values_mut()
        .next()
        .unwrap()
        .bytes
        .extend_from_slice(&[1, 2, 3, 4]);
    snapshot.validate_project_resources(&project).unwrap();
    assert_eq!(snapshot.embedded_project_bytes(), 0);

    let changed_manifest = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision: 2,
        files: vec![SubmittedFile {
            relative_path: "resources/opaque.bin".into(),
            category: FileCategory::Resource,
            payload: FilePayload::Bytes(ProtocolBytes::new(vec![4, 3, 2, 1])),
            content_hash: None,
        }],
    };
    let (changed, diagnostics) = ResourceGraph::from_manifest(&changed_manifest);
    assert!(diagnostics.is_empty());
    assert!(snapshot.validate_project_resources(&changed).is_err());
}

#[test]
fn resolves_emuera_audio_names_from_the_sound_directory() {
    let manifest = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "sound/Theme.MP3".into(),
            category: FileCategory::Resource,
            payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
            content_hash: None,
        }],
    };
    let (graph, diagnostics) = ResourceGraph::from_manifest(&manifest);

    assert!(diagnostics.is_empty());
    assert_eq!(graph.audio_path("theme.mp3"), Some("sound/Theme.MP3"));
    assert_eq!(
        graph.audio_path("SOUND\\THEME.MP3"),
        Some("sound/Theme.MP3")
    );
    assert!(graph.contains_audio("Theme.MP3"));
    assert_eq!(graph.audio_path("missing.mp3"), None);
}

#[test]
fn content_directory_images_create_frontend_resource_backed_canvases() {
    let manifest = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision: 1,
        files: vec![
            SubmittedFile {
                relative_path: "face.png".into(),
                category: FileCategory::Resource,
                payload: FilePayload::Bytes(ProtocolBytes::new(vec![1])),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "resources/face.png".into(),
                category: FileCategory::Resource,
                payload: FilePayload::Bytes(ProtocolBytes::new(vec![2])),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "resources/missing-metadata.png".into(),
                category: FileCategory::Resource,
                payload: FilePayload::Bytes(ProtocolBytes::new(vec![3])),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "resources/oversized.png".into(),
                category: FileCategory::Resource,
                payload: FilePayload::Bytes(ProtocolBytes::new(vec![4])),
                content_hash: None,
            },
        ],
    };
    let (mut graph, diagnostics) = ResourceGraph::from_manifest(&manifest);
    assert!(diagnostics.is_empty());
    graph
        .apply_metadata(
            "face.png",
            ImageMetadataResponse {
                width: 7,
                height: 5,
                format: "png".into(),
                animated: false,
            },
        )
        .unwrap();
    graph
        .apply_metadata(
            "resources/face.png",
            ImageMetadataResponse {
                width: 32,
                height: 16,
                format: "png".into(),
                animated: false,
            },
        )
        .unwrap();
    graph
        .apply_metadata(
            "resources/oversized.png",
            ImageMetadataResponse {
                width: 8_193,
                height: 1,
                format: "png".into(),
                animated: false,
            },
        )
        .unwrap();

    assert!(graph.create_canvas_from_resource(1, "face.png"));
    assert_eq!(graph.canvas_state(1), Some((32, 16)));
    assert!(graph.create_canvas_from_resource(2, "resources/face.png"));
    assert_eq!(graph.canvas_state(2), Some((32, 16)));
    assert!(!graph.create_canvas_from_resource(3, "missing-metadata.png"));
    assert!(!graph.create_canvas_from_resource(4, "oversized.png"));

    let replay = graph.replay();
    let canvas = replay
        .canvases
        .iter()
        .find(|canvas| canvas.canvas_id == 1)
        .unwrap();
    let CanvasReplayCommand::DrawSprite { name, .. } = &canvas.commands[0] else {
        panic!("project resource canvas must replay through a frontend resource sprite");
    };
    let sprite = replay
        .sprites
        .iter()
        .find(|sprite| sprite.name == *name)
        .unwrap();
    assert_eq!(sprite.size, [32, 16]);
    assert_eq!(sprite.frames[0].resource_id, "resources/face.png");
    assert!(replay.canvases.iter().all(|canvas| {
        canvas
            .commands
            .iter()
            .all(|command| !matches!(command, CanvasReplayCommand::LoadEncodedImage { .. }))
    }));
}

#[test]
fn file_sprites_resolve_only_submitted_safe_paths_and_bind_content_digests() {
    let digest = vec![9; 32];
    let manifest = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision: 1,
        files: [
            ("root.png", vec![1]),
            ("copy.png", vec![1]),
            ("erb/sub/local.png", vec![2]),
            ("erb/link/../../outside.png", vec![3]),
        ]
        .into_iter()
        .map(|(relative_path, bytes)| SubmittedFile {
            relative_path: relative_path.into(),
            category: FileCategory::Resource,
            payload: FilePayload::Bytes(ProtocolBytes::new(bytes)),
            content_hash: matches!(relative_path, "root.png" | "copy.png")
                .then(|| ProtocolBytes::new(digest.clone())),
        })
        .collect(),
    };
    let (mut graph, diagnostics) = ResourceGraph::from_manifest(&manifest);
    assert!(diagnostics.is_empty());
    for path in ["root.png", "copy.png", "erb/sub/local.png"] {
        graph
            .apply_metadata(
                path,
                ImageMetadataResponse {
                    width: 2,
                    height: 1,
                    format: "png".into(),
                    animated: false,
                },
            )
            .unwrap();
    }

    assert!(graph.create_file_sprite("root", "root.png", Some("erb/main.erb"), false));
    assert!(graph.create_file_sprite("local", "local.png", Some("erb/sub/main.erb"), true,));
    assert!(graph.create_file_sprite("copy", "copy.png", None, false));
    assert!(graph.create_file_sprite("root", "missing.png", None, false));
    for path in ["/root.png", "../root.png", "erb/link/../../outside.png"] {
        assert!(!graph.create_file_sprite("unsafe", path, Some("erb/main.erb"), false));
    }
    assert!(!graph.create_file_sprite("no-source", "local.png", None, true));

    let replay = graph.replay();
    let root = replay
        .sprites
        .iter()
        .find(|sprite| sprite.name == "ROOT")
        .unwrap();
    let copy = replay
        .sprites
        .iter()
        .find(|sprite| sprite.name == "COPY")
        .unwrap();
    let local = replay
        .sprites
        .iter()
        .find(|sprite| sprite.name == "LOCAL")
        .unwrap();
    assert_eq!(root.size, [2, 1]);
    assert_eq!(local.frames[0].resource_id, "erb/sub/local.png");
    assert_eq!(root.frames[0].content_digest, copy.frames[0].content_digest);
    assert_eq!(
        root.frames[0].content_digest.as_ref().unwrap().as_slice(),
        digest
    );
}

#[test]
fn file_sprite_reload_inherits_only_an_identical_resource_digest() {
    fn graph(payload: Option<Vec<u8>>, revision: u64) -> ResourceGraph {
        let files = payload.into_iter().map(|payload| SubmittedFile {
            relative_path: "root.png".into(),
            category: FileCategory::Resource,
            payload: FilePayload::Bytes(ProtocolBytes::new(payload)),
            content_hash: None,
        });
        let manifest = ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: revision,
            files: files.collect(),
        };
        let (graph, diagnostics) = ResourceGraph::from_manifest(&manifest);
        assert!(diagnostics.is_empty());
        graph
    }

    let mut previous = graph(Some(vec![1, 2, 3]), 1);
    previous
        .apply_metadata(
            "root.png",
            ImageMetadataResponse {
                width: 2,
                height: 1,
                format: "png".into(),
                animated: false,
            },
        )
        .unwrap();
    assert!(previous.create_file_sprite("file", "root.png", None, false));
    let original = previous.sprite("file").unwrap();
    let original_revision = original.revision;
    let original_digest = original.frames[0].content_digest;

    let mut identical = graph(Some(vec![1, 2, 3]), 2);
    identical.inherit_runtime_graph(&previous, &[]).unwrap();
    let inherited = identical.sprite("file").expect("identical file sprite");
    assert_eq!(inherited.revision, original_revision);
    assert_eq!(inherited.frames[0].content_digest, original_digest);

    let mut changed = graph(Some(vec![3, 2, 1]), 2);
    changed.inherit_runtime_graph(&previous, &[]).unwrap();
    assert!(changed.sprite("file").is_none());
    changed
        .apply_metadata(
            "root.png",
            ImageMetadataResponse {
                width: 2,
                height: 1,
                format: "png".into(),
                animated: false,
            },
        )
        .unwrap();
    assert!(changed.create_file_sprite("file", "root.png", None, false));
    let rebuilt = changed.sprite("file").unwrap();
    assert!(rebuilt.revision > original_revision);
    assert_ne!(rebuilt.frames[0].content_digest, original_digest);

    let mut deleted = graph(None, 2);
    deleted.inherit_runtime_graph(&previous, &[]).unwrap();
    assert!(deleted.sprite("file").is_none());
}

use super::*;
use era_runtime_protocol::CanvasReplayCommand;

#[test]
fn parses_static_and_animation_sprites_then_validates_metadata() {
    let manifest = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision: 1,
        files: vec![
            SubmittedFile {
                relative_path: "resources/sprites.csv".into(),
                category: FileCategory::ResourceManifest,
                payload: FilePayload::Utf8(
                    "FACE,face.png,0,0,32,16,1,2\nRUN,ANIME,10,20\nRUN,face.png,0,0,8,8,0,0,50"
                        .into(),
                ),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "resources/face.png".into(),
                category: FileCategory::Resource,
                payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
                content_hash: None,
            },
        ],
    };
    let (mut graph, diagnostics) = ResourceGraph::from_manifest(&manifest);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    assert_eq!(graph.metadata_requests().len(), 1);
    graph
        .apply_metadata(
            "resources/face.png",
            ImageMetadataResponse {
                width: 64,
                height: 32,
                format: "png".into(),
                animated: false,
            },
        )
        .unwrap();
    assert_eq!(graph.sprite("face").unwrap().width, 32);
    assert_eq!(graph.sprite("run").unwrap().width, 10);
    let static_revision = graph.sprite("face").unwrap().revision;
    assert_eq!(graph.sprite("run").unwrap().revision, static_revision);
    assert_eq!(
        graph
            .replay()
            .sprites
            .iter()
            .find(|sprite| sprite.name == "FACE")
            .unwrap()
            .revision,
        static_revision
    );
    assert!(graph.create_canvas_from_resource(1, "resources/face.png"));
    graph
        .apply_metadata(
            "resources/face.png",
            ImageMetadataResponse {
                width: 8_193,
                height: 32,
                format: "png".into(),
                animated: false,
            },
        )
        .unwrap();
    assert!(!graph.create_canvas_from_resource(2, "resources/face.png"));
}

#[test]
fn snake_profile_skips_missing_manifest_images_without_rejecting_the_project() {
    let manifest = |profile| ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity {
            profile,
            ..era_runtime_protocol::CompatibilityIdentity::default()
        },
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "resources/sprites.csv".into(),
            category: FileCategory::ResourceManifest,
            payload: FilePayload::Utf8("MISSING,absent.png,0,0,1,1".into()),
            content_hash: None,
        }],
    };

    let (_, reference_diagnostics) = ResourceGraph::from_manifest(&manifest(
        era_runtime_protocol::CompatibilityProfileId::EmueraEm,
    ));
    assert_eq!(reference_diagnostics.len(), 1);
    assert!(reference_diagnostics[0].error);

    let (snake_graph, snake_diagnostics) = ResourceGraph::from_manifest(&manifest(
        era_runtime_protocol::CompatibilityProfileId::EmueraSkiaSnake,
    ));
    assert_eq!(snake_diagnostics.len(), 1);
    assert_eq!(snake_diagnostics[0].code, "runtime.missing_resource_image");
    assert!(!snake_diagnostics[0].error);
    assert!(snake_graph.sprite("missing").is_none());
}
