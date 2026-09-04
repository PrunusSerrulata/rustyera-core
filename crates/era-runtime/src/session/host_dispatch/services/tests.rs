#[cfg(test)]
mod immediate_tests {
    use super::{
        CellAlignment, ImmediateTagSplitTargets, ImmediateTextFormatting, LineAlignment,
        PreparedButton, PreparedHtmlPrint, PreparedPresentationState, RuntimeQueryEvaluation,
        RuntimeQueryState, evaluate_runtime_query, immediate_host_path_memo_safe,
        immediate_html_tag_split, immediate_text_value, is_immediate_committed_text_print,
        is_immediate_text_print, skips_runtime_command_immediately,
    };
    use crate::presentation::PresentationModel;
    use era_runtime_protocol::ProtocolValue;
    use erabasic_vm::{PlaceDescriptor, VmValue};

    fn tag_split_targets(capacity: usize) -> ImmediateTagSplitTargets {
        let place = |index| PlaceDescriptor {
            indices: vec![index],
            ..PlaceDescriptor::default()
        };
        ImmediateTagSplitTargets {
            result: place(0),
            results: place(0),
            results_capacity: capacity,
        }
    }

    #[test]
    fn immediate_tag_split_preserves_default_target_write_semantics() {
        let targets = tag_split_targets(2);
        let ready =
            immediate_html_tag_split(&[VmValue::String("a<b>x</b>".into())], Some(&targets))
                .unwrap();
        assert_eq!(ready.writes.len(), 3);
        assert_eq!(ready.writes[0].value, VmValue::String("a".into()));
        assert_eq!(ready.writes[1].value, VmValue::String("<b>".into()));
        assert_eq!(ready.writes[2].value, VmValue::Integer(4));

        let empty = immediate_html_tag_split(
            &[VmValue::String(String::new())],
            Some(&tag_split_targets(2)),
        )
        .unwrap();
        assert_eq!(empty.writes.len(), 1);
        assert_eq!(empty.writes[0].value, VmValue::Integer(0));

        let malformed = immediate_html_tag_split(
            &[VmValue::String("a<b".into())],
            Some(&tag_split_targets(2)),
        )
        .unwrap();
        assert_eq!(malformed.writes.len(), 1);
        assert_eq!(malformed.writes[0].value, VmValue::Integer(-1));
    }

    #[test]
    fn immediate_tag_split_rejects_nondefault_or_mistyped_calls() {
        let targets = tag_split_targets(2);
        assert!(immediate_html_tag_split(&[VmValue::Integer(1)], Some(&targets)).is_none());
        assert!(
            immediate_html_tag_split(
                &[
                    VmValue::String("a".into()),
                    VmValue::StringPlace(Box::default()),
                ],
                Some(&targets),
            )
            .is_none()
        );
        assert!(
            immediate_html_tag_split(
                &[
                    VmValue::String("a".into()),
                    VmValue::StringPlace(Box::default()),
                    VmValue::IntegerPlace(Box::default()),
                ],
                Some(&targets),
            )
            .is_none()
        );
        assert!(immediate_html_tag_split(&[VmValue::String("a".into())], None).is_none());
    }

    #[test]
    fn html_print_preparation_distinguishes_clean_warning_and_error_inputs() {
        let clean = PreparedHtmlPrint::prepare(&[VmValue::String(
            "<p align='left'><nobr>clean</nobr></p>".into(),
        )])
        .unwrap();
        assert!(clean.warnings.is_empty());
        assert!(!clean.inline);

        let inline = PreparedHtmlPrint::prepare(&[
            VmValue::String("<nobr>inline</nobr>".into()),
            VmValue::Integer(1),
        ])
        .unwrap();
        assert!(inline.warnings.is_empty());
        assert!(inline.inline);

        let warning = PreparedHtmlPrint::prepare(&[VmValue::String(
            "<font color='#fff'><button value='1'>crossed</font></button>".into(),
        )])
        .unwrap();
        assert!(!warning.warnings.is_empty());
        assert!(PreparedHtmlPrint::prepare(&[VmValue::String("<unknown>".into())]).is_err());
    }

    #[test]
    fn layout_queries_are_never_classified_as_immediate_prints() {
        assert!(!is_immediate_text_print("PRINTCPERLINE"));
        assert!(!is_immediate_text_print("PRINTCLENGTH"));
        assert!(is_immediate_committed_text_print("PRINTL"));
        assert!(is_immediate_committed_text_print("PRINTFORMKL"));
        assert!(!is_immediate_committed_text_print("PRINTW"));
        assert!(!is_immediate_committed_text_print("PRINTFORMC"));
    }

    #[test]
    fn common_button_and_style_commands_are_safe_for_the_immediate_lane() {
        let mut presentation = PresentationModel::default();
        PreparedPresentationState::prepare(
            "SETCOLOR",
            &[
                VmValue::Integer(0x12),
                VmValue::Integer(0x34),
                VmValue::Integer(0x56),
            ],
        )
        .unwrap()
        .unwrap()
        .apply(&mut presentation);
        assert_eq!(presentation.foreground_rgb(), 0x12_34_56);
        PreparedPresentationState::prepare("ALIGNMENT", &[VmValue::String("center".into())])
            .unwrap()
            .unwrap()
            .apply(&mut presentation);
        assert_eq!(presentation.alignment(), LineAlignment::Center);
        assert!(
            PreparedPresentationState::prepare(
                "SETCOLOR",
                &[
                    VmValue::Integer(300),
                    VmValue::Integer(0),
                    VmValue::Integer(0)
                ],
            )
            .is_err()
        );

        let button = PreparedButton::prepare(
            "PRINTBUTTONC",
            &[VmValue::String("A\nB".into()), VmValue::Integer(42)],
        )
        .unwrap();
        assert_eq!(button.text, "AB");
        assert_eq!(button.value, VmValue::Integer(42));
        assert_eq!(button.protocol_value, ProtocolValue::Integer(42));
        assert_eq!(button.alignment, Some(CellAlignment::Right));
        assert!(
            PreparedButton::prepare(
                "PRINTBUTTON",
                &[
                    VmValue::String("bad".into()),
                    VmValue::IntegerPlace(Box::default()),
                ],
            )
            .is_err()
        );
    }

    #[test]
    fn skipped_runtime_commands_use_the_immediate_lane_without_hiding_input_errors() {
        for name in ["PRINTL", "HTML_PRINT", "DRAWLINE", "WAITANYKEY", "INPUT"] {
            assert!(skips_runtime_command_immediately(
                "rustyera.text",
                name,
                true,
                false,
            ));
        }
        assert!(!skips_runtime_command_immediately(
            "rustyera.text",
            "INPUT",
            true,
            true,
        ));
        assert!(!skips_runtime_command_immediately(
            "rustyera.extension",
            "PRINTL",
            true,
            false,
        ));
        assert!(!skips_runtime_command_immediately(
            "rustyera.text",
            "GETCOLOR",
            true,
            false,
        ));
        assert!(!skips_runtime_command_immediately(
            "rustyera.text",
            "PRINTL",
            false,
            false,
        ));
    }

    #[test]
    fn pure_text_values_only_use_the_immediate_lane_when_the_slow_path_would_succeed() {
        let formatting = Some(ImmediateTextFormatting {
            bar_char_1: '*',
            bar_char_2: '.',
            money_first: true,
            money_label: "$",
        });
        assert_eq!(
            immediate_text_value(
                "TOSTR",
                &[VmValue::Integer(-12), VmValue::String("+#0;-#0".into())],
                formatting,
            ),
            Some(VmValue::String("-12".into()))
        );
        assert_eq!(
            immediate_text_value(
                "BARSTR",
                &[
                    VmValue::Integer(1),
                    VmValue::Integer(2),
                    VmValue::Integer(3),
                ],
                formatting,
            ),
            Some(VmValue::String("[*..]".into()))
        );
        assert_eq!(
            immediate_text_value(
                "MONEYSTR",
                &[VmValue::Integer(7), VmValue::String("0".into())],
                formatting,
            ),
            Some(VmValue::String("$7".into()))
        );
        assert!(
            immediate_text_value(
                "TOSTR",
                &[VmValue::Integer(1), VmValue::String("invalid[".into())],
                formatting,
            )
            .is_none()
        );
        assert!(
            immediate_text_value(
                "BARSTR",
                &[
                    VmValue::Integer(1),
                    VmValue::Integer(0),
                    VmValue::Integer(3),
                ],
                formatting,
            )
            .is_none()
        );
        assert!(
            immediate_text_value(
                "BARSTR",
                &[
                    VmValue::Integer(1),
                    VmValue::Integer(1),
                    VmValue::Integer(100),
                ],
                formatting,
            )
            .is_none()
        );
        assert!(
            immediate_text_value("TOSTR", &[VmValue::String("1".into())], formatting).is_none()
        );
        assert!(immediate_text_value("MONEYSTR", &[VmValue::Integer(7)], None).is_none());
    }

    #[test]
    fn path_memo_only_crosses_argument_pure_immediate_text_hosts() {
        for name in ["HTML_ESCAPE", "html_tagsplit", "HTML_TOPLAINTEXT", "tostr"] {
            assert!(immediate_host_path_memo_safe("rustyera.text", name));
        }
        for name in ["GETBGCOLOR", "GETCOLOR", "GETFONT", "MONEYSTR"] {
            assert!(!immediate_host_path_memo_safe("rustyera.text", name));
        }
        assert!(!immediate_host_path_memo_safe(
            "rustyera.extension",
            "TOSTR",
        ));
    }

    #[test]
    fn runtime_query_evaluator_covers_every_immediate_query() {
        let presentation = PresentationModel::default();
        let state = RuntimeQueryState {
            skip_print: false,
            message_skip: true,
            snake_display_state: false,
        };
        let cases = [
            (
                "HTML_ESCAPE",
                vec![VmValue::String("<&".into())],
                VmValue::String("&lt;&amp;".into()),
            ),
            (
                "HTML_TOPLAINTEXT",
                vec![VmValue::String("a&nbsp;b".into())],
                VmValue::String("a b".into()),
            ),
            ("CURRENTALIGN", vec![], VmValue::String("LEFT".into())),
            ("GETFONT", vec![], VmValue::String(presentation.font())),
            (
                "CURRENTREDRAW",
                vec![],
                VmValue::Integer(i64::from(presentation.redraw_enabled())),
            ),
            (
                "GETBGCOLOR",
                vec![],
                VmValue::Integer(presentation.background_rgb()),
            ),
            (
                "GETCOLOR",
                vec![],
                VmValue::Integer(presentation.foreground_rgb()),
            ),
            (
                "GETDEFBGCOLOR",
                vec![],
                VmValue::Integer(presentation.default_background_rgb()),
            ),
            (
                "GETDEFCOLOR",
                vec![],
                VmValue::Integer(presentation.default_foreground_rgb()),
            ),
            (
                "GETFOCUSCOLOR",
                vec![],
                VmValue::Integer(presentation.focus_rgb()),
            ),
            (
                "GETSTYLE",
                vec![],
                VmValue::Integer(presentation.style_bits()),
            ),
            ("ISSKIP", vec![], VmValue::Integer(0)),
            ("MESSKIP", vec![], VmValue::Integer(1)),
            ("MOUSESKIP", vec![], VmValue::Integer(1)),
            (
                "LINEISEMPTY",
                vec![],
                VmValue::Integer(i64::from(presentation.last_line_is_empty())),
            ),
        ];
        for (name, arguments, expected) in cases {
            assert_eq!(
                evaluate_runtime_query(name, &arguments, &presentation, state).unwrap(),
                RuntimeQueryEvaluation::Ready(expected),
                "{name}"
            );
        }
        assert_eq!(
            evaluate_runtime_query(
                "HTML_TOPLAINTEXT",
                &[VmValue::String("&#xD800;".into())],
                &presentation,
                state,
            )
            .unwrap(),
            RuntimeQueryEvaluation::MalformedHtml
        );
        assert_eq!(
            evaluate_runtime_query("UNKNOWN", &[], &presentation, state).unwrap(),
            RuntimeQueryEvaluation::Unhandled
        );
        assert!(
            evaluate_runtime_query(
                "HTML_TOPLAINTEXT",
                &[VmValue::Integer(1)],
                &presentation,
                state,
            )
            .is_err()
        );
    }

    #[test]
    fn runtime_query_evaluator_classifies_negative_printed_html_indexes() {
        assert_eq!(
            evaluate_runtime_query(
                "HTML_GETPRINTEDSTR",
                &[VmValue::Integer(-1)],
                &PresentationModel::default(),
                RuntimeQueryState {
                    skip_print: false,
                    message_skip: false,
                    snake_display_state: false,
                },
            )
            .unwrap(),
            RuntimeQueryEvaluation::InvalidPrintedHtmlIndex
        );
    }
}
