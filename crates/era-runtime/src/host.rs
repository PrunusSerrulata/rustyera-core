use era_runtime_protocol::{InputWait, InteractionToken, ProtocolValue, WaitKind, WaitStability};
use erabasic_bytecode::BytecodeType;
use erabasic_vm::{HostRequestId, PlaceDescriptor, VmHostRequest, VmValue};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PendingInput {
    pub(crate) host_request: Option<HostRequestId>,
    pub(crate) wait: InputWait,
    pub(crate) result_name: Option<String>,
    #[serde(with = "crate::runtime_snapshot::token_value_map")]
    pub(crate) choices: std::collections::BTreeMap<InteractionToken, VmValue>,
    pub(crate) timeout_duration_ns: Option<u64>,
    pub(crate) post_input: Option<PostInputAction>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum PostInputAction {
    OpenUrl { url: String, trigger_value: i64 },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum ExternalCompletion {
    DevicePump {
        request: HostRequestId,
        epoch: u64,
        after_event_sequence: u64,
        milliseconds: u64,
    },
    GetKey {
        request: HostRequestId,
        key_code: u8,
        triggered: bool,
    },
    LocalDateTime {
        request: HostRequestId,
        operation: ClockOperation,
        result: Option<BytecodeType>,
    },
    SpritePixel {
        request: HostRequestId,
    },
    UpdateCheck {
        request: HostRequestId,
    },
    PointerState {
        request: HostRequestId,
        coordinate: PointerCoordinate,
        presentation_revision: u64,
        environment_revision: u64,
        projection_space_revision: u64,
    },
    Extension {
        request: HostRequestId,
        return_type: era_runtime_protocol::ExtensionValueType,
        mutable_places: Vec<Option<(PlaceDescriptor, era_runtime_protocol::ExtensionValueType)>>,
    },
    HtmlQuery {
        continuation: Box<crate::session::html_query::HtmlQueryContinuation>,
    },
    TextExtent {
        request: HostRequestId,
        context: era_runtime_protocol::ProjectionQueryContext,
    },
    DrawTextExtent {
        request: HostRequestId,
        context: era_runtime_protocol::ProjectionQueryContext,
        canvas_id: i64,
        text: String,
        point: [i32; 2],
    },
    CanvasPixel {
        request: HostRequestId,
        context: era_runtime_protocol::ProjectionQueryContext,
        canvas_id: i64,
        canvas_revision: u64,
    },
    DecodeCanvasImage {
        request: HostRequestId,
        canvas_id: i64,
        encoded: Vec<u8>,
    },
    EncodeCanvasPng {
        request: HostRequestId,
        relative_path: String,
    },
    SerializePhysicalHistory {
        request: HostRequestId,
        context: era_runtime_protocol::ProjectionQueryContext,
        relative_path: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum PointerCoordinate {
    X,
    Y,
    Button,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum ClockOperation {
    Time,
    Times,
    Millisecond,
    Second,
}

pub(crate) fn input_wait(
    request: &VmHostRequest,
    wait_id: u64,
    submission_token: InteractionToken,
    logical_time_ns: u64,
) -> Option<PendingInput> {
    let name = request.import.import.name.to_ascii_uppercase();
    let arguments = &request.arguments;
    let (kind, one_input, stop_message_skip, result_name) = match name.as_str() {
        "WAIT" | "FORCEWAIT" => (WaitKind::EnterKey, false, name == "FORCEWAIT", None),
        "WAITANYKEY" => (WaitKind::AnyKey, false, false, None),
        "INPUT" | "ONEINPUT" | "TINPUT" | "TONEINPUT" | "TINPUTNF" | "TONEINPUTNF" => (
            WaitKind::IntegerValue,
            name.starts_with("ONE") || name.starts_with("TONE"),
            false,
            Some("RESULT"),
        ),
        "INPUTS" | "ONEINPUTS" | "TINPUTS" | "TONEINPUTS" | "TINPUTSNF" | "TONEINPUTSNF" => (
            WaitKind::StringValue,
            name.starts_with("ONE") || name.starts_with("TONE"),
            false,
            Some("RESULTS"),
        ),
        "INPUTANY" => (WaitKind::AnyValue, false, false, None),
        "BINPUT" | "ONEBINPUT" => (
            WaitKind::IntegerButton,
            name.starts_with("ONE"),
            false,
            Some("RESULT"),
        ),
        "BINPUTS" | "ONEBINPUTS" => (
            WaitKind::StringButton,
            name.starts_with("ONE"),
            false,
            Some("RESULTS"),
        ),
        "INPUTMOUSEKEY" => (WaitKind::PrimitiveMouseKey, false, false, Some("RESULT")),
        "TWAIT" => {
            let void = integer(arguments.get(1)).is_some_and(|flag| flag != 0);
            (
                if void {
                    WaitKind::Void
                } else {
                    WaitKind::EnterKey
                },
                false,
                false,
                None,
            )
        }
        _ => return None,
    };
    // INPUTMOUSEKEY is reference-timed even though its name has no T prefix.
    let timed = name.starts_with('T') || name == "INPUTMOUSEKEY";
    let timelimit_ms = if timed {
        integer(arguments.first()).unwrap_or(-1)
    } else {
        -1
    };
    let default_index = usize::from(timed);
    let default_value = match kind {
        WaitKind::IntegerValue | WaitKind::IntegerButton => {
            integer(arguments.get(default_index)).map(ProtocolValue::Integer)
        }
        WaitKind::StringValue | WaitKind::StringButton => {
            string(arguments.get(default_index)).map(|value| ProtocolValue::String(value.into()))
        }
        _ => None,
    };
    let deadline_ns = (timelimit_ms > 0).then(|| {
        logical_time_ns.saturating_add(timelimit_ms.cast_unsigned().saturating_mul(1_000_000))
    });
    let timed_value_input = timed && name != "TWAIT";
    let display_time = timed_value_input && integer(arguments.get(2)).unwrap_or(1) != 0;
    let timeout_message =
        timed_value_input.then(|| string(arguments.get(3)).unwrap_or("時間切れ").to_owned());
    let mouse_input = if timed_value_input {
        integer(arguments.get(4)) == Some(1)
    } else {
        integer(arguments.get(1)).is_some_and(|value| value != 0)
    };
    Some(PendingInput {
        host_request: Some(request.id),
        wait: InputWait {
            wait_id,
            kind,
            stability: WaitStability::for_reference_wait(kind, timelimit_ms),
            one_input,
            stop_message_skip,
            system_input: false,
            mouse_input,
            default_value,
            deadline_ns,
            display_time,
            timeout_message,
            submission_token,
            viewport_policy: input_viewport_policy(&name),
            countdown_remaining_ms: deadline_ns
                .filter(|_| display_time)
                .map(|_| timelimit_ms.cast_unsigned()),
        },
        result_name: result_name.map(str::to_owned),
        choices: std::collections::BTreeMap::new(),
        timeout_duration_ns: (timelimit_ms > 0)
            .then(|| timelimit_ms.cast_unsigned().saturating_mul(1_000_000)),
        post_input: None,
    })
}

fn input_viewport_policy(name: &str) -> era_runtime_protocol::InputViewportPolicy {
    if name.ends_with("NF") {
        era_runtime_protocol::InputViewportPolicy::PreserveUserViewport
    } else {
        era_runtime_protocol::InputViewportPolicy::FollowOutput
    }
}

fn integer(value: Option<&VmValue>) -> Option<i64> {
    match value {
        Some(VmValue::Integer(value)) if *value != i64::MIN => Some(*value),
        _ => None,
    }
}

fn string(value: Option<&VmValue>) -> Option<&str> {
    match value {
        Some(VmValue::String(value)) => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use erabasic_bytecode::{
        CapabilityFallback, HostCapability, HostImport, HostSnapshotCapability, OperationContract,
        OperationDebugPolicy, OperationHotReloadPolicy, OperationPersistence,
        OperationSnapshotPolicy, OperationState, OperationWaitPolicy, RuntimeImport, SymbolKey,
        TransactionPolicy,
    };
    use erabasic_vm::{FiberId, GenerationId, HostRequestId, VmExecutionOrigin};

    use super::*;

    fn request(name: &str, arguments: Vec<VmValue>) -> VmHostRequest {
        let contract = OperationContract {
            state: OperationState::Controller,
            transaction: TransactionPolicy::Forbidden,
            candidate: erabasic_bytecode::CandidatePolicy::Forbidden,
            persistence: OperationPersistence::RuntimeOnly,
            snapshot: OperationSnapshotPolicy::Included,
            hot_reload: OperationHotReloadPolicy::Preserve,
            wait: OperationWaitPolicy::StableInput,
            capability_fallback: CapabilityFallback::ScriptResult,
            debug: OperationDebugPolicy::Forbidden,
            portability: erabasic_bytecode::OperationPortability::Portable,
        };
        VmHostRequest {
            id: HostRequestId(1),
            fiber: FiberId(1),
            import: HostImport {
                import: RuntimeImport {
                    key: SymbolKey::derive("test.host", name.as_bytes()),
                    namespace: "rustyera.input".into(),
                    name: name.into(),
                    abi_version: 1,
                    parameters: Vec::new(),
                    result: None,
                },
                effect: contract.effect(),
                capability: HostCapability::Input,
                snapshot_capability: HostSnapshotCapability::StableWait,
                contract,
            },
            arguments,
            omitted_arguments: Vec::new(),
            origin: VmExecutionOrigin {
                generation: GenerationId(1),
                function: SymbolKey::derive("test.function", b"TEST"),
                function_name: "TEST".into(),
                instruction: 0,
                command: name.into(),
                source: None,
            },
        }
    }

    #[test]
    fn nf_waits_reuse_timed_flags_and_only_change_viewport_policy() {
        for (ordinary, nf, default) in [
            ("TINPUT", "TINPUTNF", VmValue::Integer(7)),
            ("TONEINPUT", "TONEINPUTNF", VmValue::Integer(7)),
            ("TINPUTS", "TINPUTSNF", VmValue::String("default".into())),
            (
                "TONEINPUTS",
                "TONEINPUTSNF",
                VmValue::String("default".into()),
            ),
        ] {
            let arguments = vec![
                VmValue::Integer(100),
                default,
                VmValue::Integer(1),
                VmValue::String("timeout".into()),
                VmValue::Integer(0),
                VmValue::Integer(0),
            ];
            let token = InteractionToken { epoch: 1, id: 3 };
            let ordinary = input_wait(&request(ordinary, arguments.clone()), 7, token, 42).unwrap();
            let mut nf = input_wait(&request(nf, arguments), 7, token, 42).unwrap();
            assert_eq!(
                nf.wait.viewport_policy,
                era_runtime_protocol::InputViewportPolicy::PreserveUserViewport
            );
            nf.wait.viewport_policy = era_runtime_protocol::InputViewportPolicy::FollowOutput;
            assert_eq!(nf.wait, ordinary.wait);
            assert_eq!(nf.result_name, ordinary.result_name);
        }
    }

    #[test]
    fn timed_string_input_preserves_reference_flags() {
        let pending = input_wait(
            &request(
                "TONEINPUTS",
                vec![
                    VmValue::Integer(1000),
                    VmValue::String("DEFAULT".into()),
                    VmValue::Integer(1),
                    VmValue::String("timeout".into()),
                    VmValue::Integer(0),
                    VmValue::Integer(0),
                ],
            ),
            7,
            InteractionToken { epoch: 1, id: 3 },
            5_000_000,
        )
        .expect("known input instruction");
        assert_eq!(pending.wait.kind, WaitKind::StringValue);
        assert!(pending.wait.one_input);
        assert!(!pending.wait.mouse_input);
        assert_eq!(pending.wait.deadline_ns, Some(1_005_000_000));
        assert_eq!(pending.wait.stability, WaitStability::Transient);
        assert_eq!(
            pending.wait.default_value,
            Some(ProtocolValue::String("DEFAULT".into()))
        );
        assert_eq!(pending.wait.timeout_message.as_deref(), Some("timeout"));
        assert_eq!(pending.result_name.as_deref(), Some("RESULTS"));
    }

    #[test]
    fn untimed_one_input_uses_the_shared_default_mouse_and_skip_slots() {
        let pending = input_wait(
            &request(
                "ONEINPUTS",
                vec![
                    VmValue::String("DEFAULT".into()),
                    VmValue::Integer(1),
                    VmValue::Integer(0),
                ],
            ),
            8,
            InteractionToken { epoch: 1, id: 4 },
            0,
        )
        .expect("known input instruction");
        assert_eq!(pending.wait.kind, WaitKind::StringValue);
        assert!(pending.wait.one_input);
        assert!(pending.wait.mouse_input);
        assert_eq!(pending.wait.stability, WaitStability::StableInput);
        assert_eq!(
            pending.wait.default_value,
            Some(ProtocolValue::String("DEFAULT".into()))
        );
    }

    #[test]
    fn forcewait_and_twait_keep_distinct_reference_semantics() {
        let force = input_wait(
            &request("FORCEWAIT", Vec::new()),
            1,
            InteractionToken { epoch: 1, id: 1 },
            0,
        )
        .expect("known wait instruction");
        assert!(force.wait.stop_message_skip);
        assert_eq!(force.wait.stability, WaitStability::StableInput);

        let timed = input_wait(
            &request("TWAIT", vec![VmValue::Integer(100), VmValue::Integer(1)]),
            2,
            InteractionToken { epoch: 1, id: 1 },
            0,
        )
        .expect("known timed wait");
        assert_eq!(timed.wait.kind, WaitKind::Void);
        assert_eq!(timed.wait.stability, WaitStability::Transient);
        assert!(!timed.wait.display_time);
        assert_eq!(timed.wait.timeout_message, None);
    }
}
