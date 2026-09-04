#[test]
fn canvas_and_dynamic_sprite_mutations_form_a_deterministic_replay_graph() {
    let mut graph = ResourceGraph::default();
    assert_eq!(graph.create_canvas(3, 64, 32), Ok(true));
    assert_eq!(graph.create_canvas(3, 1, 1), Ok(false));
    assert!(graph.clear_canvas(3, 0x00ff_00ff, None));
    assert!(graph.create_canvas_sprite("generated", 3, None, [0, 0], None));
    let created_revision = graph.sprite("generated").unwrap().revision;
    assert!(graph.create_animation_sprite("animated", 16, 16));
    assert!(graph.add_animation_frame("animated", 3, [0, 0, 16, 16], [2, 3], 55,));
    assert_eq!(
        graph
            .sprite("GENERATED")
            .map(|sprite| (sprite.width, sprite.height)),
        Some((64, 32))
    );
    assert!(graph.move_sprite("generated", 4, 5, false));
    assert!(graph.sprite("generated").unwrap().revision > created_revision);
    assert!(graph.set_animation_timer(55));
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
    assert_eq!(
        animated.revision,
        graph.sprite("animated").unwrap().revision
    );
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
fn canvas_sprite_overloads_preserve_offsets_scaling_and_source_flips() {
    let mut graph = ResourceGraph::default();
    graph.create_canvas(1, 4, 3).unwrap();
    assert!(graph.create_canvas_sprite("two", 1, None, [0, 0], None));
    assert!(graph.create_canvas_sprite("six", 1, Some([1, 1, 2, 2]), [0, 0], None,));
    assert!(graph.create_canvas_sprite("eight", 1, Some([0, 0, 2, 1]), [-3, 4], None,));
    assert!(graph.create_canvas_sprite("ten", 1, Some([0, 0, 2, 1]), [-3, 4], Some([-7, -9]),));
    assert!(graph.create_canvas_sprite("flip", 1, Some([3, 2, -2, -1]), [0, 0], None,));
    assert!(!graph.create_canvas_sprite("outside", 1, Some([5, 0, -1, 1]), [0, 0], None,));

    let replay = graph.replay();
    let sprite = |name: &str| {
        replay
            .sprites
            .iter()
            .find(|sprite| sprite.name == name)
            .unwrap()
    };
    assert_eq!(sprite("TWO").size, [4, 3]);
    assert_eq!(sprite("SIX").size, [2, 2]);
    assert_eq!(sprite("EIGHT").position, [-3, 4]);
    assert_eq!(sprite("TEN").size, [7, 9]);
    assert_eq!(sprite("FLIP").size, [2, 1]);
    assert_eq!(sprite("FLIP").canvas_rectangle.unwrap().width, -2);
    assert_eq!(sprite("FLIP").canvas_rectangle.unwrap().height, -1);
}

#[test]
fn polygon_point_state_replays_deterministically_and_survives_full_clear() {
    let mut graph = ResourceGraph::default();
    graph.create_canvas(1, 8, 8).unwrap();
    assert_eq!(
        graph.draw_canvas_polygon(1, false),
        Err("polygon point set is empty")
    );
    assert_eq!(graph.draw_canvas_polygon(99, false), Ok(false));
    for point in [[1, 1], [6, 1], [3, 6]] {
        assert!(graph.add_canvas_polygon_point(1, point));
    }
    assert_eq!(graph.draw_canvas_polygon(1, false), Ok(true));
    assert_eq!(graph.draw_canvas_polygon(1, true), Ok(true));
    assert!(graph.clear_canvas(1, 0, None));
    assert_eq!(graph.draw_canvas_polygon(1, false), Ok(true));
    assert!(graph.clear_canvas_polygon_points(1));
    assert_eq!(
        graph.draw_canvas_polygon(1, true),
        Err("polygon point set is empty")
    );

    let replay = graph.replay();
    let canvas = replay
        .canvases
        .iter()
        .find(|canvas| canvas.canvas_id == 1)
        .unwrap();
    assert_eq!(canvas.revision, 8);
    assert_eq!(
        canvas
            .commands
            .iter()
            .filter(|command| matches!(command, CanvasReplayCommand::PolygonPointAdd { .. }))
            .count(),
        3,
    );
    assert!(matches!(
        canvas.commands.as_slice(),
        [
            CanvasReplayCommand::Clear { rectangle: None, .. },
            CanvasReplayCommand::SetBrush { .. },
            CanvasReplayCommand::SetPen { .. },
            CanvasReplayCommand::SetDashStyle { .. },
            CanvasReplayCommand::SetFont { .. },
            CanvasReplayCommand::PolygonPointAdd { point: first },
            CanvasReplayCommand::PolygonPointAdd { point: second },
            CanvasReplayCommand::PolygonPointAdd { point: third },
            CanvasReplayCommand::DrawPolygon,
            CanvasReplayCommand::PolygonPointClear,
        ] if [first.x, first.y] == [1, 1]
            && [second.x, second.y] == [6, 1]
            && [third.x, third.y] == [3, 6]
    ));
    assert!(matches!(
        canvas.commands.last(),
        Some(CanvasReplayCommand::PolygonPointClear)
    ));
}

#[test]
fn static_sprite_revision_binds_same_name_to_resource_content() {
    let manifest = |image_byte| ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision: 4,
        files: vec![
            SubmittedFile {
                relative_path: "resources/sprites.csv".into(),
                category: FileCategory::ResourceManifest,
                payload: FilePayload::Utf8("SAME,image.png,0,0,1,1".into()),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "resources/image.png".into(),
                category: FileCategory::Resource,
                payload: FilePayload::Bytes(ProtocolBytes::new(vec![image_byte])),
                content_hash: None,
            },
        ],
    };
    let (first, diagnostics) = ResourceGraph::from_manifest(&manifest(1));
    assert!(diagnostics.is_empty());
    let (second, diagnostics) = ResourceGraph::from_manifest(&manifest(2));
    assert!(diagnostics.is_empty());
    assert_ne!(
        first.sprite_revision("same"),
        second.sprite_revision("same")
    );
}

#[test]
fn full_canvas_clear_discards_the_unobservable_replay_prefix() {
    let mut graph = ResourceGraph::default();
    assert_eq!(graph.create_canvas(9, 64, 32), Ok(true));
    for index in 0..1_000 {
        assert!(graph.draw_canvas_text(9, format!("retained-{index}"), [0, 0]));
    }
    assert_eq!(graph.canvases[&9].commands.len(), 1_000);
    let old_capacity = graph.canvases[&9].commands.capacity();
    assert!(graph.set_canvas_brush(9, 0xff11_2233));
    assert!(graph.set_canvas_pen(9, 0xff44_5566, 3));
    assert!(graph.set_canvas_dash(9, 2, 1));
    assert!(graph.set_canvas_font(9, "checkpoint".into(), 18, 9));

    assert!(graph.clear_canvas(9, 0xff00_0000, None));
    assert!(graph.draw_canvas_line(9, [0, 0], [2, 3]));

    let canvas = &graph.canvases[&9];
    assert_eq!(canvas.commands.len(), 6);
    assert!(canvas.commands.capacity() < old_capacity);
    assert_eq!(
        canvas.retained_command_bytes,
        canvas
            .commands
            .iter()
            .map(CanvasCommand::retained_bytes)
            .sum::<usize>()
    );
    assert_eq!(
        graph.retained_canvas_command_bytes,
        canvas.retained_command_bytes
    );
    assert!(matches!(
        graph.replay().canvases[0].commands.as_slice(),
        [
            CanvasReplayCommand::Clear { rectangle: None, .. },
            CanvasReplayCommand::SetBrush { argb: 0xff11_2233 },
            CanvasReplayCommand::SetPen { argb: 0xff44_5566, width: 3 },
            CanvasReplayCommand::SetDashStyle { style: 2, cap: 1 },
            CanvasReplayCommand::SetFont { family, size: 18, style_bits: 9 },
            CanvasReplayCommand::DrawLine { .. },
        ] if family == "checkpoint"
    ));
}

#[test]
fn canvas_command_budget_is_graph_wide_and_restored_lazily_for_old_snapshots() {
    assert!(super::canvas::retained_canvas_bytes_fit(60, 30, 40, 100));
    assert!(!super::canvas::retained_canvas_bytes_fit(70, 20, 31, 100));
    assert!(!super::canvas::retained_canvas_bytes_fit(30, 80, 21, 100));

    let mut graph = ResourceGraph::default();
    graph.create_canvas(1, 10, 10).unwrap();
    graph.create_canvas(2, 10, 10).unwrap();
    assert!(graph.draw_canvas_text(1, "first".into(), [0, 0]));
    assert!(graph.draw_canvas_text(2, "second".into(), [0, 0]));
    let expected = graph
        .canvases
        .values()
        .map(|canvas| canvas.retained_command_bytes)
        .sum();
    assert_eq!(graph.retained_canvas_command_bytes, expected);

    let encoded = serde_json::to_vec(&graph).expect("serialize legacy-shaped graph");
    let mut restored: ResourceGraph =
        serde_json::from_slice(&encoded).expect("restore graph without derived counters");
    assert_eq!(restored.retained_canvas_command_bytes, 0);
    assert!(
        restored
            .canvases
            .values()
            .all(|canvas| canvas.retained_command_bytes == 0)
    );
    assert!(!restored.push_canvas_command_with_limit(
        1,
        CanvasCommand::DrawText {
            text: "over restored budget".into(),
            point: [0, 0],
        },
        expected,
    ));
    assert_eq!(restored.retained_canvas_command_bytes, expected);
    assert!(restored.draw_canvas_text(1, "after restore".into(), [0, 0]));
    assert_eq!(
        restored.retained_canvas_command_bytes,
        restored
            .canvases
            .values()
            .map(|canvas| canvas.retained_command_bytes)
            .sum::<usize>()
    );

    let released = restored.canvases[&2].retained_command_bytes;
    assert!(restored.dispose_canvas(2));
    assert_eq!(
        restored.retained_canvas_command_bytes,
        expected.saturating_sub(released).saturating_add(
            CanvasCommand::DrawText {
                text: "after restore".into(),
                point: [0, 0],
            }
            .retained_bytes()
        )
    );
}

#[test]
fn resource_backed_canvas_creation_uses_the_same_graph_budget() {
    let mut graph = ResourceGraph::default();
    let retained = super::canvas::MAXIMUM_CANVAS_COMMAND_BYTES.saturating_sub(1);
    graph.retained_canvas_command_bytes = retained;
    let image = graph
        .images
        .entry("resources/image.png".into())
        .or_insert(ResourceImage {
            relative_path: "resources/image.png".into(),
            digest: [0; 32],
            metadata: Some(ImageMetadata {
                width: 1,
                height: 1,
                format: "png".into(),
                animated: false,
            }),
            bytes: vec![1],
        });
    image.bytes = vec![1];
    assert!(!graph.create_canvas_from_resource(1, "image.png"));
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
    assert!(graph.set_animation_timer(1));
    assert_eq!(graph.animation_timer(), 10);
    assert!(graph.set_animation_timer(-1));
    assert_eq!(graph.animation_timer(), 0);
    assert!(graph.set_animation_timer(i64::from(i32::MIN)));
    assert_eq!(graph.animation_timer(), 0);
    assert!(graph.set_animation_timer(i64::from(i16::MAX)));
    assert_eq!(graph.animation_timer(), i32::from(i16::MAX));
    assert!(!graph.set_animation_timer(i64::from(i16::MAX) + 1));
    assert_eq!(graph.animation_timer(), i32::from(i16::MAX));
    assert!(!graph.set_animation_timer(i64::from(i32::MIN) - 1));
    assert_eq!(graph.animation_timer(), i32::from(i16::MAX));
}

#[test]
#[allow(clippy::too_many_lines)]
fn replay_keeps_only_the_exact_live_historical_dependency_closure() {
    let mut graph = ResourceGraph::default();
    for id in 1..=3 {
        graph.create_canvas(id, 8, 8).unwrap();
    }
    assert!(graph.clear_canvas(1, 1, None));
    assert!(graph.clear_canvas(2, 2, None));
    assert!(graph.draw_canvas(3, 1, None, None, None, Some(2), 0, None));
    assert!(graph.clear_canvas(1, 3, None));
    assert!(graph.clear_canvas(2, 4, None));

    assert!(graph.create_canvas_sprite("FROM_CANVAS", 1, None, [0, 0], None));
    assert!(graph.create_animation_sprite("ANIM", 8, 8));
    assert!(graph.add_animation_frame("ANIM", 2, [0, 0, 8, 8], [0, 0], 10));
    let animation_revision = graph.sprite_revision("ANIM").unwrap();
    assert!(graph.draw_sprite(3, "ANIM", None, None));
    let animation_source = SceneSourceV1::Sprite {
        sprite_name: "ANIM".into(),
        resource_revision: animation_revision,
    };
    assert!(graph.retain_scene_source(&animation_source));

    let scene = SceneStateV1 {
        revision: 1,
        layers: vec![SceneLayerV1 {
            layer_id: 1,
            sequence: 1,
            source: animation_source,
            depth: 1,
            anchor: SceneAnchorV1::Viewport,
            offset: SceneOffsetV1 {
                x: LogicalLength(0),
                y: LogicalLength(0),
            },
            size: SceneSizeV1 {
                width: LogicalLength(8_000),
                height: LogicalLength(8_000),
            },
            opacity: u8::MAX,
            color_matrix: None,
            scroll_policy: SceneScrollPolicyV1::Fixed,
            interaction: None,
            scene_revision: 1,
            document_origin_y: LogicalLength(0),
        }],
    };
    let roots = vec![scene.layers[0].source.clone()];
    let baseline = graph.replay_for_roots(&roots).unwrap();
    assert!(
        baseline
            .sprites
            .iter()
            .any(|sprite| { sprite.name == "ANIM" && sprite.revision == animation_revision })
    );
    let snapshot = serde_json::to_vec(&graph).expect("serialize revision snapshot maps");
    graph = serde_json::from_slice(&snapshot).expect("restore revision snapshot maps");
    assert_eq!(graph.exact_revisions.retained_canvas_command_bytes, 0);
    let _ = graph.replay_for_roots(&roots).unwrap();
    assert!(graph.exact_revisions.retained_canvas_command_bytes > 0);
    assert!(graph.move_sprite("ANIM", 1, 2, false));
    assert!(graph.dispose_sprite("ANIM"));
    let replay = graph.replay_for_roots(&roots).unwrap();
    let canvas_identities = replay
        .canvases
        .iter()
        .map(|canvas| (canvas.canvas_id, canvas.revision))
        .collect::<Vec<_>>();
    assert_eq!(canvas_identities, [(1, 1), (1, 2), (2, 1), (2, 2), (3, 2)]);
    let sprite_identities = replay
        .sprites
        .iter()
        .map(|sprite| (sprite.name.as_str(), sprite.revision))
        .collect::<Vec<_>>();
    assert_eq!(
        sprite_identities,
        [
            ("ANIM", animation_revision),
            ("FROM_CANVAS", graph.sprite_revision("FROM_CANVAS").unwrap()),
        ]
    );
    let from_canvas = replay
        .sprites
        .iter()
        .find(|sprite| sprite.name == "FROM_CANVAS")
        .unwrap();
    assert_eq!(from_canvas.canvas_id, Some(1));
    assert_eq!(from_canvas.canvas_revision, Some(2));
    let animation = replay
        .sprites
        .iter()
        .find(|sprite| sprite.name == "ANIM")
        .unwrap();
    assert_eq!(animation.frames[0].canvas_id, Some(2));
    assert_eq!(animation.frames[0].canvas_revision, Some(2));
    let target = replay
        .canvases
        .iter()
        .find(|canvas| canvas.canvas_id == 3 && canvas.revision == 2)
        .unwrap();
    assert!(matches!(
        target.commands.first(),
        Some(CanvasReplayCommand::DrawCanvas {
            source_canvas_id: 1,
            source_revision: 1,
            mask_canvas_id: Some(2),
            mask_revision: Some(1),
            ..
        })
    ));
    assert!(matches!(
        target.commands.last(),
        Some(CanvasReplayCommand::DrawSprite {
            name,
            resource_revision,
            ..
        }) if name == "ANIM" && *resource_revision == animation_revision
    ));

    assert_eq!(graph.dispose_sprites(false), 1);
    for id in 1..=3 {
        assert!(graph.dispose_canvas(id));
    }
    let empty = graph.replay_for_roots(&[]).unwrap();
    assert!(empty.sprites.is_empty());
    assert!(empty.canvases.is_empty());
    assert!(graph.exact_revisions.sprites.is_empty());
    assert!(graph.exact_revisions.canvases.is_empty());
}

#[test]
fn missing_exact_dependency_is_rejected_without_mutating_the_graph() {
    let mut graph = ResourceGraph::default();
    graph.create_canvas(1, 8, 8).unwrap();
    let before = serde_json::to_value(&graph).unwrap();
    assert!(!graph.retain_scene_source(&SceneSourceV1::Canvas {
        canvas_id: 1,
        resource_revision: 99,
    }));
    assert_eq!(serde_json::to_value(&graph).unwrap(), before);
}
