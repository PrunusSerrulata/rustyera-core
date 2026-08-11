use super::projection::plain_text;
use super::*;

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

#[test]
fn project_default_style_changes_update_existing_default_runs() {
    let mut model = PresentationModel::default();
    let mut project_style = model.default_style.clone();
    project_style.font_family = Some("initial-project-font".into());
    project_style.font_millipixels = 18_000;
    model.apply_project_default_style(project_style);
    model.reset_style();
    model.append_print_text("default".into(), false, true);
    model.set_bold(true);
    model.append_print_text("bold".into(), false, true);
    model.set_font(Some("explicit".into()));
    model.append_print_text("explicit".into(), false, true);
    model.reset_style();
    model.append_separator("-".into());

    let mut next = model.default_style.clone();
    next.font_family = Some("project-default".into());
    next.font_millipixels = 20_000;
    model.apply_project_default_style(next.clone());

    let styles = model
        .lines
        .iter()
        .map(|line| match &line.runs[0] {
            DisplayRun::Text { style, .. } | DisplayRun::Separator { style, .. } => style,
            _ => panic!("test line must contain styled text or a separator"),
        })
        .collect::<Vec<_>>();
    assert_eq!(styles[0].font_family, next.font_family);
    assert_eq!(styles[0].font_millipixels, 20_000);
    assert_eq!(styles[1].font_family, next.font_family);
    assert_eq!(styles[1].font_millipixels, 20_000);
    assert!(styles[1].bold);
    assert_eq!(styles[2].font_family.as_deref(), Some("explicit"));
    assert_eq!(styles[2].font_millipixels, 20_000);
    assert_eq!(styles[3].font_family, next.font_family);
    assert_eq!(styles[3].font_millipixels, 20_000);
    assert_eq!(model.delivery.dirty_lines, BTreeSet::from([1, 2, 3, 4]));
    assert!(!model.delivery.dirty.force_snapshot);
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
    model.play_bgm("sound".into());
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
    assert_eq!(
        committed.history.logical_lines[0]
            .runs
            .iter()
            .filter(|run| matches!(run, DisplayRun::ColumnCell { .. }))
            .count(),
        2
    );
    assert_eq!(
        committed.history.logical_lines[0]
            .runs
            .iter()
            .filter_map(display_text)
            .collect::<String>(),
        "done"
    );
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
        .filter_map(display_text)
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
fn plain_projection_treats_ambiguous_cjk_glyphs_as_full_width() {
    let mut model = PresentationModel::default();
    model.set_projection(false, false, false, false, false);
    model.append_column_cell("■……■".into(), CellAlignment::Left);
    let snapshot = model.snapshot();
    let text = snapshot.history.logical_lines[0]
        .runs
        .iter()
        .filter_map(display_text)
        .collect::<String>();

    assert_eq!(text, format!("■……■{}", " ".repeat(17)));
}

#[test]
fn projection_attaches_runtime_columns_to_nested_and_fallback_text() {
    let projected = super::projection::project_runs(
        vec![
            DisplayRun::Button {
                runs: vec![plain_text("■".into(), 19_000)],
                token: InteractionToken { epoch: 1, id: 1 },
                title: None,
                hover_style: None,
                value: ProtocolValue::Integer(1),
                generation: 0,
                enabled: true,
            },
            DisplayRun::ColumnCell {
                content: vec![plain_text("……".into(), 19_000)],
                alignment: CellAlignment::Left,
                preferred_columns: 4,
            },
            DisplayRun::HtmlDocument {
                document: erabasic_html::parse_document("<b>■……■</b>").unwrap(),
            },
            DisplayRun::Image {
                placement: MediaPlacement {
                    resource_id: "missing.png".into(),
                    x: LogicalLength(0),
                    y: LogicalLength(0),
                    width: LogicalLength(0),
                    height: LogicalLength(0),
                    depth: 0,
                    opacity: RationalOpacity {
                        numerator: 255,
                        denominator: 255,
                    },
                    revision: 0,
                    hover_resource_id: None,
                    mask_resource_id: None,
                    requested_width: None,
                    requested_height: None,
                    requested_y: None,
                },
                alt_text: Some("…".into()),
            },
        ],
        false,
        false,
        19_000,
        false,
        false,
        erabasic_vm::CharacterWidthMode::Automatic,
    );
    let mut layouts = Vec::new();
    collect_text_layouts(&projected, &mut layouts);
    assert!(layouts.contains(&("■", 2)));
    assert_eq!(
        layouts.iter().filter(|layout| **layout == ("…", 2)).count(),
        3
    );
    assert!(layouts.contains(&("■……■", 8)));
}

#[test]
fn projection_uses_the_selected_width_mode_for_console_text_but_not_html() {
    let runs = vec![
        plain_text("☀❤……".into(), 19_000),
        DisplayRun::HtmlDocument {
            document: erabasic_html::parse_document("<b>☀❤</b>").unwrap(),
        },
    ];
    let automatic = super::projection::project_runs(
        runs.clone(),
        true,
        true,
        19_000,
        true,
        true,
        erabasic_vm::CharacterWidthMode::Automatic,
    );
    let narrow = super::projection::project_runs(
        runs,
        true,
        true,
        19_000,
        true,
        true,
        erabasic_vm::CharacterWidthMode::AmbiguousNarrow,
    );

    let mut automatic_layouts = Vec::new();
    collect_text_layouts(&automatic, &mut automatic_layouts);
    assert_eq!(automatic_layouts, [("☀", 2), ("❤", 2), ("…", 2), ("…", 2)]);
    let mut narrow_layouts = Vec::new();
    collect_text_layouts(&narrow, &mut narrow_layouts);
    assert_eq!(narrow_layouts, [("☀", 1), ("❤", 1), ("…", 1), ("…", 1)]);
    assert!(matches!(
        automatic.last(),
        Some(DisplayRun::HtmlDocument { .. })
    ));
    assert!(matches!(
        narrow.last(),
        Some(DisplayRun::HtmlDocument { .. })
    ));
}

#[test]
fn projection_preserves_internal_spaces_across_adjacent_runs_with_the_same_style() {
    let projected = super::projection::project_runs(
        vec![
            plain_text("A ".into(), 19_000),
            plain_text(String::new(), 19_000),
            plain_text(" B C".into(), 19_000),
        ],
        false,
        false,
        19_000,
        false,
        false,
        erabasic_vm::CharacterWidthMode::Automatic,
    );
    let mut layouts = Vec::new();
    collect_text_layouts(&projected, &mut layouts);

    assert_eq!(
        layouts,
        [
            ("A", 1),
            (" ", 1),
            ("", 0),
            (" ", 1),
            ("B", 1),
            (" ", 1),
            ("C", 1),
        ]
    );
}

#[test]
fn projection_only_suppresses_alignment_space_before_double_vertical_edge() {
    let mut changed_style = super::projection::plain_text("B C ".into(), 19_000);
    let DisplayRun::Text { style, .. } = &mut changed_style else {
        panic!("plain_text must create a text run")
    };
    style.foreground.red = 1;
    let mut double_edge = super::projection::plain_text("║".into(), 19_000);
    let DisplayRun::Text { style, .. } = &mut double_edge else {
        panic!("plain_text must create a text run")
    };
    style.foreground.blue = 1;
    let system_reference = SystemTextRef {
        key: SystemTextKey::PressAnyKey,
        arguments: Vec::new(),
    };
    let projected = super::projection::project_runs(
        vec![
            plain_text("A ".into(), 19_000),
            changed_style,
            DisplayRun::Button {
                runs: vec![plain_text("D ".into(), 19_000)],
                token: InteractionToken { epoch: 1, id: 1 },
                title: None,
                hover_style: None,
                value: ProtocolValue::Integer(1),
                generation: 0,
                enabled: true,
            },
            plain_text("Q ".into(), 19_000),
            plain_text("F ".into(), 19_000),
            double_edge,
            DisplayRun::Text {
                text: "E ".into(),
                style: super::projection::default_style(),
                system_text: Some(system_reference.clone()),
            },
        ],
        false,
        false,
        19_000,
        false,
        false,
        erabasic_vm::CharacterWidthMode::Automatic,
    );
    let mut layouts = Vec::new();
    collect_text_layouts(&projected, &mut layouts);

    assert_eq!(
        layouts,
        [
            ("A", 1),
            (" ", 1),
            ("B", 1),
            (" ", 1),
            ("C", 1),
            (" ", 1),
            ("D", 1),
            (" ", 1),
            ("Q", 1),
            (" ", 1),
            ("F", 1),
            (" ", 0),
            ("║", 2),
            ("E", 1),
            (" ", 1),
        ]
    );
    let tail = &projected[projected.len() - 2..];
    assert!(matches!(
        tail,
        [
            DisplayRun::TextLayout {
                text,
                system_text: Some(reference),
                ..
            },
            DisplayRun::TextLayout {
                text: space,
                system_text: None,
                columns: 1,
                ..
            }
        ] if text == "E" && reference == &system_reference && space == " "
    ));
}

#[test]
fn plain_separator_fallback_fills_logical_columns_with_ambiguous_patterns() {
    let mut separator_style = TextStyle::default();
    separator_style.foreground.red = 18;
    let projected = super::projection::project_runs(
        vec![DisplayRun::Separator {
            pattern: "■A".into(),
            role: SeparatorRole::Rule,
            style: separator_style.clone(),
        }],
        false,
        false,
        19_000,
        false,
        false,
        erabasic_vm::CharacterWidthMode::Automatic,
    );
    assert!(matches!(
        projected.as_slice(),
        [DisplayRun::TextLayout { text, style, columns: 75, .. }]
            if text == &"■A".repeat(25) && style == &separator_style
    ));
}

#[test]
fn separator_flushes_existing_text_to_an_independent_line() {
    let mut model = PresentationModel::default();
    model.append_print_text("prefix".into(), false, false);
    model.set_foreground(0x12_34_56);
    model.current_style.background = Some(rgb_color(0x65_43_21));
    model.set_font(Some("separator-font".into()));
    model.current_style.font_millipixels = 21_000;
    model.set_font_style(1 | 2 | 4 | 8);
    model.append_separator("=".into());
    let snapshot = model.snapshot();
    assert_eq!(snapshot.history.logical_lines.len(), 2);
    assert!(matches!(
        &snapshot.history.logical_lines[1].runs[0],
        DisplayRun::Separator { pattern, style, .. }
            if pattern == "="
                && style.foreground.red == 0x12
                && style.foreground.green == 0x34
                && style.foreground.blue == 0x56
                && style.background == Some(rgb_color(0x65_43_21))
                && style.font_family.as_deref() == Some("separator-font")
                && style.font_millipixels == 21_000
                && !style.bold
                && !style.italic
                && !style.underline
                && !style.strikeout
    ));
}

#[test]
fn separator_style_defaults_when_restoring_legacy_messagepack() {
    #[derive(Serialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum LegacyDisplayRun {
        Separator {
            pattern: String,
            role: SeparatorRole,
        },
    }

    let encoded = rmp_serde::to_vec(&LegacyDisplayRun::Separator {
        pattern: "-".into(),
        role: SeparatorRole::Rule,
    })
    .unwrap();
    let restored: DisplayRun = rmp_serde::from_slice(&encoded).unwrap();
    assert!(matches!(
        restored,
        DisplayRun::Separator { pattern, style, .. }
            if pattern == "-" && style == TextStyle::default()
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
    assert_eq!(
        snapshot.history.logical_lines[1]
            .runs
            .iter()
            .filter_map(display_text)
            .collect::<String>(),
        "invalid"
    );
}

#[test]
fn logical_line_string_uses_width_without_splitting_graphemes() {
    assert_eq!(erabasic_vm::logical_line_string("界", 5), Ok("界界".into()));
    assert_eq!(
        erabasic_vm::logical_line_string("e\u{301}", 3),
        Ok("e\u{301}e\u{301}e\u{301}".into())
    );
    assert!(erabasic_vm::logical_line_string("\u{301}", 10).is_err());
    assert!(erabasic_vm::logical_line_string("", 10).is_err());
}

#[test]
fn style_and_media_are_canonical_but_capability_projected() {
    let mut model = PresentationModel::default();
    model.set_font_style(1 | 8);
    model.set_alignment(LineAlignment::Center);
    model.append_print_text("styled".into(), false, true);
    model.append_html(erabasic_html::parse_document("<b>fallback</b>").unwrap());
    model.append_image("image.png".into(), Some("image".into()));
    model.play_bgm("sound.ogg".into());

    let fallback = model.snapshot();
    assert!(fallback.audio.is_empty());
    assert_eq!(
        fallback.history.logical_lines[0].alignment,
        LineAlignment::Center
    );
    let DisplayRun::TextLayout { style, .. } = &fallback.history.logical_lines[0].runs[0] else {
        panic!("first run must be text");
    };
    assert!(style.bold);
    assert!(style.underline);
    assert!(matches!(
        &fallback.history.logical_lines[1].runs[0],
        DisplayRun::TextLayout { text, .. } if text == "fallback"
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
    let snapshot = mixed.snapshot();
    let plain = snapshot.history.logical_lines[0]
        .runs
        .iter()
        .filter(|run| !matches!(run, DisplayRun::Button { .. }))
        .filter_map(display_text)
        .collect::<String>();
    assert_eq!(plain, "[2] plain ");
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
