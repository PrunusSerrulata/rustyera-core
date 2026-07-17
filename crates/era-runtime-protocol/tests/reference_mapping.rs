use era_runtime_protocol::{
    Color, DisplayLine, DisplayRun, InputWait, LineAlignment, RuntimePhase, TextStyle, WaitKind,
    WaitStability,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    reference_commit: String,
    load: LoadObservation,
    run: RunObservation,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoadObservation {
    state: String,
    termination: String,
    last_output: String,
    input_request: InputObservation,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
struct InputObservation {
    id: u64,
    input_type: String,
    one_input: bool,
    stop_messkip: bool,
    is_system_input: bool,
    mouse_input: bool,
    timelimit: i64,
    display_time: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunObservation {
    entry: String,
    input: String,
    termination: String,
    last_output: String,
    result: i64,
}

#[test]
fn pinned_reference_wait_and_output_are_losslessly_representable() {
    let observed: Observation =
        serde_json::from_str(include_str!("fixtures/reference-runtime-observation.json"))
            .expect("valid normalized reference observation");
    assert_eq!(
        observed.reference_commit,
        "26a35dc9334bb67590b96f7b8efbefbf199e391e"
    );
    assert_eq!(observed.load.state, "WaitInput");
    assert_eq!(observed.load.termination, "waitingInput");
    let phase = RuntimePhase::WaitingInput;
    assert_eq!(phase, RuntimePhase::WaitingInput);

    let input = InputWait {
        wait_id: observed.load.input_request.id,
        kind: match observed.load.input_request.input_type.as_str() {
            "IntValue" => WaitKind::IntegerValue,
            other => panic!("unexpected reference input type: {other}"),
        },
        stability: WaitStability::StableInput,
        one_input: observed.load.input_request.one_input,
        stop_message_skip: observed.load.input_request.stop_messkip,
        system_input: observed.load.input_request.is_system_input,
        mouse_input: observed.load.input_request.mouse_input,
        default_value: None,
        deadline_ns: (observed.load.input_request.timelimit > 0).then_some(0),
        display_time: observed.load.input_request.display_time,
        timeout_message: None,
        submission_token: era_runtime_protocol::InteractionToken { epoch: 1, id: 1 },
        countdown_remaining_ms: None,
    };
    assert_eq!(input.kind, WaitKind::IntegerValue);
    assert_eq!(input.deadline_ns, None);

    let line = DisplayLine {
        line_id: 1,
        temporary: false,
        logical_line_start: true,
        line_end: true,
        alignment: LineAlignment::Left,
        runs: vec![DisplayRun::Text {
            text: observed.load.last_output,
            system_text: None,
            style: TextStyle {
                foreground: Color {
                    red: 255,
                    green: 255,
                    blue: 255,
                    alpha: 255,
                },
                background: None,
                bold: false,
                italic: false,
                underline: false,
                strikeout: false,
                font_family: None,
                font_millipoints: 16_000,
            },
        }],
    };
    assert!(matches!(
        &line.runs[0],
        DisplayRun::Text { text, .. } if text == "ORACLE_READY"
    ));

    assert_eq!(observed.run.entry, "ORACLE_INPUT");
    assert_eq!(observed.run.input, "42");
    assert_eq!(observed.run.termination, "completed");
    assert_eq!(observed.run.last_output, "ORACLE_GOT=42");
    assert_eq!(observed.run.result, 42);
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WaitFixture {
    reference_commit: String,
    observations: Vec<WaitObservation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
struct WaitObservation {
    operation: String,
    input_type: String,
    default_int: i64,
    default_str: Option<String>,
    has_default: bool,
    one_input: bool,
    stop_messkip: bool,
    mouse_input: bool,
    timelimit: i64,
    display_time: bool,
    timeout_message: Option<String>,
    expected_stability: String,
}

#[test]
fn audited_reference_waits_have_the_expected_snapshot_stability() {
    let fixture: WaitFixture =
        serde_json::from_str(include_str!("fixtures/reference-input-waits.json"))
            .expect("valid normalized wait observations");
    assert_eq!(
        fixture.reference_commit,
        "26a35dc9334bb67590b96f7b8efbefbf199e391e"
    );
    for observed in fixture.observations {
        let kind = match observed.input_type.as_str() {
            "IntValue" => WaitKind::IntegerValue,
            "StrValue" => WaitKind::StringValue,
            "EnterKey" => WaitKind::EnterKey,
            "Void" => WaitKind::Void,
            other => panic!("unexpected input type {other}"),
        };
        let stability = WaitStability::for_reference_wait(kind, observed.timelimit);
        let expected = match observed.expected_stability.as_str() {
            "stable_input" => WaitStability::StableInput,
            "transient" => WaitStability::Transient,
            other => panic!("unexpected stability {other}"),
        };
        assert_eq!(stability, expected, "{}", observed.operation);
        assert_eq!(observed.one_input, observed.operation == "TONEINPUTS");
        assert_eq!(observed.stop_messkip, observed.operation == "FORCEWAIT");
        assert_eq!(observed.mouse_input, observed.default_int == 8);
        assert_eq!(
            observed.has_default,
            observed.operation == "TINPUT" || observed.operation == "TONEINPUTS"
        );
        assert_eq!(
            observed.display_time,
            observed.timelimit == 1000 && observed.has_default
        );
        assert_eq!(
            observed.default_str.is_some(),
            kind == WaitKind::StringValue
        );
        assert_eq!(
            observed.timeout_message.is_some(),
            observed.operation == "TINPUT" || observed.operation == "TONEINPUTS"
        );
    }
    assert_eq!(
        WaitStability::for_reference_wait(WaitKind::Void, 0),
        WaitStability::Transient,
        "a deadline-free Void wait is still not a stable user-input point"
    );
}
