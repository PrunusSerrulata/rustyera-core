use super::projection::plain_text;
use super::*;
use era_runtime_protocol::{
    CellWidthIntent, Color, PresentationDelta, PresentationSnapshot, ResourceReplay, SceneAnchorV1,
    SceneScrollPolicyV1, SceneSourceV1,
};
use serde::Serialize;

fn display_text(run: &DisplayRun) -> Option<&str> {
    match run {
        DisplayRun::Text { text, .. } | DisplayRun::TextLayout { text, .. } => Some(text),
        _ => None,
    }
}

fn collect_text_layouts<'a>(runs: &'a [DisplayRun], output: &mut Vec<(&'a str, u32)>) {
    for run in runs {
        match run {
            DisplayRun::TextLayout { text, columns, .. } => output.push((text, *columns)),
            DisplayRun::Button { runs, .. } | DisplayRun::ColumnCell { content: runs, .. } => {
                collect_text_layouts(runs, output);
            }
            _ => {}
        }
    }
}

fn query_test_line(
    line_id: u64,
    logical_line_start: bool,
    alignment: LineAlignment,
    text: &str,
) -> Arc<DisplayLine> {
    Arc::new(DisplayLine {
        line_id,
        temporary: false,
        logical_line_start,
        line_end: true,
        alignment,
        text_background_eligible: !text.trim().is_empty(),
        runs: vec![DisplayRun::Text {
            text: text.into(),
            style: TextStyle::default(),
            system_text: None,
        }],
    })
}

#[test]
fn runtime_projection_queries_use_physical_and_logical_history_order() {
    let mut model = PresentationModel::default();
    model.set_projection(true, true, true, true, true);
    model
        .lines
        .push_back(query_test_line(1, true, LineAlignment::Left, "oldest"));
    model
        .lines
        .push_back(query_test_line(2, true, LineAlignment::Center, "wrapped-a"));
    model
        .lines
        .push_back(query_test_line(3, false, LineAlignment::Right, "wrapped-b"));

    assert_eq!(model.display_line(0, false), "oldest");
    assert_eq!(model.display_line(1, false), "wrapped-a");
    assert_eq!(model.display_line(2, false), "wrapped-b");
    assert_eq!(
        model.printed_html_line(0),
        "<p align='center'><nobr>wrapped-a<br>wrapped-b</nobr></p>"
    );
    assert_eq!(
        model.printed_html_line(1),
        "<p align='left'><nobr>oldest</nobr></p>"
    );
    assert_eq!(model.display_line(3, false), "");
    assert_eq!(model.printed_html_line(2), "");

    model.set_alignment(LineAlignment::Right);
    model.append_print_text("pending".into(), false, false);
    assert_eq!(model.line_id_at_display_index(0), Some(1));
    assert_eq!(
        model.line_id_at_display_index(3),
        Some(model.current_line_id())
    );
    assert_eq!(model.line_id_at_display_index(-1), None);
    assert_eq!(model.line_id_at_display_index(4), None);
    assert_eq!(model.display_line(3, false), "pending");
    assert_eq!(
        model.printed_html_line(0),
        "<p align='right'><nobr>pending</nobr></p>"
    );
    assert_eq!(
        model.printed_html_line(1),
        "<p align='center'><nobr>wrapped-a<br>wrapped-b</nobr></p>"
    );
    assert_eq!(model.display_line(-1, false), "");
    assert_eq!(model.display_line(-1, true), "wrapped-b");
    assert_eq!(model.display_line(-2, true), "wrapped-a");
    assert_eq!(model.display_line(i64::MIN, true), "");

    model.append_print_text(String::new(), false, true);
    assert_eq!(model.display_line(-1, true), "pending");
    model.delete_last_lines(1);
    assert_eq!(model.display_line(-1, true), "wrapped-b");

    model.settings.maximum_physical_lines = 2;
    model.append_text("trim-a".into(), false);
    model.append_text("trim-b".into(), false);
    assert_eq!(model.display_line(-1, true), "trim-b");
    assert_eq!(model.display_line(-2, true), "trim-a");
    assert_eq!(model.display_line(-3, true), "");
}

fn test_scene_source(name: &str, revision: u64) -> SceneSourceV1 {
    SceneSourceV1::Sprite {
        sprite_name: name.into(),
        resource_revision: revision,
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn snake_cbg_and_image_layers_keep_stable_order_and_type_scoped_clear_rules() {
    let mut model = PresentationModel::default();
    model.add_client_background(
        test_scene_source("low", 1),
        2,
        0,
        0,
        10,
        10,
        255,
        None,
        None,
    );
    let token = InteractionToken { epoch: 3, id: 4 };
    model.add_client_background(
        test_scene_source("button", 2),
        5,
        1,
        2,
        3,
        4,
        127,
        None,
        Some((
            token,
            42,
            Some(test_scene_source("hover", 3)),
            Some("tip".into()),
        )),
    );
    model.add_image_layer(
        test_scene_source("image", 4),
        5,
        SceneAnchorV1::Viewport,
        5,
        6,
        7,
        8,
        255,
        None,
        false,
    );

    assert_eq!(
        model
            .scene
            .layers
            .iter()
            .map(|layer| (layer.depth, layer.sequence))
            .collect::<Vec<_>>(),
        [(5, 2), (5, 3), (2, 1)]
    );
    assert!(model.image_layer_exists(5));
    let map = SceneSourceV1::Canvas {
        canvas_id: 9,
        resource_revision: 10,
    };
    assert!(model.set_client_background_button_map(map.clone()));
    assert!(!model.set_client_background_button_map(map.clone()));
    let button = model
        .scene
        .layers
        .iter()
        .find(|layer| layer.source == test_scene_source("button", 2))
        .unwrap();
    let interaction = button.interaction.as_ref().unwrap();
    assert_eq!(interaction.token, token);
    assert_eq!(interaction.value, ProtocolValue::Integer(42));
    assert_eq!(interaction.hit_map, Some(map));
    assert_eq!(interaction.title.as_deref(), Some("tip"));
    assert_eq!(
        model.enabled_button_value(token),
        Some(VmValue::Integer(42))
    );
    assert_eq!(
        model.enabled_button_value(token),
        Some(VmValue::Integer(42)),
        "scene buttons remain authoritative across waits"
    );

    let rebound = InteractionToken { epoch: 8, id: 9 };
    let revision = model.scene.revision;
    model.rebind_scene_interactions(&std::collections::BTreeMap::from([(token, rebound)]));
    assert_eq!(model.scene.revision, revision + 1);
    assert_eq!(model.enabled_button_value(token), None);
    assert_eq!(
        model.enabled_button_value(rebound),
        Some(VmValue::Integer(42))
    );

    model.clear_image_layer(5);
    assert!(!model.image_layer_exists(5));
    assert!(
        model
            .scene
            .layers
            .iter()
            .any(|layer| layer.source == test_scene_source("button", 2))
    );
    assert_eq!(model.clear_client_background_range(5, 5), [rebound]);
    assert_eq!(model.enabled_button_value(rebound), None);
    assert_eq!(model.scene.layers.len(), 1);
    assert_eq!(model.scene.layers[0].source, test_scene_source("low", 1));
    assert!(model.clear_client_backgrounds().is_empty());
    assert!(model.scene.layers.is_empty());
}

#[test]
fn line_anchored_image_layers_follow_content_and_expire_with_stable_lines() {
    let mut model = PresentationModel::default();
    let first_line_id = model.current_line_id();
    model.add_image_layer(
        test_scene_source("line-image", 1),
        3,
        SceneAnchorV1::DisplayLine {
            line_id: first_line_id,
        },
        0,
        0,
        1,
        1,
        255,
        None,
        true,
    );
    assert_eq!(
        model.scene.layers[0].scroll_policy,
        SceneScrollPolicyV1::FollowContent
    );
    assert_eq!(model.scene.layers[0].document_origin_y, LogicalLength(0));
    model.append_print_text("line".into(), false, true);
    assert_eq!(model.line_id_at_display_index(0), Some(first_line_id));
    model.delete_last_lines(1);
    assert!(!model.image_layer_exists(3));
    assert!(model.scene.layers.is_empty());

    model.settings.maximum_physical_lines = 1;
    let trimmed_line_id = model.current_line_id();
    model.add_image_layer(
        test_scene_source("trimmed-image", 2),
        4,
        SceneAnchorV1::DisplayLine {
            line_id: trimmed_line_id,
        },
        0,
        0,
        1,
        1,
        255,
        None,
        true,
    );
    model.append_print_text("trimmed".into(), false, true);
    model.append_print_text("survivor".into(), false, true);
    assert_eq!(model.line_id_at_display_index(0), Some(trimmed_line_id + 1));
    assert!(!model.image_layer_exists(4));
    assert!(model.scene.layers.is_empty());
}

#[test]
fn follow_content_captures_a_canonical_document_origin_not_client_scroll() {
    let mut model = PresentationModel::default();
    model.set_projection(true, true, true, true, true);
    model.settings.maximum_physical_lines = 1;
    model.append_text("completed".into(), false);
    let canonical_line_height = model.settings.line_height;
    model.delete_last_lines(1);
    model.append_text("trimmed replacement".into(), false);
    model.append_text("forces max-log trim".into(), false);
    model.add_image_layer(
        test_scene_source("following", 3),
        1,
        SceneAnchorV1::Viewport,
        0,
        0,
        1,
        1,
        255,
        None,
        true,
    );
    let layer = &model.scene.layers[0];
    assert_eq!(
        layer.document_origin_y,
        LogicalLength(canonical_line_height.0.saturating_mul(3))
    );
    assert_eq!(model.snapshot().scene.layers[0], layer.clone());
}

#[test]
fn whole_line_text_background_uses_stable_line_eligibility_and_settings_delta() {
    let text = DisplayRun::Text {
        text: "visible".into(),
        style: TextStyle::default(),
        system_text: None,
    };
    let whitespace = DisplayRun::Text {
        text: " \t".into(),
        style: TextStyle::default(),
        system_text: None,
    };
    let html = DisplayRun::HtmlDocument {
        document: erabasic_html::parse_document("<b>html</b>").unwrap(),
    };
    assert!(line_has_text_background(std::slice::from_ref(&text)));
    assert!(!line_has_text_background(&[whitespace]));
    assert!(line_has_text_background(&[DisplayRun::Button {
        runs: vec![text],
        token: InteractionToken { epoch: 1, id: 1 },
        title: None,
        hover_style: None,
        value: ProtocolValue::Integer(1),
        generation: 0,
        enabled: true,
    }]));
    assert!(line_has_text_background(&[html]));
    assert!(!line_has_text_background(&[DisplayRun::Image {
        placement: MediaPlacement {
            resource_id: "image".into(),
            x: LogicalLength(0),
            y: LogicalLength(0),
            width: LogicalLength(1),
            height: LogicalLength(1),
            depth: 0,
            opacity: RationalOpacity {
                numerator: 1,
                denominator: 1
            },
            revision: 1,
            hover_resource_id: None,
            mask_resource_id: None,
            requested_width: None,
            requested_height: None,
            requested_y: None,
        },
        alt_text: Some("not styled text".into()),
    }]));

    let mut model = PresentationModel::default();
    model.append_print_text("existing".into(), false, true);
    let PresentationUpdate::Snapshot(snapshot) = model.next_update() else {
        panic!("initial delivery must be a snapshot");
    };
    assert!(snapshot.history.logical_lines[0].text_background_eligible);
    let color = Color {
        red: 1,
        green: 2,
        blue: 3,
        alpha: 127,
    };
    model.set_text_line_background(Some(color));
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("background toggle must be a delta");
    };
    assert!(delta.operations.iter().any(|operation| matches!(
        operation,
        PresentationOperation::SetSettings { settings }
            if settings.text_line_background == Some(color)
    )));
}

#[test]
fn logical_animation_timer_is_projected_without_graphics_resources() {
    let mut model = PresentationModel::default();
    model.set_projection(false, false, false, false, false);
    model.set_resource_replay(ResourceReplay {
        animation_timer_ms: 10,
        ..ResourceReplay::default()
    });
    let PresentationUpdate::Snapshot(snapshot) = model.next_update() else {
        panic!("first delivery must establish a snapshot baseline");
    };
    assert_eq!(snapshot.resources.animation_timer_ms, 10);
    assert!(snapshot.resources.sprites.is_empty());
    assert!(snapshot.resources.canvases.is_empty());

    model.set_resource_replay(ResourceReplay {
        animation_timer_ms: 20,
        ..ResourceReplay::default()
    });
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("timer update must be a delta");
    };
    assert!(delta.operations.iter().any(|operation| matches!(
        operation,
        PresentationOperation::SetResources { resources }
            if resources.animation_timer_ms == 20
                && resources.sprites.is_empty()
                && resources.canvases.is_empty()
    )));
}

fn rich_projected_runs() -> Vec<DisplayRun> {
    let token = InteractionToken { epoch: 1, id: 1 };
    let style = TextStyle {
        bold: true,
        italic: true,
        underline: true,
        strikeout: true,
        ..TextStyle::default()
    };
    let button = |value, title: Option<&str>| DisplayRun::Button {
        runs: vec![plain_text("button".into(), 18_000)],
        token,
        title: title.map(str::to_owned),
        hover_style: None,
        value,
        generation: 0,
        enabled: true,
    };
    vec![
        DisplayRun::Text {
            text: "<&".into(),
            style,
            system_text: None,
        },
        button(ProtocolValue::Integer(7), Some("t'&")),
        button(ProtocolValue::String("s<&".into()), None),
        button(ProtocolValue::Boolean(true), None),
        DisplayRun::HtmlDocument {
            document: erabasic_html::parse_document(
                "<p align='right'><nobr><b>root</b></nobr></p>",
            )
            .unwrap(),
        },
        DisplayRun::Image {
            placement: MediaPlacement {
                resource_id: "image<&".into(),
                x: LogicalLength(0),
                y: LogicalLength(0),
                width: LogicalLength(0),
                height: LogicalLength(0),
                depth: 0,
                opacity: RationalOpacity {
                    numerator: 1,
                    denominator: 1,
                },
                revision: 1,
                hover_resource_id: Some("hover".into()),
                mask_resource_id: Some("mask".into()),
                requested_width: Some(PresentationLength::Logical(LogicalLength(12_000))),
                requested_height: None,
                requested_y: None,
            },
            alt_text: Some("alt".into()),
        },
        DisplayRun::Shape {
            shape: Shape {
                kind: "rect".into(),
                parameters: vec![PresentationLength::Logical(LogicalLength(5_000))],
                foreground: Some(Color {
                    red: 1,
                    green: 2,
                    blue: 3,
                    alpha: 255,
                }),
                background: Some(Color {
                    red: 4,
                    green: 5,
                    blue: 6,
                    alpha: 255,
                }),
            },
        },
        DisplayRun::Space {
            width: PresentationLength::FontHeightHundredths(50),
        },
        DisplayRun::Separator {
            pattern: "-&".into(),
            role: SeparatorRole::Rule,
            style: TextStyle::default(),
        },
        DisplayRun::ColumnCell {
            content: vec![plain_text("cell".into(), 18_000)],
            alignment: CellAlignment::Left,
            width: CellWidthIntent::ProjectColumns(4),
        },
    ]
}
