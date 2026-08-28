use super::*;

fn text(nodes: &[HtmlNode]) -> String {
    nodes
        .iter()
        .map(|node| match node {
            HtmlNode::Text { text, .. } => text.clone(),
            HtmlNode::Element { children, .. } => text(children),
        })
        .collect()
}

fn substring(source: &str, budget: i64) -> Result<HtmlSubstringResult, HtmlQueryError> {
    let mut plan = HtmlSubstringPlan::new(source, budget, HtmlQueryLimits::default())?;
    for _ in 0..256 {
        match plan.poll()? {
            HtmlSubstringPoll::Complete(result) => return Ok(result),
            HtmlSubstringPoll::NeedMeasure(probe) => plan.resume(
                probe.id,
                if probe.kind == HtmlQueryProbeKind::Atomic {
                    5
                } else {
                    1
                },
            )?,
        }
    }
    panic!("fixture exceeded bounded planner transitions")
}

fn lines(source: &str, budget: i64, limits: HtmlQueryLimits) -> Result<u64, HtmlQueryError> {
    let mut plan = HtmlStringLinesPlan::new(source, budget, limits)?;
    for _ in 0..256 {
        match plan.poll()? {
            HtmlLinesPoll::Complete(count) => return Ok(count),
            HtmlLinesPoll::NeedMeasure(probe) => plan.resume(probe.id, 1)?,
        }
    }
    panic!("fixture exceeded bounded line transitions")
}

#[test]
fn source_boundaries_preserve_entities_and_unicode_coordinate_systems() {
    let mapped = parse_document_with_source_map(
        "A&amp;中😀",
        HtmlQueryEntityPolicy::Existing,
        HtmlQueryLimits::default(),
    )
    .unwrap();
    let boundaries = &mapped.texts[0].boundaries;
    assert_eq!(
        boundaries
            .iter()
            .map(|b| b.decoded_utf8)
            .collect::<Vec<_>>(),
        [0, 1, 2, 5, 9]
    );
    assert_eq!(
        boundaries
            .iter()
            .map(|b| b.decoded_utf16)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 5]
    );
    assert_eq!(
        boundaries.iter().map(|b| b.source_byte).collect::<Vec<_>>(),
        [0, 1, 6, 9, 13]
    );
    assert_eq!(mapped.source_cut(&[0], 2, 2), Some(6));
    assert_eq!(mapped.source_cut(&[0], 7, 4), None);
    assert_eq!(mapped.source_cut(&[1], 0, 0), None);
}

#[test]
fn query_entities_merge_surrogate_pairs_without_changing_the_normal_parser() {
    let limits = HtmlQueryLimits::default();
    let source = "&NBSP;&#55357;&#56832;";
    let decoded =
        decode_query_entities(source, HtmlQueryEntityPolicy::ReferenceQuery, limits).unwrap();
    assert_eq!(decoded.text, " 😀");
    assert_eq!(decoded.source_byte_for_utf16(1), Some(6));
    assert_eq!(decoded.source_byte_for_utf16(2), None);
    assert_eq!(decoded.source_byte_for_utf16(3), Some(source.len()));
    assert!(super::super::parse_document(source).is_err());
    let mapped =
        parse_document_with_source_map(source, HtmlQueryEntityPolicy::ReferenceQuery, limits)
            .unwrap();
    assert_eq!(text(&mapped.document.nodes), " 😀");
    for source in [
        "&#55357;",
        "&#56832;",
        "&#55357;A",
        "&#128512;",
        "&unknown;",
    ] {
        assert!(
            decode_query_entities(source, HtmlQueryEntityPolicy::ReferenceQuery, limits).is_err(),
            "{source}"
        );
    }
}

#[test]
fn query_entity_policy_applies_to_attributes_without_rewriting_raw_source() {
    let source = "<font face='A&AMP;B'>x</font>";
    assert!(super::super::parse_document(source).is_err());
    let mapped = parse_document_with_source_map(
        source,
        HtmlQueryEntityPolicy::ReferenceQuery,
        HtmlQueryLimits::default(),
    )
    .unwrap();
    let HtmlNode::Element { attributes, .. } = &mapped.document.nodes[0] else {
        panic!("font node expected");
    };
    assert_eq!(attributes[0].value, "A&B");
    assert_eq!(
        &source[mapped.events[0].range.start..mapped.events[0].range.end],
        "<font face='A&AMP;B'>"
    );
}

#[test]
fn events_preserve_comments_quote_aware_tags_implicit_closes_and_reparented_text() {
    let source = "<!--keep--><font face='A>B'><b><i>x</b>y</i></font>";
    let mapped = parse_document_with_source_map(
        source,
        HtmlQueryEntityPolicy::Existing,
        HtmlQueryLimits::default(),
    )
    .unwrap();
    assert_eq!(mapped.events[0].kind, HtmlSourceEventKind::Comment);
    assert_eq!(
        &source[mapped.events[1].range.start..mapped.events[1].range.end],
        "<font face='A>B'>"
    );
    for mapped_text in &mapped.texts {
        assert_eq!(
            mapped.source_cut(&mapped_text.node_path, 0, 0),
            Some(mapped_text.range.start)
        );
        assert_eq!(
            mapped.source_cut(&mapped_text.node_path, 1, 1),
            Some(mapped_text.range.end)
        );
    }
    let mapped = parse_document_with_source_map(
        "<p align='left'><nobr>x",
        HtmlQueryEntityPolicy::Existing,
        HtmlQueryLimits::default(),
    )
    .unwrap();
    assert_eq!(
        mapped
            .events
            .iter()
            .filter(|event| matches!(event.kind, HtmlSourceEventKind::ImplicitClose { .. }))
            .count(),
        2
    );
}

#[test]
fn substring_closes_and_reopens_exact_working_source_tags() {
    let source = "<font color = 'red'><b>AB</b></font>";
    let result = substring(source, 1).unwrap();
    assert_eq!(result.head, "<font color = 'red'><b>A</b></font>");
    assert_eq!(result.tail, "<font color = 'red'><b>B</b></font>");
    assert!(
        result
            .head_pieces
            .iter()
            .any(|piece| matches!(piece.origin, HtmlOutputOrigin::GeneratedClose { .. }))
    );
    assert!(
        result
            .tail_pieces
            .iter()
            .any(|piece| matches!(piece.origin, HtmlOutputOrigin::Reopened { .. }))
    );
    let result = substring("A&#66;", 1).unwrap();
    assert_eq!((result.head.as_str(), result.tail.as_str()), ("A", "B"));
}

#[test]
fn unmeasured_suffix_errors_stay_deferred_but_whole_unescape_errors_do_not() {
    let result = substring("<b>A&amp;</b>", 0).unwrap();
    assert_eq!(result.head, "<b></b>");
    assert_eq!(result.tail, "<b>A&</b>");
    assert_eq!(
        substring("<b>A&amp;</b>", 1).unwrap_err().kind,
        HtmlQueryErrorKind::InvalidEntity
    );
    assert!(HtmlSubstringPlan::new("A&unknown;", 0, HtmlQueryLimits::default()).is_err());
    assert_eq!(
        substring("A<unknown>x</unknown>", 0).unwrap().tail,
        "A<unknown>x</unknown>"
    );
    assert_eq!(
        substring("A<unknown>x</unknown>", 1).unwrap_err().kind,
        HtmlQueryErrorKind::UnsupportedTag
    );
}

#[test]
fn probes_are_independent_scalars_with_only_reference_style_scopes() {
    let mut plan = HtmlSubstringPlan::new(
        "<font face='other'><u><b>😀A</b></u></font>",
        1,
        HtmlQueryLimits::default(),
    )
    .unwrap();
    let HtmlSubstringPoll::NeedMeasure(first) = plan.poll().unwrap() else {
        panic!("scalar probe expected");
    };
    assert_eq!(text(&first.document.nodes), "😀");
    assert_eq!(first.source.end - first.source.start, 4);
    assert_eq!(
        super::super::serialize_document(&first.document),
        "<b>😀</b>"
    );
    assert_eq!(
        plan.poll().unwrap(),
        HtmlSubstringPoll::NeedMeasure(first.clone())
    );
    assert_eq!(
        plan.resume(first.id + 10, 1).unwrap_err().kind,
        HtmlQueryErrorKind::InvalidMeasurement
    );
    assert_eq!(
        plan.resume(first.id, -1).unwrap_err().kind,
        HtmlQueryErrorKind::InvalidMeasurement
    );
    plan.resume(first.id, 1).unwrap();
    let HtmlSubstringPoll::NeedMeasure(second) = plan.poll().unwrap() else {
        panic!("second scalar probe expected");
    };
    assert_ne!(first.id, second.id);
    assert_eq!(text(&second.document.nodes), "A");
    assert!(plan.resume(first.id, 1).is_err());
    plan.resume(second.id, 1).unwrap();
    let HtmlSubstringPoll::Complete(result) = plan.poll().unwrap() else {
        panic!("substring expected");
    };
    assert!(result.head.contains("😀</b>"));
    assert!(result.tail.contains("<b>A"));
}

#[test]
fn leading_images_do_not_set_the_reference_text_content_flag() {
    let images = "<img src='a'><img src='b'>";
    let result = substring(images, 0).unwrap();
    assert_eq!(result.head, images);
    assert_eq!(result.tail, "");
    let result = substring("A<img src='a'>", 1).unwrap();
    assert_eq!(result.head, "A");
    assert_eq!(result.tail, "<img src='a'>");
}

#[test]
fn lexical_substring_edges_preserve_reference_raw_behavior() {
    let result = substring("A<br>B", 100).unwrap();
    assert_eq!((result.head.as_str(), result.tail.as_str()), ("A", "B"));
    let result = substring("<br >x", 100).unwrap();
    assert_eq!((result.head.as_str(), result.tail.as_str()), ("", ">x"));
    let result = substring("A<", 100).unwrap();
    assert_eq!((result.head.as_str(), result.tail.as_str()), ("A", "<"));
    let result = substring("<BR>x", 100).unwrap();
    assert_eq!(
        (result.head.as_str(), result.tail.as_str()),
        ("<BR>x</BR>", "<BR>")
    );
    // This is lexical reference behavior, not a claim that the returned string is valid HTML.
    let result = substring("<!--x-->A", 100).unwrap();
    assert_eq!(
        (result.head.as_str(), result.tail.as_str()),
        ("<!--x-->A</!--x-->", "<!--x-->")
    );
}

#[test]
fn shape_semantics_are_validated_only_when_the_atomic_probe_is_reached() {
    let shape = "<shape type='rect' param='0,0,10,10'>";
    let result = substring(&format!("A{shape}"), 1).unwrap();
    assert_eq!(result.head, "A");
    assert_eq!(result.tail, shape);
    let malformed = "A<shape type='rect'>";
    assert_eq!(substring(malformed, 0).unwrap().tail, malformed);
    assert_eq!(
        substring(malformed, 1).unwrap_err().kind,
        HtmlQueryErrorKind::InvalidMarkup
    );
}

#[test]
fn lines_redecode_each_tail_count_breaks_and_fail_no_progress_immediately() {
    let limits = HtmlQueryLimits::default();
    assert_eq!(lines("", 1, limits).unwrap(), 0);
    assert_eq!(lines("A<br>", 1, limits).unwrap(), 1);
    assert_eq!(lines("<br><br>", 1, limits).unwrap(), 2);
    assert_eq!(lines("AB&#38;#67;", 1, limits).unwrap(), 3);
    assert_eq!(
        lines("A", 0, limits).unwrap_err().kind,
        HtmlQueryErrorKind::NoProgress
    );
    assert_eq!(
        lines("<font face='x'>", 1, limits).unwrap_err().kind,
        HtmlQueryErrorKind::NoProgress
    );
    let mut plan = HtmlStringLinesPlan::new("AB", 0, limits).unwrap();
    for _ in 0..2 {
        let HtmlLinesPoll::NeedMeasure(probe) = plan.poll().unwrap() else {
            panic!("zero-width scalar expected");
        };
        plan.resume(probe.id, 0).unwrap();
    }
    assert_eq!(plan.poll().unwrap(), HtmlLinesPoll::Complete(1));
}

#[test]
fn limits_bound_depth_outputs_measurements_lines_and_cumulative_redecoding() {
    let limits = HtmlQueryLimits {
        maximum_depth: 1,
        ..HtmlQueryLimits::default()
    };
    assert_eq!(
        parse_document_with_source_map("<b><i>x</i></b>", HtmlQueryEntityPolicy::Existing, limits)
            .unwrap_err()
            .kind,
        HtmlQueryErrorKind::ResourceLimit
    );
    let limits = HtmlQueryLimits {
        maximum_lines: 1,
        ..HtmlQueryLimits::default()
    };
    assert_eq!(
        lines("<br><br>", 1, limits).unwrap_err().kind,
        HtmlQueryErrorKind::ResourceLimit
    );
    let limits = HtmlQueryLimits {
        maximum_measurements: 1,
        ..HtmlQueryLimits::default()
    };
    assert_eq!(
        lines("AB", 10, limits).unwrap_err().kind,
        HtmlQueryErrorKind::ResourceLimit
    );
    let limits = HtmlQueryLimits {
        maximum_work_bytes: 6,
        ..HtmlQueryLimits::default()
    };
    assert_eq!(
        lines("ABCD", 1, limits).unwrap_err().kind,
        HtmlQueryErrorKind::ResourceLimit
    );
    let mut plan = HtmlSubstringPlan::new(
        "<font face='x'>A",
        0,
        HtmlQueryLimits {
            maximum_output_bytes: 3,
            ..HtmlQueryLimits::default()
        },
    )
    .unwrap();
    let HtmlSubstringPoll::NeedMeasure(probe) = plan.poll().unwrap() else {
        panic!("scalar expected");
    };
    assert_eq!(
        plan.resume(probe.id, 1).unwrap_err().kind,
        HtmlQueryErrorKind::ResourceLimit
    );
}

#[test]
fn query_resource_limits_and_measurement_contracts_never_gain_input_origin() {
    let limits = HtmlQueryLimits {
        maximum_source_bytes: 0,
        ..HtmlQueryLimits::default()
    };
    let resource = HtmlSubstringPlan::new("x", 1, limits).unwrap_err();
    assert_eq!(resource.kind, HtmlQueryErrorKind::ResourceLimit);
    assert_eq!(resource.origin(), HtmlQueryErrorOrigin::NonScript);
    let mut plan = HtmlSubstringPlan::new("x", 1, HtmlQueryLimits::default()).unwrap();
    let measurement = plan.resume(42, 1).unwrap_err();
    assert_eq!(measurement.kind, HtmlQueryErrorKind::InvalidMeasurement);
    assert_eq!(measurement.origin(), HtmlQueryErrorOrigin::NonScript);
}

#[test]
fn parser_entity_bridge_preserves_non_script_origin() {
    let non_script = HtmlQueryError::new(HtmlQueryErrorKind::ResourceLimit, 0, 1, "bounded input");
    let ordinary = HtmlError {
        kind: super::super::HtmlErrorKind::InvalidEntity,
        start: non_script.range.start,
        end: non_script.range.end,
        origin: non_script.origin(),
    };
    let bridged = HtmlQueryError::markup(&ordinary);
    assert_eq!(bridged.kind, HtmlQueryErrorKind::InvalidEntity);
    assert_eq!(bridged.origin(), HtmlQueryErrorOrigin::NonScript);
}
