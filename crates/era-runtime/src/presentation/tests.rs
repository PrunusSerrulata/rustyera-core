use super::*;

#[test]
fn project_default_style_changes_update_existing_default_runs() {
    let mut model = PresentationModel::default();
    model.append_print_text("default".into(), false, true);
    model.set_bold(true);
    model.append_print_text("bold".into(), false, true);
    model.set_font(Some("explicit".into()));
    model.append_print_text("explicit".into(), false, true);

    let mut next = model.default_style.clone();
    next.font_family = Some("project-default".into());
    next.font_millipixels = 20_000;
    model.apply_project_default_style(next.clone());

    let styles = model
        .lines
        .iter()
        .map(|line| match &line.runs[0] {
            DisplayRun::Text { style, .. } => style,
            _ => panic!("test line must contain text"),
        })
        .collect::<Vec<_>>();
    assert_eq!(styles[0].font_family, next.font_family);
    assert_eq!(styles[0].font_millipixels, 20_000);
    assert_eq!(styles[1].font_family, next.font_family);
    assert_eq!(styles[1].font_millipixels, 20_000);
    assert!(styles[1].bold);
    assert_eq!(styles[2].font_family.as_deref(), Some("explicit"));
    assert_eq!(styles[2].font_millipixels, 20_000);
    assert!(model.delivery.dirty.force_snapshot);
}
use era_runtime_protocol::PresentationOperation;

fn apply_delta(snapshot: &mut PresentationSnapshot, delta: PresentationDelta) {
    assert_eq!(snapshot.revision, delta.base_revision);
    for operation in delta.operations {
        match operation {
            PresentationOperation::AppendLine { line } => {
                snapshot.history.logical_lines.push(line);
            }
            PresentationOperation::DeleteLines { count } => {
                let retained = snapshot
                    .history
                    .logical_lines
                    .len()
                    .saturating_sub(count as usize);
                snapshot.history.logical_lines.truncate(retained);
            }
            PresentationOperation::Clear => snapshot.history.logical_lines.clear(),
            PresentationOperation::TrimLines { count } => {
                let count = (count as usize).min(snapshot.history.logical_lines.len());
                snapshot.history.logical_lines.drain(..count);
            }
            PresentationOperation::SetTitle { title } => snapshot.title = title,
            PresentationOperation::SetBackgrounds { backgrounds } => {
                snapshot.backgrounds = backgrounds;
            }
            PresentationOperation::SetAudio { audio } => snapshot.audio = audio,
            PresentationOperation::SetInputWait { input_wait } => {
                snapshot.input_wait = input_wait;
            }
            PresentationOperation::ReplaceLine { line_id, line } => {
                let target = snapshot
                    .history
                    .logical_lines
                    .iter_mut()
                    .find(|current| current.line_id == line_id)
                    .expect("delta replaces an existing logical line");
                *target = line;
            }
            PresentationOperation::SetSettings { settings } => snapshot.settings = settings,
            PresentationOperation::SetTooltip { tooltip } => snapshot.tooltip = tooltip,
            PresentationOperation::SetResources { resources } => {
                snapshot.resources = resources;
            }
            PresentationOperation::SetHtmlIsland { html_island } => {
                snapshot.html_island = html_island;
            }
            PresentationOperation::SetRedraw { redraw } => snapshot.redraw = redraw,
            PresentationOperation::SetButtonGeneration { generation } => {
                for line in &mut snapshot.history.logical_lines {
                    disable_old_buttons(&mut line.runs, generation);
                }
            }
        }
    }
    snapshot.revision = delta.new_revision;
}

fn assert_visible_snapshot_eq(left: &PresentationSnapshot, right: &PresentationSnapshot) {
    assert_eq!(left.revision, right.revision);
    assert_eq!(left.title, right.title);
    assert_eq!(left.history.logical_lines, right.history.logical_lines);
    assert_eq!(left.backgrounds, right.backgrounds);
    assert_eq!(left.audio, right.audio);
    assert_eq!(left.input_wait, right.input_wait);
    assert_eq!(left.settings, right.settings);
    assert_eq!(left.tooltip, right.tooltip);
    assert_eq!(left.resources, right.resources);
    assert_eq!(left.html_island, right.html_island);
    assert_eq!(left.redraw, right.redraw);
}

#[test]
fn presentation_deltas_replay_to_the_same_visible_state_as_a_snapshot() {
    let mut model = PresentationModel::default();
    model.set_projection(true, true, true, true, true);
    let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
        panic!("first delivery must establish a snapshot baseline");
    };

    model.append_print_text("left".into(), false, false);
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("pending text should use a delta after synchronization");
    };
    assert!(matches!(
        delta.operations.as_slice(),
        [PresentationOperation::AppendLine { line }] if !line.line_end
    ));
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());

    model.append_print_text(" right".into(), false, true);
    model.set_title("delta title".into());
    model.set_background(0x0011_2233);
    model.set_redraw(false);
    model.set_tooltip_delay(250).unwrap();
    model.set_resource_replay(ResourceReplay::default());
    model.add_background("background".into(), 2, 128);
    model.set_audio("sound".into(), false, true);
    model.append_html_island(erabasic_html::parse_document("<b>top</b>").unwrap());
    model.set_button_generation(1);
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("recoverable presentation fields should remain incremental");
    };
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());

    let restored = serde_json::to_vec(&model).unwrap();
    let mut restored: PresentationModel = serde_json::from_slice(&restored).unwrap();
    assert!(matches!(
        restored.next_update(),
        PresentationUpdate::Snapshot(_)
    ));
}

#[test]
fn consecutive_column_cells_share_the_pending_logical_line() {
    let mut model = PresentationModel::default();
    model.append_column_cell("A".into(), CellAlignment::Right);
    model.append_column_cell("B".into(), CellAlignment::Left);
    let pending = model.snapshot();
    assert_eq!(pending.history.logical_lines.len(), 1);
    assert!(!pending.history.logical_lines[0].line_end);
    assert_eq!(pending.history.logical_lines[0].runs.len(), 2);

    model.append_print_text("done".into(), false, true);
    let committed = model.snapshot();
    assert_eq!(committed.history.logical_lines.len(), 1);
    assert!(committed.history.logical_lines[0].line_end);
    assert_eq!(committed.history.logical_lines[0].runs.len(), 3);
}

#[test]
fn content_reset_preserves_negotiated_html_projection() {
    let mut model = PresentationModel::default();
    model.set_projection(true, true, true, true, true);
    model.append_print_text("old".into(), false, true);

    model.reset_preserving_projection();
    model.append_html(
        erabasic_html::parse_document("<p align='center'><img src='title'></p>").unwrap(),
    );

    assert!(matches!(
        &model.snapshot().history.logical_lines[0].runs[0],
        DisplayRun::HtmlDocument { .. }
    ));
}

#[test]
fn plain_projection_pads_column_cells_to_their_preferred_width() {
    let mut model = PresentationModel::default();
    model.set_projection(false, false, false, false, false);
    model.append_column_cell("A".into(), CellAlignment::Right);
    model.append_column_cell("B".into(), CellAlignment::Right);
    let snapshot = model.snapshot();
    let text = snapshot.history.logical_lines[0]
        .runs
        .iter()
        .filter_map(|run| match run {
            DisplayRun::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(text, format!("{}A{}B", " ".repeat(24), " ".repeat(24)));

    model.append_print_text(String::new(), false, true);
    let committed = model.snapshot();
    let Some(PresentationHistoryOperation::Append { line }) = committed.history.operations.last()
    else {
        panic!("committed cell line must append to physical history");
    };
    assert!(
        line.runs
            .iter()
            .all(|run| !matches!(run, DisplayRun::ColumnCell { .. }))
    );
}

#[test]
fn separator_flushes_existing_text_to_an_independent_line() {
    let mut model = PresentationModel::default();
    model.append_print_text("prefix".into(), false, false);
    model.append_separator("=".into());
    let snapshot = model.snapshot();
    assert_eq!(snapshot.history.logical_lines.len(), 2);
    assert!(matches!(
        &snapshot.history.logical_lines[1].runs[0],
        DisplayRun::Separator { pattern, .. } if pattern == "="
    ));
}

#[test]
fn temporary_empty_lines_can_be_replaced_without_frontend_state() {
    let mut model = PresentationModel::default();
    model.append_text("before".into(), false);
    model.append_text(String::new(), true);
    assert!(model.last_line_is_temporary());
    assert!(model.last_line_is_empty());
    model.replace_last_temporary("invalid".into());
    let snapshot = model.snapshot();
    assert_eq!(snapshot.history.logical_lines.len(), 2);
    assert!(snapshot.history.logical_lines[1].temporary);
    assert!(matches!(
        &snapshot.history.logical_lines[1].runs[0],
        DisplayRun::Text { text, .. } if text == "invalid"
    ));
}

#[test]
fn logical_line_string_uses_width_without_splitting_graphemes() {
    assert_eq!(logical_line_string("界", 5), Ok("界界".into()));
    assert_eq!(
        logical_line_string("e\u{301}", 3),
        Ok("e\u{301}e\u{301}e\u{301}".into())
    );
    assert!(logical_line_string("\u{301}", 10).is_err());
    assert!(logical_line_string("", 10).is_err());
}

#[test]
fn style_and_media_are_canonical_but_capability_projected() {
    let mut model = PresentationModel::default();
    model.set_font_style(1 | 8);
    model.set_alignment(LineAlignment::Center);
    model.append_print_text("styled".into(), false, true);
    model.append_html(erabasic_html::parse_document("<b>fallback</b>").unwrap());
    model.append_image("image.png".into(), Some("image".into()));
    model.set_audio("sound.ogg".into(), false, true);

    let fallback = model.snapshot();
    assert!(fallback.audio.is_empty());
    assert_eq!(
        fallback.history.logical_lines[0].alignment,
        LineAlignment::Center
    );
    let DisplayRun::Text { style, .. } = &fallback.history.logical_lines[0].runs[0] else {
        panic!("first run must be text");
    };
    assert!(style.bold);
    assert!(style.underline);
    assert!(matches!(
        &fallback.history.logical_lines[1].runs[0],
        DisplayRun::Text { text, .. } if text == "fallback"
    ));

    model.set_projection(true, true, true, true, true);
    let rich = model.snapshot();
    assert_eq!(rich.audio.len(), 1);
    assert!(matches!(
        rich.history.logical_lines[1].runs[0],
        DisplayRun::HtmlDocument { .. }
    ));
    assert!(matches!(
        rich.history.logical_lines[2].runs[0],
        DisplayRun::Image { .. }
    ));
}

#[test]
fn style_queries_and_html_island_remain_canonical() {
    let mut model = PresentationModel::default();
    model.set_bold(true);
    model.set_italic(true);
    model.set_alignment(LineAlignment::Right);
    model.append_html_island(erabasic_html::parse_document("<b>x</b>").unwrap());
    assert_eq!(model.style_bits(), 3);
    assert_eq!(model.alignment(), LineAlignment::Right);
    assert_eq!(
        model.snapshot().html_island,
        vec![erabasic_html::parse_document("<b>x</b>").unwrap()]
    );
    model.clear_html_island();
    assert!(model.snapshot().html_island.is_empty());
}

#[test]
fn plain_buttons_remain_on_the_current_logical_line() {
    let mut model = PresentationModel::default();
    model.append_button(
        "one".into(),
        ProtocolValue::Integer(1),
        InteractionToken { epoch: 1, id: 1 },
        None,
    );
    model.append_button(
        "two".into(),
        ProtocolValue::Integer(2),
        InteractionToken { epoch: 1, id: 2 },
        None,
    );
    let pending = model.snapshot();
    assert_eq!(pending.history.logical_lines.len(), 1);
    assert!(!pending.history.logical_lines[0].line_end);
    assert_eq!(pending.history.logical_lines[0].runs.len(), 2);
}

#[test]
fn automatic_buttons_are_grouped_after_the_complete_print_buffer_is_committed() {
    let mut model = PresentationModel::default();
    model.append_print_text("[1] one  ".into(), false, false);
    model.set_bold(true);
    model.append_print_text("[2] two".into(), false, true);
    assert_eq!(model.last_line_auto_button_values(), vec![1, 2]);
    let tokens = [
        InteractionToken { epoch: 4, id: 1 },
        InteractionToken { epoch: 4, id: 2 },
    ];
    assert_eq!(
        model.bind_last_line_auto_buttons(&tokens),
        vec![(tokens[0], 1), (tokens[1], 2)]
    );
    assert!(
        model.snapshot().history.logical_lines[0]
            .runs
            .iter()
            .all(|run| matches!(run, DisplayRun::Button { .. }))
    );

    let mut mixed = PresentationModel::default();
    mixed.append_print_text("[1] automatic ".into(), false, false);
    mixed.append_plain_print_text("[2] plain ".into(), false, false);
    mixed.append_print_text("[3] automatic".into(), false, true);
    let tokens = [
        InteractionToken { epoch: 1, id: 3 },
        InteractionToken { epoch: 1, id: 4 },
    ];
    assert_eq!(mixed.last_line_auto_button_values(), vec![1, 3]);
    assert_eq!(
        mixed.bind_last_line_auto_buttons(&tokens),
        vec![(tokens[0], 1), (tokens[1], 3)]
    );
    assert!(matches!(
        &mixed.snapshot().history.logical_lines[0].runs[1],
        DisplayRun::Text { text, .. } if text == "[2] plain "
    ));
}

#[test]
fn html_pop_serializes_semantic_button_values_and_consumes_pending_runs() {
    let mut model = PresentationModel::default();
    model.append_print_text("A<&".into(), false, false);
    model.append_button(
        "choose".into(),
        ProtocolValue::Integer(42),
        InteractionToken { epoch: 1, id: 9 },
        None,
    );
    assert_eq!(
        model.pop_printing_html(),
        "A&lt;&amp;<button value='42'>choose</button>"
    );
    assert_eq!(model.pop_printing_html(), "");
    assert!(model.last_line_is_empty());
}

#[test]
fn backgrounds_and_tooltips_are_recoverable_canonical_state() {
    let mut model = PresentationModel::default();
    model.set_projection(true, true, true, true, true);
    model.add_background("BACK".into(), 2, 128);
    model.set_tooltip_colors(0x0011_2233, 0x0044_5566);
    model.set_tooltip_delay(250).unwrap();
    model.set_tooltip_duration(100_000).unwrap();
    let snapshot = model.snapshot();
    assert_eq!(snapshot.backgrounds.len(), 1);
    assert_eq!(snapshot.backgrounds[0].depth, 2);
    assert_eq!(snapshot.backgrounds[0].opacity.numerator, 128);
    assert_eq!(snapshot.backgrounds[0].opacity.denominator, 255);
    assert_eq!(snapshot.tooltip.delay_ms, 250);
    assert_eq!(snapshot.tooltip.duration_ms, i16::MAX as u32);
}

#[test]
fn clearing_client_backgrounds_preserves_set_bg_image_state() {
    let mut model = PresentationModel::default();
    model.set_projection(true, true, true, true, true);
    model.add_background("PERSISTENT".into(), 2, 255);
    model.client_backgrounds.push(MediaPlacement {
        resource_id: "CBG".into(),
        x: LogicalLength(0),
        y: LogicalLength(0),
        width: LogicalLength(1),
        height: LogicalLength(1),
        depth: 1,
        opacity: RationalOpacity {
            numerator: 255,
            denominator: 255,
        },
        revision: 1,
        hover_resource_id: None,
        mask_resource_id: None,
        requested_width: None,
        requested_height: None,
        requested_y: None,
    });

    model.clear_client_backgrounds();

    let snapshot = model.snapshot();
    assert_eq!(snapshot.backgrounds.len(), 1);
    assert_eq!(snapshot.backgrounds[0].resource_id, "PERSISTENT");
}

#[test]
fn history_buttons_redraw_and_duplicate_backgrounds_remain_semantic() {
    let mut model = PresentationModel::default();
    model.set_projection(true, true, true, true, true);
    model.append_button(
        "old".into(),
        ProtocolValue::Integer(1),
        InteractionToken { epoch: 1, id: 1 },
        None,
    );
    model.append_text(String::new(), false);
    model.set_button_generation(1);
    model.add_background("SAME".into(), 1, 1);
    model.add_background("SAME".into(), 3, 2);
    model.set_tooltip_format(2 | 16 | (1_i64 << 40));
    model.set_redraw(false);
    let snapshot = model.snapshot();
    assert_eq!(snapshot.backgrounds.len(), 2);
    assert_eq!(snapshot.backgrounds[0].depth, 3);
    assert!(!snapshot.redraw.enabled);
    assert_eq!(
        snapshot.tooltip.normalized_format.flags,
        vec![
            era_runtime_protocol::TooltipFormatFlag::Right,
            era_runtime_protocol::TooltipFormatFlag::WordBreak,
        ]
    );
    assert_eq!(snapshot.tooltip.normalized_format.unknown_bits, 1_u64 << 40);
    assert!(
        snapshot
            .history
            .operations
            .iter()
            .all(|operation| matches!(operation, PresentationHistoryOperation::Append { .. }))
    );
    assert!(matches!(
        &snapshot.history.logical_lines[0].runs[0],
        DisplayRun::Button {
            enabled: false,
            generation: 0,
            ..
        }
    ));
    assert!(model.remove_background("SAME"));
    assert_eq!(model.snapshot().backgrounds.len(), 1);
}

#[test]
fn logical_line_count_tracks_reference_clearline_semantics() {
    let mut model = PresentationModel::default();
    model.append_text("one".into(), false);
    model.append_text("two".into(), false);
    assert_eq!(model.logical_line_count(), 2);

    model.append_print_text("pending".into(), false, false);
    model.delete_last_lines(2);
    assert_eq!(model.logical_line_count(), 1);
    assert_eq!(model.snapshot().history.logical_lines.len(), 1);

    model.delete_last_lines(3);
    assert_eq!(model.logical_line_count(), -2);
}

#[test]
fn max_log_trims_oldest_physical_lines_without_changing_linecount() {
    let mut model = PresentationModel::default();
    model.settings.maximum_physical_lines = 2;
    model.append_text("one".into(), false);
    model.append_text("two".into(), false);
    model.append_text("three".into(), false);

    let snapshot = model.snapshot();
    assert_eq!(model.logical_line_count(), 3);
    assert_eq!(snapshot.history.logical_lines.len(), 2);
    assert_eq!(snapshot.history.operations.len(), 2);
    assert!(
        snapshot
            .history
            .operations
            .iter()
            .all(|operation| matches!(operation, PresentationHistoryOperation::Append { .. }))
    );
}
