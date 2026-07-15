use era_runtime_protocol::{InputWait, ProtocolValue, WaitKind, WaitStability};
use erabasic_bytecode::BytecodeType;
use erabasic_vm::{HostRequestId, VmHostRequest, VmValue};

#[derive(Clone, Debug)]
pub(crate) struct PendingInput {
    pub(crate) host_request: Option<HostRequestId>,
    pub(crate) wait: InputWait,
    pub(crate) result_name: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExternalCompletion {
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClockOperation {
    Time,
    Times,
    Millisecond,
}

pub(crate) fn input_wait(
    request: &VmHostRequest,
    wait_id: u64,
    button_generation: u64,
    logical_time_ns: u64,
) -> Option<PendingInput> {
    let name = request.import.import.name.to_ascii_uppercase();
    let arguments = &request.arguments;
    let (kind, one_input, stop_message_skip, result_name) = match name.as_str() {
        "WAIT" | "FORCEWAIT" => (WaitKind::EnterKey, false, name == "FORCEWAIT", None),
        "WAITANYKEY" => (WaitKind::AnyKey, false, false, None),
        "INPUT" | "ONEINPUT" | "TINPUT" | "TONEINPUT" => (
            WaitKind::IntegerValue,
            name.starts_with("ONE") || name.starts_with("TONE"),
            false,
            Some("RESULT"),
        ),
        "INPUTS" | "ONEINPUTS" | "TINPUTS" | "TONEINPUTS" => (
            WaitKind::StringValue,
            name.starts_with("ONE") || name.starts_with("TONE"),
            false,
            Some("RESULTS"),
        ),
        "INPUTANY" => (WaitKind::AnyValue, false, false, Some("RESULT")),
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
    let timed = name.starts_with('T');
    let timelimit_ms = if timed {
        integer(arguments.first()).unwrap_or(-1)
    } else {
        -1
    };
    let default_value = match kind {
        WaitKind::IntegerValue | WaitKind::IntegerButton => {
            integer(arguments.get(1)).map(ProtocolValue::Integer)
        }
        WaitKind::StringValue | WaitKind::StringButton => {
            string(arguments.get(1)).map(|value| ProtocolValue::String(value.into()))
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
    let mouse_input = timed_value_input && integer(arguments.get(4)) == Some(1);
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
            button_generation,
        },
        result_name,
    })
}

fn integer(value: Option<&VmValue>) -> Option<i64> {
    match value {
        Some(VmValue::Integer(value)) => Some(*value),
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
        HostCapability, HostEffect, HostImport, HostSnapshotCapability, RuntimeImport, SymbolKey,
    };
    use erabasic_vm::{FiberId, HostRequestId};

    use super::*;

    fn request(name: &str, arguments: Vec<VmValue>) -> VmHostRequest {
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
                effect: HostEffect {
                    may_suspend: true,
                    ..HostEffect::default()
                },
                capability: HostCapability::Input,
                snapshot_capability: HostSnapshotCapability::StableWait,
            },
            arguments,
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
            3,
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
        assert_eq!(pending.result_name, Some("RESULTS"));
    }

    #[test]
    fn forcewait_and_twait_keep_distinct_reference_semantics() {
        let force =
            input_wait(&request("FORCEWAIT", Vec::new()), 1, 1, 0).expect("known wait instruction");
        assert!(force.wait.stop_message_skip);
        assert_eq!(force.wait.stability, WaitStability::StableInput);

        let timed = input_wait(
            &request("TWAIT", vec![VmValue::Integer(100), VmValue::Integer(1)]),
            2,
            1,
            0,
        )
        .expect("known timed wait");
        assert_eq!(timed.wait.kind, WaitKind::Void);
        assert_eq!(timed.wait.stability, WaitStability::Transient);
        assert!(!timed.wait.display_time);
        assert_eq!(timed.wait.timeout_message, None);
    }
}
