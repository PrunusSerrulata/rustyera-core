use super::super::super::{HtmlElementSemantic, HtmlNode};
use super::*;

fn settings() -> HtmlStringLengthSettings {
    HtmlStringLengthSettings {
        font_size_pixels: 20,
        drawable_width_pixels: 20,
        prevent_button_wrap: true,
        legacy_nonbutton_wrap: false,
        foreground_rgb: 0x00ff_ffff,
        focus_rgb: 0x00ff_ff00,
    }
}

fn plan(source: &str) -> HtmlStringLengthPlan {
    HtmlStringLengthPlan::new(source, settings(), 1, HtmlQueryLimits::default()).unwrap()
}

fn text(document: &HtmlDocument) -> String {
    fn visit(nodes: &[HtmlNode], result: &mut String) {
        for node in nodes {
            match node {
                HtmlNode::Text { text, .. } => result.push_str(text),
                HtmlNode::Element { children, .. } => visit(children, result),
            }
        }
    }
    let mut result = String::new();
    visit(&document.nodes, &mut result);
    result
}

// These widths are synthetic inputs to core layout, never browser/font evidence.
fn synthetic(probe: &HtmlLengthProbe, scalar_millipixels: i64) -> HtmlLengthMeasuredValue {
    match &probe.kind {
        HtmlLengthProbeKind::TextPart { cuts, .. } => HtmlLengthMeasuredValue::TextPart {
            prefix_advances_millipixels: (0..cuts.len())
                .map(|index| i64::try_from(index).unwrap() * scalar_millipixels)
                .collect(),
        },
        HtmlLengthProbeKind::ImageSlot { .. } => {
            HtmlLengthMeasuredValue::ImageSlot(HtmlLengthImageResolution::Loaded {
                natural_width: 100,
                natural_height: 100,
            })
        }
        HtmlLengthProbeKind::FallbackText => HtmlLengthMeasuredValue::FallbackText {
            advance_millipixels: i64::try_from(text(&probe.document).chars().count()).unwrap()
                * scalar_millipixels,
        },
        HtmlLengthProbeKind::FixedSlot => HtmlLengthMeasuredValue::FixedSlotReady,
    }
}

fn measure_all(plan: &mut HtmlStringLengthPlan, scalar_millipixels: i64) {
    for probe in plan.probes().to_vec() {
        plan.resume(HtmlLengthMeasurement {
            probe_id: probe.id,
            value: synthetic(&probe, scalar_millipixels),
        })
        .unwrap();
    }
}

fn drive(plan: &mut HtmlStringLengthPlan, scalar_millipixels: i64) -> HtmlStringLengthResult {
    loop {
        match plan.poll().unwrap() {
            HtmlStringLengthPoll::Complete(result) => return result,
            HtmlStringLengthPoll::NeedMeasurements { probe_ids } => {
                assert!(!probe_ids.is_empty());
                for id in probe_ids {
                    let probe = plan.probes()[usize::try_from(id).unwrap()].clone();
                    plan.resume(HtmlLengthMeasurement {
                        probe_id: id,
                        value: synthetic(&probe, scalar_millipixels),
                    })
                    .unwrap();
                }
            }
        }
    }
}

fn pixels(source: &str, settings: HtmlStringLengthSettings) -> i64 {
    let mut plan =
        HtmlStringLengthPlan::new(source, settings, 1, HtmlQueryLimits::default()).unwrap();
    drive(&mut plan, 5000).first_line_pixels
}

#[test]
fn complete_parse_and_all_later_line_measurements_precede_success() {
    assert!(
        HtmlStringLengthPlan::new("A<br><unknown>", settings(), 1, HtmlQueryLimits::default())
            .is_err()
    );
    let mut plan = plan("A<br>B");
    assert_eq!(plan.probes().len(), 2);
    let first = plan.probes()[0].clone();
    plan.resume(HtmlLengthMeasurement {
        probe_id: first.id,
        value: synthetic(&first, 5000),
    })
    .unwrap();
    assert_eq!(
        plan.finish().unwrap_err().kind,
        HtmlQueryErrorKind::InvalidMeasurement
    );
    assert!(
        plan.resume(HtmlLengthMeasurement {
            probe_id: first.id,
            value: synthetic(&first, 5000)
        })
        .is_err()
    );
    assert!(
        plan.resume(HtmlLengthMeasurement {
            probe_id: 1,
            value: HtmlLengthMeasuredValue::FixedSlotReady
        })
        .is_err()
    );
    assert!(plan.finish().is_err());
    let second = plan.probes()[1].clone();
    plan.resume(HtmlLengthMeasurement {
        probe_id: second.id,
        value: synthetic(&second, 5000),
    })
    .unwrap();
    assert_eq!(plan.finish().unwrap().first_line_pixels, 5);
}

#[test]
fn button_wrap_nonbutton_legacy_and_clearbutton_policies_choose_first_row() {
    assert_eq!(pixels("aa<button value='1'>bbb</button>", settings()), 10);
    assert_eq!(
        pixels(
            "aa<button value='1'>bbb</button>",
            HtmlStringLengthSettings {
                prevent_button_wrap: false,
                ..settings()
            }
        ),
        20
    );
    assert_eq!(pixels("aa<button>bbb</button>", settings()), 20);
    assert_eq!(
        pixels(
            "aa<nonbutton>bbb</nonbutton>",
            HtmlStringLengthSettings {
                legacy_nonbutton_wrap: true,
                ..settings()
            }
        ),
        10
    );
    assert_eq!(
        pixels(
            "aa<clearbutton><button value='1'>bbb</button></clearbutton>",
            settings()
        ),
        20
    );
    assert_eq!(pixels("<button value='1'>abcdef</button>", settings()), 20);
}

#[test]
fn each_text_part_is_quantized_before_integer_width_accumulation() {
    let mut plan = plan("<nobr>a<b>b</b>c</nobr>");
    assert_eq!(plan.probes().len(), 3);
    measure_all(&mut plan, 5900);
    assert_eq!(plan.finish().unwrap().first_line_pixels, 15);
}

#[test]
fn positions_do_not_turn_width_sum_into_bounding_box() {
    assert_eq!(
        pixels(
            "<nobr><nonbutton pos='-100'>ab</nonbutton><button value='1' pos='1000'>c</button></nobr>",
            settings()
        ),
        15
    );
    assert!(
        HtmlStringLengthPlan::new(
            "<button pos='100'>x</button>",
            settings(),
            1,
            HtmlQueryLimits::default()
        )
        .is_err()
    );
    assert!(
        HtmlStringLengthPlan::new(
            "<p align='right'><nobr><nonbutton pos='100'>x</nonbutton></nobr></p>",
            settings(),
            1,
            HtmlQueryLimits::default()
        )
        .is_err()
    );
}

#[test]
fn absolute_division_preserves_negative_group_width_and_signed_half_units() {
    let source = "<nobr><nonbutton pos='95'>x</nonbutton>y<div width='100px' height='20px' display='absolute'></div></nobr>";
    for (flag, expected) in [(1, -19), (-7, -19), (i64::MIN, -19), (0, -2)] {
        let mut plan =
            HtmlStringLengthPlan::new(source, settings(), flag, HtmlQueryLimits::default())
                .unwrap();
        measure_all(&mut plan, 5000);
        assert_eq!(
            plan.finish().unwrap(),
            HtmlStringLengthResult {
                first_line_pixels: -19,
                value: expected
            }
        );
    }
}

#[test]
fn default_length_units_wrap_int32_and_round_using_the_original_width_sign() {
    for (pixels, expected) in [
        (i64::from(i32::MAX), 1),
        (i64::from(i32::MIN), 0),
        (1_073_741_824, -119_304_646),
        (-1_073_741_826, 119_304_645),
        (10, 2),
        (-10, -2),
        (0, 0),
    ] {
        assert_eq!(html_string_length_units(pixels, 0, 18).unwrap(), expected);
        for flag in [1, -7, i64::MIN] {
            assert_eq!(html_string_length_units(pixels, flag, 18).unwrap(), pixels);
        }
    }
    for pixels in [i64::from(i32::MIN) - 1, i64::from(i32::MAX) + 1] {
        for flag in [0, 1] {
            assert_eq!(
                html_string_length_units(pixels, flag, 18).unwrap_err().kind,
                HtmlQueryErrorKind::ResourceLimit
            );
        }
    }
    assert_eq!(
        html_string_length_units(10, 0, 0).unwrap_err().kind,
        HtmlQueryErrorKind::InvalidMeasurement
    );
    assert_eq!(
        html_string_length_units(10, 0, -18).unwrap_err().kind,
        HtmlQueryErrorKind::InvalidMeasurement
    );
}

#[test]
fn completed_shape_plan_uses_wrapped_units_without_relaxing_layout_bounds() {
    // A power-of-two pixel slot is exact in the reference float32 shape width.
    // Only slot readiness is synthetic; geometry and units are calculated by core.
    let source = "<nobr><shape type='space' param='1073741824px'></nobr>";
    for (flag, expected) in [(0, -119_304_646), (1, 1_073_741_824), (-7, 1_073_741_824)] {
        let mut plan = HtmlStringLengthPlan::new(
            source,
            HtmlStringLengthSettings {
                font_size_pixels: 18,
                ..settings()
            },
            flag,
            HtmlQueryLimits::default(),
        )
        .unwrap();
        assert_eq!(
            drive(&mut plan, 5000),
            HtmlStringLengthResult {
                first_line_pixels: 1_073_741_824,
                value: expected
            }
        );
    }
    let mut over = plan(
        "<nobr><shape type='space' param='1073741824px'><shape type='space' param='1073741824px'></nobr>",
    );
    measure_all(&mut over, 5000);
    assert_eq!(
        over.poll().unwrap_err().kind,
        HtmlQueryErrorKind::ResourceLimit
    );
}

#[test]
fn shape_residual_survives_text_and_button_boundaries_without_clamping() {
    let config = HtmlStringLengthSettings {
        font_size_pixels: 21,
        ..settings()
    };
    assert_eq!(
        pixels(
            "<nobr><shape type='space' param='50'>a<button value='1'><shape type='space' param='50'></button></nobr>",
            config
        ),
        26
    );
    let mut plan = HtmlStringLengthPlan::new(
        "<shape type='space' param='-3px'>",
        settings(),
        0,
        HtmlQueryLimits::default(),
    )
    .unwrap();
    measure_all(&mut plan, 5000);
    assert_eq!(
        plan.finish().unwrap(),
        HtmlStringLengthResult {
            first_line_pixels: -3,
            value: -1
        }
    );
    assert_eq!(
        pixels("<shape type='rect' param='2px,0px,3px,1px'>", settings()),
        5
    );
}

#[test]
fn loaded_image_replaces_residual_and_keeps_signed_fraction_when_flipped() {
    assert_eq!(
        pixels(
            "<nobr><img src='known' width='-33'><shape type='space' param='3'></nobr>",
            settings()
        ),
        6
    );
    let mut plan = plan("<nobr><img src='known'><shape type='space' param='50'></nobr>");
    for probe in plan.probes().to_vec() {
        let value = if matches!(probe.kind, HtmlLengthProbeKind::ImageSlot { .. }) {
            HtmlLengthMeasuredValue::ImageSlot(HtmlLengthImageResolution::Loaded {
                natural_width: 1,
                natural_height: 3,
            })
        } else {
            synthetic(&probe, 5000)
        };
        plan.resume(HtmlLengthMeasurement {
            probe_id: probe.id,
            value,
        })
        .unwrap();
    }
    assert_eq!(plan.finish().unwrap().first_line_pixels, 16);
}

#[test]
fn missing_image_alttext_is_core_generated_default_font_and_indivisible() {
    let mut plan = plan("<b><img src='missing' height='150' width='2px'></b>");
    let HtmlLengthProbeKind::ImageSlot { missing_document } = &plan.probes()[0].kind else {
        panic!("image probe");
    };
    assert_eq!(
        text(missing_document),
        "<img src='missing' height='30' width='2px'>"
    );
    assert!(matches!(missing_document.nodes[0], HtmlNode::Text { .. }));
    plan.resume(HtmlLengthMeasurement {
        probe_id: 0,
        value: HtmlLengthMeasuredValue::ImageSlot(HtmlLengthImageResolution::Missing {
            fallback_advance_millipixels: 31500,
        }),
    })
    .unwrap();
    assert_eq!(plan.finish().unwrap().first_line_pixels, 31);
}

#[test]
fn invalid_shapes_keep_reference_alttext_instead_of_zero_slot() {
    let mut plan = plan("<shape type='polygon' param='1,2px' color='#112233'>");
    assert!(matches!(
        plan.probes()[0].kind,
        HtmlLengthProbeKind::FallbackText
    ));
    assert_eq!(
        text(&plan.probes()[0].document),
        "<shape type='polygon' param='1, 2px' color='#112233'>"
    );
    plan.resume(HtmlLengthMeasurement {
        probe_id: 0,
        value: HtmlLengthMeasuredValue::FallbackText {
            advance_millipixels: 30000,
        },
    })
    .unwrap();
    assert_eq!(plan.finish().unwrap().first_line_pixels, 30);
}

#[test]
fn division_children_are_measured_but_visual_box_width_is_not_outer_advance() {
    let mut plan = plan("<div width='100px' height='20px'>abc</div>X");
    assert_eq!(plan.probes().len(), 3);
    assert!(matches!(
        plan.probes()[1].kind,
        HtmlLengthProbeKind::FixedSlot
    ));
    let outer = plan.probes()[2].clone();
    plan.resume(HtmlLengthMeasurement {
        probe_id: outer.id,
        value: synthetic(&outer, 5000),
    })
    .unwrap();
    assert!(plan.finish().is_err());
    let probes = plan.probes()[..2].to_vec();
    for probe in probes {
        plan.resume(HtmlLengthMeasurement {
            probe_id: probe.id,
            value: synthetic(&probe, 5000),
        })
        .unwrap();
    }
    assert_eq!(plan.finish().unwrap().first_line_pixels, 5);
}

#[test]
fn source_cuts_preserve_entities_and_full_non_bmp_scalars() {
    let mut plan = plan("é&amp;😀");
    let HtmlLengthProbeKind::TextPart { cuts, .. } = &plan.probes()[0].kind else {
        panic!("text probe");
    };
    assert_eq!(
        cuts.iter()
            .map(|cut| (cut.decoded_utf8, cut.decoded_utf16, cut.source_byte))
            .collect::<Vec<_>>(),
        [
            (0, 0, Some(0)),
            (2, 1, Some(2)),
            (3, 2, Some(7)),
            (7, 4, Some(11))
        ]
    );
    measure_all(&mut plan, 5000);
    assert_eq!(plan.finish().unwrap().first_line_pixels, 15);
    assert_eq!(
        pixels(
            "😀b",
            HtmlStringLengthSettings {
                drawable_width_pixels: 5,
                ..settings()
            }
        ),
        5
    );
}

#[test]
fn literal_newline_breaks_but_decoded_entity_lf_stays_in_text_part() {
    let mut plan = plan("A\nB&#10;C");
    assert_eq!(
        plan.probes()
            .iter()
            .map(|probe| text(&probe.document))
            .collect::<Vec<_>>(),
        ["A", "B\nC"]
    );
    measure_all(&mut plan, 5000);
    assert_eq!(plan.finish().unwrap().first_line_pixels, 5);
    for source in ["", "<b></b>", "<br>", "\n\n"] {
        assert_eq!(pixels(source, settings()), 0);
    }
}

#[test]
fn nested_canonical_queries_do_not_reparse_or_forge_source_lexemes() {
    let document = HtmlDocument {
        nodes: vec![HtmlNode::Text {
            text: "<&😀".into(),
            start: 100,
            end: 200,
        }],
    };
    let mut plan =
        HtmlStringLengthPlan::from_document(document, settings(), 1, HtmlQueryLimits::default())
            .unwrap();
    assert!(plan.mapped_document().is_none());
    assert_eq!(text(&plan.probes()[0].document), "<&😀");
    let HtmlLengthProbeKind::TextPart { cuts, .. } = &plan.probes()[0].kind else {
        panic!("text");
    };
    assert!(cuts.iter().all(|cut| cut.source_byte.is_none()));
    measure_all(&mut plan, 5000);
    assert_eq!(plan.finish().unwrap().value, 15);
}

#[test]
fn malformed_reply_variants_cuts_and_resource_limits_never_complete() {
    let mut plan = plan("ab");
    for widths in [vec![0, 5000], vec![0, -1, 10000], vec![1, 5000, 10000]] {
        assert!(
            plan.resume(HtmlLengthMeasurement {
                probe_id: 0,
                value: HtmlLengthMeasuredValue::TextPart {
                    prefix_advances_millipixels: widths
                }
            })
            .is_err()
        );
    }
    assert!(plan.finish().is_err());
    for limits in [
        HtmlQueryLimits {
            maximum_measurements: 1,
            ..HtmlQueryLimits::default()
        },
        HtmlQueryLimits {
            maximum_work_bytes: 1,
            ..HtmlQueryLimits::default()
        },
    ] {
        assert_eq!(
            HtmlStringLengthPlan::new("ab", settings(), 1, limits)
                .unwrap_err()
                .kind,
            HtmlQueryErrorKind::ResourceLimit
        );
    }
}

#[test]
fn text_probes_keep_font_style_and_nested_font_inheritance() {
    let plan = plan("<font face='one'><b><font color='#123456'>x</font></b></font>");
    let mut nodes = &plan.probes()[0].document.nodes;
    let mut font_seen = false;
    while let Some(HtmlNode::Element {
        semantic, children, ..
    }) = nodes.first()
    {
        if let HtmlElementSemantic::Font { face, color, .. } = semantic {
            assert_eq!(face.as_deref(), Some("one"));
            assert_eq!(*color, Some(0x0012_3456));
            font_seen = true;
        }
        nodes = children;
    }
    assert!(font_seen);
}

#[test]
fn later_rows_request_independently_shaped_suffixes_before_returning_first_width() {
    let mut plan = plan("abcdef");
    measure_all(&mut plan, 5000);
    let HtmlStringLengthPoll::NeedMeasurements { probe_ids } = plan.poll().unwrap() else {
        panic!("suffix required");
    };
    assert_eq!(probe_ids.len(), 1);
    let suffix = plan.probes()[usize::try_from(probe_ids[0]).unwrap()].clone();
    assert_eq!(text(&suffix.document), "ef");
    let usage = plan.usage();
    assert_eq!(
        plan.poll().unwrap(),
        HtmlStringLengthPoll::NeedMeasurements {
            probe_ids: probe_ids.clone()
        }
    );
    assert_eq!(plan.usage(), usage, "waiting does not replay pure layout");
    assert_eq!(plan.probes().len(), 2);
    assert!(plan.completed.is_none());
    // This independently shaped suffix is wider than original(full)-original(prefix).
    plan.resume(HtmlLengthMeasurement {
        probe_id: suffix.id,
        value: synthetic(&suffix, 15000),
    })
    .unwrap();
    let HtmlStringLengthPoll::NeedMeasurements { probe_ids } = plan.poll().unwrap() else {
        panic!("second suffix required");
    };
    let suffix = plan.probes()[usize::try_from(probe_ids[0]).unwrap()].clone();
    assert_eq!(text(&suffix.document), "f");
    plan.resume(HtmlLengthMeasurement {
        probe_id: suffix.id,
        value: synthetic(&suffix, 5000),
    })
    .unwrap();
    assert_eq!(
        plan.poll().unwrap(),
        HtmlStringLengthPoll::Complete(HtmlStringLengthResult {
            first_line_pixels: 20,
            value: 20
        })
    );
    let work = plan.work_bytes;
    assert!(matches!(
        plan.poll().unwrap(),
        HtmlStringLengthPoll::Complete(_)
    ));
    assert_eq!(plan.work_bytes, work);
}

#[test]
fn invalid_later_suffix_measurement_cannot_publish_an_already_known_first_row() {
    let mut plan = plan("abcdef");
    measure_all(&mut plan, 5000);
    let HtmlStringLengthPoll::NeedMeasurements { probe_ids } = plan.poll().unwrap() else {
        panic!("suffix");
    };
    let id = probe_ids[0];
    assert!(
        plan.resume(HtmlLengthMeasurement {
            probe_id: id,
            value: HtmlLengthMeasuredValue::TextPart {
                prefix_advances_millipixels: vec![0, -1, 10000]
            }
        })
        .is_err()
    );
    assert_eq!(
        plan.poll().unwrap(),
        HtmlStringLengthPoll::NeedMeasurements { probe_ids }
    );
    assert!(plan.completed.is_none());
}

#[test]
fn later_indivisible_alttext_divide_error_invalidates_the_entire_query() {
    for source in [
        "ok<br><img src='missing'>abc",
        "abcdef<nonbutton><img src='missing'>abc</nonbutton>",
        "ok<br><shape type='polygon' param='1'>abc",
        "<div width='20px' height='20px'>ok<br><img src='missing'>abc</div>X",
    ] {
        let mut plan = plan(source);
        let mut saw_suffix = false;
        let error = loop {
            match plan.poll() {
                Ok(HtmlStringLengthPoll::Complete(_)) => {
                    panic!("later reference DivideAt error was skipped")
                }
                Ok(HtmlStringLengthPoll::NeedMeasurements { probe_ids }) => {
                    for id in probe_ids {
                        let probe = plan.probes()[usize::try_from(id).unwrap()].clone();
                        if text(&probe.document) == "ef" {
                            saw_suffix = true;
                        }
                        let value = if matches!(probe.kind, HtmlLengthProbeKind::ImageSlot { .. }) {
                            HtmlLengthMeasuredValue::ImageSlot(HtmlLengthImageResolution::Missing {
                                fallback_advance_millipixels: 30000,
                            })
                        } else {
                            synthetic(&probe, 5000)
                        };
                        plan.resume(HtmlLengthMeasurement {
                            probe_id: id,
                            value,
                        })
                        .unwrap();
                    }
                }
                Err(error) => break error,
            }
        };
        assert_eq!(error.kind, HtmlQueryErrorKind::InvalidMarkup);
        assert!(error.message.contains("indivisible"));
        assert_eq!(saw_suffix, source.starts_with("abcdef"));
        assert_eq!(
            plan.poll().unwrap_err(),
            error,
            "terminal layout error stays terminal"
        );
        assert!(plan.completed.is_none());
    }
}

#[test]
fn suffixes_keep_styles_entity_provenance_and_complete_surrogate_boundaries() {
    let mut plan = HtmlStringLengthPlan::new(
        "<b>é&amp;😀Z</b>",
        HtmlStringLengthSettings {
            drawable_width_pixels: 10,
            ..settings()
        },
        1,
        HtmlQueryLimits::default(),
    )
    .unwrap();
    measure_all(&mut plan, 5000);
    let HtmlStringLengthPoll::NeedMeasurements { probe_ids } = plan.poll().unwrap() else {
        panic!("suffix");
    };
    let suffix = &plan.probes()[usize::try_from(probe_ids[0]).unwrap()];
    assert_eq!(text(&suffix.document), "😀Z");
    assert!(matches!(
        &suffix.document.nodes[0],
        HtmlNode::Element {
            kind: super::super::super::HtmlElementKind::Bold,
            ..
        }
    ));
    let HtmlLengthProbeKind::TextPart { cuts, .. } = &suffix.kind else {
        panic!("text");
    };
    assert_eq!(
        cuts.iter()
            .map(|cut| (cut.decoded_utf8, cut.decoded_utf16, cut.source_byte))
            .collect::<Vec<_>>(),
        [(0, 0, Some(10)), (4, 2, Some(14)), (5, 3, Some(15))]
    );
    assert_eq!(drive(&mut plan, 5000).first_line_pixels, 10);

    let document = HtmlDocument {
        nodes: vec![HtmlNode::Text {
            text: "<&😀".into(),
            start: 100,
            end: 200,
        }],
    };
    let mut canonical = HtmlStringLengthPlan::from_document(
        document,
        HtmlStringLengthSettings {
            drawable_width_pixels: 5,
            ..settings()
        },
        1,
        HtmlQueryLimits::default(),
    )
    .unwrap();
    assert_eq!(drive(&mut canonical, 5000).value, 5);
    assert_eq!(
        canonical
            .probes()
            .iter()
            .map(|probe| text(&probe.document))
            .collect::<Vec<_>>(),
        ["<&😀", "&😀", "😀"]
    );
    for probe in canonical.probes() {
        let HtmlLengthProbeKind::TextPart { cuts, .. } = &probe.kind else {
            panic!("text");
        };
        assert!(cuts.iter().all(|cut| cut.source_byte.is_none()));
    }
}

#[test]
fn boundary_splits_reuse_parts_and_full_end_null_keeps_its_extra_break() {
    let mut plan = HtmlStringLengthPlan::new(
        "ab<b>cd</b>",
        HtmlStringLengthSettings {
            drawable_width_pixels: 10,
            ..settings()
        },
        1,
        HtmlQueryLimits::default(),
    )
    .unwrap();
    assert_eq!(drive(&mut plan, 5000).first_line_pixels, 10);
    assert_eq!(
        plan.probes().len(),
        2,
        "a part boundary needs no suffix shaping"
    );
    let mut end = HtmlStringLengthPlan::new(
        "<img src='known' width='30px'>ab",
        settings(),
        1,
        HtmlQueryLimits {
            maximum_lines: 1,
            ..HtmlQueryLimits::default()
        },
    )
    .unwrap();
    measure_all(&mut end, 5000);
    assert_eq!(
        end.poll().unwrap_err().kind,
        HtmlQueryErrorKind::ResourceLimit,
        "reference's full-end DivideAt null adds an empty second row"
    );
}

#[test]
fn continuation_keeps_global_measurement_line_and_work_budgets() {
    let mut measurements = HtmlStringLengthPlan::new(
        "abcdef",
        settings(),
        1,
        HtmlQueryLimits {
            maximum_measurements: 8,
            ..HtmlQueryLimits::default()
        },
    )
    .unwrap();
    measure_all(&mut measurements, 5000);
    assert_eq!(
        measurements.poll().unwrap_err().kind,
        HtmlQueryErrorKind::ResourceLimit
    );
    assert_eq!(
        measurements.probes().len(),
        1,
        "failed reservation does not append a suffix"
    );

    let mut work = plan("abcdef");
    measure_all(&mut work, 5000);
    work.limits.maximum_work_bytes = work.work_bytes + 2048;
    assert_eq!(
        work.poll().unwrap_err().kind,
        HtmlQueryErrorKind::ResourceLimit
    );
    assert!(work.completed.is_none());

    let mut lines = HtmlStringLengthPlan::new(
        "abcdefghi",
        settings(),
        1,
        HtmlQueryLimits {
            maximum_lines: 2,
            ..HtmlQueryLimits::default()
        },
    )
    .unwrap();
    loop {
        match lines.poll() {
            Ok(HtmlStringLengthPoll::NeedMeasurements { probe_ids }) => {
                for id in probe_ids {
                    let probe = lines.probes()[usize::try_from(id).unwrap()].clone();
                    lines
                        .resume(HtmlLengthMeasurement {
                            probe_id: id,
                            value: synthetic(&probe, 5000),
                        })
                        .unwrap();
                }
            }
            Ok(HtmlStringLengthPoll::Complete(_)) => panic!("third row was skipped"),
            Err(error) => {
                assert_eq!(error.kind, HtmlQueryErrorKind::ResourceLimit);
                break;
            }
        }
    }
}
