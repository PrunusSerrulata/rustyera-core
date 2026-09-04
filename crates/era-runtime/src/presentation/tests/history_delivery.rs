#[test]
fn deferred_tail_replacements_collapse_to_the_final_frame() {
    let mut model = PresentationModel::default();
    model.append_text("stable".into(), false);
    let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
        panic!("initial delivery must establish a snapshot");
    };

    for frame in ["frame 0", "frame 1", "final"] {
        model.delete_last_lines(1);
        model.append_text(frame.into(), false);
    }

    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("tail replacement must remain incremental");
    };
    let structural_operations = delta
        .operations
        .iter()
        .filter(|operation| {
            matches!(
                operation,
                PresentationOperation::AppendLine { .. }
                    | PresentationOperation::DeleteLines { .. }
            )
        })
        .count();
    assert_eq!(structural_operations, 2);

    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());
}

#[test]
fn production_history_reducer_preserves_button_generations_and_structural_edits() {
    let mut model = PresentationModel::default();
    model.settings.maximum_physical_lines = 3;
    let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
        panic!("initial delivery must establish a snapshot");
    };

    model.append_button(
        "old".into(),
        ProtocolValue::Integer(1),
        InteractionToken { epoch: 1, id: 1 },
        None,
    );
    model.append_text(String::new(), false);
    model.set_button_generation(1);
    model.append_button(
        "current".into(),
        ProtocolValue::Integer(2),
        InteractionToken { epoch: 1, id: 2 },
        None,
    );
    model.append_text(String::new(), false);
    model.set_button_generation(2);
    model.set_button_generation(3);
    model.append_button(
        "new".into(),
        ProtocolValue::Integer(3),
        InteractionToken { epoch: 1, id: 3 },
        None,
    );
    model.append_text(String::new(), false);
    model.append_text("trimmed by maxlog".into(), false);
    model.delete_last_lines(1);
    model.print_temporary_line("temporary one".into());
    model.print_temporary_line("temporary two".into());

    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("mixed history edits must remain incremental");
    };
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());
}

#[test]
fn first_temporary_line_appends_without_a_spurious_replacement() {
    let mut model = PresentationModel::default();
    let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
        panic!("initial delivery must establish a snapshot");
    };

    model.print_temporary_line("first".into());
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("first temporary line must remain incremental");
    };
    assert!(matches!(
        delta.operations.as_slice(),
        [PresentationOperation::AppendLine { line }]
            if line.temporary && line.line_end
    ));
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());
}

#[test]
fn delivered_committed_temporary_line_is_deleted_then_appended() {
    let mut model = PresentationModel::default();
    let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
        panic!("initial delivery must establish a snapshot");
    };
    model.print_temporary_line("first".into());
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("first temporary line must remain incremental");
    };
    apply_delta(&mut frontend, delta);

    model.print_temporary_line("replacement".into());
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("committed temporary replacement must remain incremental");
    };
    assert!(matches!(
        delta.operations.as_slice(),
        [
            PresentationOperation::DeleteLines { count: 1 },
            PresentationOperation::AppendLine { line },
        ] if line.temporary && line.line_end
    ));
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());
}

#[test]
fn delivered_pending_temporary_line_is_replaced_in_place() {
    let mut model = PresentationModel::default();
    let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
        panic!("initial delivery must establish a snapshot");
    };
    model.append_print_text("pending".into(), true, false);
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("pending temporary line must remain incremental");
    };
    apply_delta(&mut frontend, delta);

    model.print_temporary_line("replacement".into());
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("delivered pending replacement must remain incremental");
    };
    assert!(matches!(
        delta.operations.as_slice(),
        [PresentationOperation::ReplaceLine { line, .. }]
            if line.temporary && line.line_end
    ));
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());
}

#[test]
fn undelivered_pending_temporary_line_emits_only_its_replacement() {
    let mut model = PresentationModel::default();
    let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
        panic!("initial delivery must establish a snapshot");
    };
    model.append_print_text("never delivered".into(), true, false);
    model.print_temporary_line("replacement".into());

    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("undelivered pending replacement must remain incremental");
    };
    assert!(matches!(
        delta.operations.as_slice(),
        [PresentationOperation::AppendLine { line }]
            if line.temporary && line.line_end
    ));
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());
}

#[test]
fn empty_temporary_text_forces_an_empty_temporary_line() {
    let mut model = PresentationModel::default();
    let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
        panic!("initial delivery must establish a snapshot");
    };

    model.print_temporary_line(String::new());
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("empty temporary line must remain incremental");
    };
    assert!(matches!(
        delta.operations.as_slice(),
        [PresentationOperation::AppendLine { line }]
            if line.temporary && line.line_end && line.runs.iter().all(run_is_empty)
    ));
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());
}

#[test]
fn pending_line_deletions_distinguish_undelivered_and_delivered_rows() {
    let mut model = PresentationModel::default();
    model.append_text("baseline one".into(), false);
    model.append_text("baseline two".into(), false);
    let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
        panic!("initial delivery must establish a snapshot");
    };

    model.append_print_text("never delivered".into(), false, false);
    model.delete_last_lines(1);
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("undelivered pending deletion must remain incremental");
    };
    assert!(
        delta
            .operations
            .iter()
            .all(|operation| !matches!(operation, PresentationOperation::DeleteLines { .. }))
    );
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());

    model.append_print_text("delivered pending".into(), false, false);
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("pending line must be delivered incrementally");
    };
    apply_delta(&mut frontend, delta);
    model.delete_last_lines(2);
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("pending plus committed deletion must remain incremental");
    };
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());
}

#[test]
fn delivered_pending_commit_then_delete_preserves_the_frontend_deletion() {
    let mut model = PresentationModel::default();
    model.append_text("baseline".into(), false);
    let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
        panic!("initial delivery must establish a snapshot");
    };
    model.append_print_text("pending".into(), false, false);
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("pending line must be delivered incrementally");
    };
    apply_delta(&mut frontend, delta);

    model.flush_pending_line();
    model.delete_last_lines(1);
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("pending commit and deletion must remain incremental");
    };
    assert!(
        delta
            .operations
            .iter()
            .any(|operation| matches!(operation, PresentationOperation::DeleteLines { count: 1 }))
    );
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());
}

#[test]
fn delivered_pending_commit_reenables_tail_redraw_compaction() {
    let mut model = PresentationModel::default();
    model.append_text("baseline".into(), false);
    let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
        panic!("initial delivery must establish a snapshot");
    };
    model.append_print_text("pending".into(), false, false);
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("pending line must be delivered incrementally");
    };
    apply_delta(&mut frontend, delta);

    model.flush_pending_line();
    for frame in 0..100 {
        model.delete_last_lines(1);
        model.append_text(format!("frame {frame}"), false);
    }
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("pending commit and redraw must remain incremental");
    };
    assert!(matches!(
        delta.operations.as_slice(),
        [
            PresentationOperation::ReplaceLine { .. },
            PresentationOperation::DeleteLines { count: 1 },
            PresentationOperation::AppendLine { line },
        ] if line.runs.iter().any(|run| matches!(run, DisplayRun::TextLayout { text, .. } if text == "9"))
    ));
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());
}

#[test]
fn pending_button_generation_projects_the_final_enabled_state() {
    let mut model = PresentationModel::default();
    let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
        panic!("initial delivery must establish a snapshot");
    };
    model.append_button(
        "pending old".into(),
        ProtocolValue::Integer(1),
        InteractionToken { epoch: 1, id: 1 },
        None,
    );
    model.set_button_generation(1);
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("pending generation update must remain incremental");
    };
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());
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
    model.add_background("BACK".into(), 8, 2, 128);
    model.set_tooltip_colors(0x0011_2233, 0x0044_5566);
    model.set_tooltip_delay(250).unwrap();
    model.set_tooltip_duration(100_000).unwrap();
    let snapshot = model.snapshot();
    assert_eq!(snapshot.scene.layers.len(), 1);
    assert_eq!(snapshot.scene.layers[0].depth, 2);
    assert_eq!(snapshot.scene.layers[0].opacity, 128);
    assert!(matches!(
        &snapshot.scene.layers[0].source,
        SceneSourceV1::Sprite { sprite_name, resource_revision }
            if sprite_name == "BACK" && *resource_revision == 8
    ));
    assert_eq!(snapshot.tooltip.delay_ms, 250);
    assert_eq!(snapshot.tooltip.duration_ms, i16::MAX as u32);
}

#[test]
fn clearing_client_backgrounds_preserves_set_bg_image_state() {
    let mut model = PresentationModel::default();
    model.set_projection(true, true, true, true, true);
    model.add_background("PERSISTENT".into(), 9, 2, 255);

    let before = model.snapshot().scene.revision;
    model.clear_client_backgrounds();

    let snapshot = model.snapshot();
    assert_eq!(snapshot.scene.revision, before + 1);
    assert_eq!(snapshot.scene.layers.len(), 1);
    assert!(matches!(
        &snapshot.scene.layers[0].source,
        SceneSourceV1::Sprite { sprite_name, .. } if sprite_name == "PERSISTENT"
    ));
}

#[test]
fn empty_background_clear_still_advances_scene_revision() {
    let mut model = PresentationModel::default();
    model.set_projection(true, true, true, true, true);
    let before = model.snapshot().scene.revision;
    model.clear_backgrounds();
    let snapshot = model.snapshot();
    assert_eq!(snapshot.scene.revision, before + 1);
    assert!(snapshot.scene.layers.is_empty());
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
    model.add_background("SAME".into(), 10, 1, 1);
    model.add_background("SAME".into(), 11, 3, 2);
    model.set_tooltip_format(2 | 16 | (1_i64 << 40));
    model.set_redraw(false);
    let snapshot = model.snapshot();
    assert_eq!(snapshot.scene.layers.len(), 2);
    assert_eq!(snapshot.scene.layers[0].depth, 3);
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
    assert_eq!(model.snapshot().scene.layers.len(), 1);
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

#[test]
fn canonical_history_and_delivery_journal_share_committed_lines() {
    let mut model = PresentationModel::default();
    model.append_text("shared".into(), false);

    let canonical = model.lines.back().expect("committed canonical line");
    let PresentationHistoryEdit::Append { line: journal } = model
        .history_edits
        .last()
        .expect("committed journal append")
    else {
        panic!("committed line must be journaled as an append");
    };
    assert!(Arc::ptr_eq(canonical, journal));
}

#[test]
fn canonical_cow_mutations_do_not_rewrite_queued_journal_lines() {
    let mut model = PresentationModel::default();
    let PresentationUpdate::Snapshot(_) = model.next_update() else {
        panic!("initial delivery must establish a snapshot");
    };
    let old = InteractionToken { epoch: 1, id: 1 };
    model.append_button("old".into(), ProtocolValue::Integer(1), old, None);
    model.append_text(String::new(), false);
    let PresentationHistoryEdit::Append { line: journal } =
        model.history_edits.last().expect("queued journal append")
    else {
        panic!("committed line must be journaled as an append");
    };
    let journal = Arc::clone(journal);

    let rebound = InteractionToken { epoch: 2, id: 9 };
    model.rebind_interactions(&BTreeMap::from([(old, rebound)]), &BTreeMap::new());
    assert!(!Arc::ptr_eq(model.lines.back().unwrap(), &journal));
    assert!(matches!(
        &journal.runs[0],
        DisplayRun::Button { token, .. } if *token == old
    ));
    assert!(matches!(
        &model.lines.back().unwrap().runs[0],
        DisplayRun::Button { token, .. } if *token == rebound
    ));

    let PresentationUpdate::Snapshot(snapshot) = model.next_update() else {
        panic!("interaction rebinding requires an authoritative snapshot");
    };
    assert_visible_snapshot_eq(&snapshot, &model.snapshot());
}

#[test]
fn default_style_cow_delivers_the_canonical_replacement() {
    let mut model = PresentationModel::default();
    let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
        panic!("initial delivery must establish a snapshot");
    };
    model.append_text("styled".into(), false);
    let PresentationHistoryEdit::Append { line: journal } =
        model.history_edits.last().expect("queued journal append")
    else {
        panic!("committed line must be journaled as an append");
    };
    let journal = Arc::clone(journal);
    let mut next = model.default_style.clone();
    next.font_family = Some("replacement".into());
    model.apply_project_default_style(next);

    assert!(!Arc::ptr_eq(model.lines.back().unwrap(), &journal));
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("default style replacement must remain incremental");
    };
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());
}

#[test]
fn redraw_tail_replacement_keeps_the_runtime_journal_bounded() {
    let mut model = PresentationModel::default();
    model.settings.maximum_physical_lines = 2;
    model.append_text("baseline one".into(), false);
    model.append_text("baseline two".into(), false);
    let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
        panic!("initial delivery must establish a snapshot");
    };

    for frame in 0..100 {
        model.delete_last_lines(2);
        model.append_text(format!("frame {frame} one"), false);
        model.append_text(format!("frame {frame} two"), false);
        assert!(model.history_edits.len() <= 3);
    }

    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("replacement must remain incremental");
    };
    assert_eq!(delta.operations.len(), 3);
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());
}

#[test]
fn max_log_rollover_delivers_only_the_retained_window() {
    let mut model = PresentationModel::default();
    model.settings.maximum_physical_lines = 2;
    model.append_text("baseline one".into(), false);
    model.append_text("baseline two".into(), false);
    let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
        panic!("initial delivery must establish a snapshot");
    };

    for line in 0..100 {
        model.append_text(format!("rollover {line}"), false);
    }
    let PresentationUpdate::Delta(delta) = model.next_update() else {
        panic!("rollover must remain incremental");
    };
    assert!(matches!(
        delta.operations.as_slice(),
        [
            PresentationOperation::TrimLines { count: 2 },
            PresentationOperation::AppendLine { .. },
            PresentationOperation::AppendLine { .. }
        ]
    ));
    apply_delta(&mut frontend, delta);
    assert_visible_snapshot_eq(&frontend, &model.snapshot());
}
