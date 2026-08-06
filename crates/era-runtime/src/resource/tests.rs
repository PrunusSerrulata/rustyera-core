use era_protocol::ProtocolBytes;
use era_runtime_protocol::{ProjectManifest, SubmittedFile};

#[test]
fn compact_snapshot_preserves_and_validates_static_resource_identities() {
    let manifest = ProjectManifest {
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

use super::*;

#[test]
fn parses_static_and_animation_sprites_then_validates_metadata() {
    let manifest = ProjectManifest {
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
fn canvas_and_dynamic_sprite_mutations_form_a_deterministic_replay_graph() {
    let mut graph = ResourceGraph::default();
    assert_eq!(graph.create_canvas(3, 64, 32), Ok(true));
    assert_eq!(graph.create_canvas(3, 1, 1), Ok(false));
    assert!(graph.clear_canvas(3, 0x00ff_00ff, None));
    assert!(graph.create_canvas_sprite("generated", 3, None));
    assert!(graph.create_animation_sprite("animated", 16, 16));
    assert!(graph.add_animation_frame("animated", 3, [0, 0, 16, 16], [2, 3], 55,));
    assert_eq!(
        graph
            .sprite("GENERATED")
            .map(|sprite| (sprite.width, sprite.height)),
        Some((64, 32))
    );
    assert!(graph.move_sprite("generated", 4, 5, false));
    graph.set_animation_timer(55);
    assert_eq!(graph.animation_timer(), 55);
    assert_eq!(
        graph
            .sprite("generated")
            .map(|sprite| (sprite.position_x, sprite.position_y)),
        Some((4, 5))
    );
    let replay = graph.replay();
    assert_eq!(replay.canvases.len(), 1);
    assert_eq!(replay.canvases[0].revision, 1);
    let animated = replay
        .sprites
        .iter()
        .find(|sprite| sprite.name == "ANIMATED")
        .unwrap();
    assert_eq!(animated.frames[0].canvas_id, Some(3));
    assert_eq!(animated.frames[0].delay_ms, 55);
    assert!(
        replay
            .sprites
            .iter()
            .any(|sprite| sprite.canvas_id == Some(3))
    );
    assert_eq!(replay.animation_timer_ms, 55);
    assert_eq!(graph.dispose_sprites(false), 2);
    assert!(graph.dispose_canvas(3));
}

#[test]
fn portable_canvas_replay_captures_style_draw_and_snapshot_revisions() {
    let mut graph = ResourceGraph::default();
    graph.configure_canvas_defaults(0x0011_2233, 0x0044_5566, "Project Font".into(), 3);
    graph.create_canvas(1, 20, 10).unwrap();
    graph.create_canvas(2, 20, 10).unwrap();
    assert_eq!(
        graph.canvas_style(1),
        Some((0xff44_5566, 0xff11_2233, 1, "Project Font", 100, 3))
    );
    assert!(graph.set_canvas_brush(1, 0xff11_2233));
    assert!(graph.set_canvas_pen(1, 0xff44_5566, 3));
    assert!(graph.set_canvas_dash(1, 2, 1));
    assert!(graph.set_canvas_font(1, "portable".into(), 18, 9));
    assert!(graph.set_canvas_pixel(1, 0xff00_ff00, [0, 0]));
    assert!(!graph.set_canvas_pixel(1, 0, [0, -1]));
    assert!(graph.fill_canvas_rectangle(1, [1, 2, 3, 4]));
    assert!(graph.draw_canvas_line(1, [0, 0], [2, 3]));
    assert!(graph.draw_canvas_text(1, "text".into(), [4, 5]));
    assert!(graph.draw_canvas(1, 2, None, None, Some(vec![256; 25]), None, 0, None));
    let replay = graph.replay();
    let canvas = replay
        .canvases
        .iter()
        .find(|item| item.canvas_id == 1)
        .unwrap();
    assert_eq!(canvas.revision, 9);
    assert!(matches!(
        canvas.commands.last(),
        Some(CanvasReplayCommand::DrawCanvas {
            source_canvas_id: 2,
            source_revision: 0,
            color_matrix: Some(matrix),
            ..
        }) if matrix.len() == 25
    ));
    graph.set_animation_timer(1);
    assert_eq!(graph.animation_timer(), 10);
    graph.set_animation_timer(-1);
    assert_eq!(graph.animation_timer(), 0);
}
