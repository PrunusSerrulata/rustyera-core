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
fn replay_button_description_uses_canonical_text_and_enabled_ordinal() {
    let mut model = PresentationModel::default();
    let first = InteractionToken { epoch: 4, id: 8 };
    let second = InteractionToken { epoch: 4, id: 9 };
    model.append_button("first".into(), ProtocolValue::Integer(7), first, None);
    model.append_button(
        "visible choice".into(),
        ProtocolValue::String("semantic".into()),
        second,
        None,
    );

    let description = model
        .replay_button(
            second,
            crate::input_replay::ReplayValue::String("semantic".into()),
        )
        .expect("enabled button description");

    assert_eq!(description.visible_text, "visible choice");
    assert_eq!(description.ordinal, 2);
    assert!(description.title.is_none());
    assert!(description.alt_text.is_none());
}

#[test]
fn button_generation_is_lazy_in_canonical_history_and_authoritative_at_boundaries() {
    let mut model = PresentationModel::default();
    let old = InteractionToken { epoch: 5, id: 1 };
    model.append_button("old".into(), ProtocolValue::Integer(1), old, None);
    model.set_button_generation(1);

    assert!(matches!(
        &model.pending_runs[0],
        DisplayRun::Button {
            enabled: true,
            generation: 0,
            ..
        }
    ));
    assert!(model.enabled_button_value(old).is_none());
    assert!(
        model
            .replay_button(old, crate::input_replay::ReplayValue::Integer("1".into()))
            .is_none()
    );
    assert!(matches!(
        &model.snapshot().history.logical_lines[0].runs[0],
        DisplayRun::Button {
            enabled: false,
            generation: 0,
            ..
        }
    ));

    let current = InteractionToken { epoch: 5, id: 2 };
    model.append_button("current".into(), ProtocolValue::Integer(2), current, None);
    assert_eq!(
        model.enabled_button_value(current),
        Some(VmValue::Integer(2))
    );

    let mut document = erabasic_html::parse_document("<button value='3'>island</button>").unwrap();
    assert!(install_test_html_interaction(&mut document.nodes, 5, 3, 1));
    model.append_html_island(document);
    model.set_button_generation(2);
    let snapshot = model.snapshot();
    let interaction = find_test_html_interaction(&snapshot.html_island[0].nodes)
        .expect("projected HTML interaction");
    assert!(!interaction.enabled);
}

fn install_test_html_interaction(
    nodes: &mut [erabasic_html::HtmlNode],
    epoch: u64,
    id: u64,
    generation: u64,
) -> bool {
    for node in nodes {
        let erabasic_html::HtmlNode::Element {
            interaction,
            children,
            ..
        } = node
        else {
            continue;
        };
        if interaction.is_none() {
            *interaction = Some(erabasic_html::HtmlInteraction {
                epoch,
                id,
                integer_value: Some(3),
                string_value: None,
                generation,
                enabled: true,
            });
            return true;
        }
        if install_test_html_interaction(children, epoch, id, generation) {
            return true;
        }
    }
    false
}

fn find_test_html_interaction(
    nodes: &[erabasic_html::HtmlNode],
) -> Option<&erabasic_html::HtmlInteraction> {
    for node in nodes {
        let erabasic_html::HtmlNode::Element {
            interaction,
            children,
            ..
        } = node
        else {
            continue;
        };
        if interaction.is_some() {
            return interaction.as_ref();
        }
        if let Some(interaction) = find_test_html_interaction(children) {
            return Some(interaction);
        }
    }
    None
}
