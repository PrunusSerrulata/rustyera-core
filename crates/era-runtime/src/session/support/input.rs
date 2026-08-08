#[allow(clippy::wildcard_imports)]
use super::super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in super::super) enum InputSubmission {
    Value(VmValue),
    Primitive(PrimitiveResult),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in super::super) struct PrimitiveResult {
    pub(in super::super) fields: [i32; 5],
    pub(in super::super) selection: Option<VmValue>,
}

pub(in super::super) fn input_value(
    pending: &PendingInput,
    token: InteractionToken,
    intent: InputIntent,
    allow_long_activation: bool,
) -> Option<InputSubmission> {
    if let InputIntent::Activate(activated) = intent {
        if token != pending.wait.submission_token {
            return None;
        }
        let value = pending.choices.get(&activated)?;
        if pending.wait.kind == WaitKind::PrimitiveMouseKey {
            return Some(InputSubmission::Value(value.clone()));
        }
        let text = match value {
            VmValue::Integer(value) => value.to_string(),
            VmValue::String(value) => value.clone(),
            VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => return None,
        };
        return submitted_text_value(pending, text, allow_long_activation)
            .map(InputSubmission::Value);
    }
    if token != pending.wait.submission_token {
        return None;
    }
    match (pending.wait.kind, intent) {
        (WaitKind::EnterKey | WaitKind::AnyKey, InputIntent::Continue)
        | (WaitKind::EnterKey, InputIntent::Enter)
        | (WaitKind::AnyKey, InputIntent::AnyKey(_)) => {
            Some(InputSubmission::Value(VmValue::Integer(0)))
        }
        (
            WaitKind::IntegerValue
            | WaitKind::StringValue
            | WaitKind::IntegerButton
            | WaitKind::StringButton,
            InputIntent::CommitText(value),
        ) => submitted_text_value(pending, value, false).map(InputSubmission::Value),
        (WaitKind::AnyValue, InputIntent::CommitText(value)) => Some(InputSubmission::Value(
            value
                .parse()
                .map_or_else(|_| VmValue::String(value), VmValue::Integer),
        )),
        (WaitKind::PrimitiveMouseKey, InputIntent::Primitive(value))
            if matches!(value.input_type, 1..=3) =>
        {
            let selection = match value.selection_token {
                Some(token) => Some(pending.choices.get(&token)?.clone()),
                None => None,
            };
            Some(InputSubmission::Primitive(PrimitiveResult {
                fields: [
                    value.input_type,
                    value.result_1,
                    value.result_2,
                    value.result_3,
                    value.result_4,
                ],
                selection,
            }))
        }
        _ => None,
    }
}

fn submitted_text_value(
    pending: &PendingInput,
    mut text: String,
    allow_long_activation: bool,
) -> Option<VmValue> {
    let use_default = text.is_empty() && pending.wait.deadline_ns.is_none();
    let value = if use_default {
        pending.wait.default_value.as_ref().map(protocol_to_vm)
    } else {
        None
    }
    .or_else(|| {
        if pending.wait.one_input && !allow_long_activation {
            text.truncate(text.chars().next().map_or(0, char::len_utf8));
        }
        match pending.wait.kind {
            WaitKind::IntegerValue | WaitKind::IntegerButton => {
                text.parse().ok().map(VmValue::Integer)
            }
            WaitKind::StringValue | WaitKind::StringButton => Some(VmValue::String(text)),
            _ => None,
        }
    })?;
    submission_matches_wait(pending, value)
}

fn submission_matches_wait(pending: &PendingInput, value: VmValue) -> Option<VmValue> {
    match (&pending.wait.kind, &value) {
        (WaitKind::IntegerValue, VmValue::Integer(_))
        | (WaitKind::StringValue, VmValue::String(_)) => Some(value),
        (WaitKind::IntegerButton, VmValue::Integer(candidate)) => pending
            .choices
            .values()
            .any(|choice| matches!(choice, VmValue::Integer(value) if value == candidate))
            .then_some(value),
        (WaitKind::StringButton, VmValue::String(candidate)) => pending
            .choices
            .values()
            .any(|choice| match choice {
                VmValue::Integer(value) => value.to_string() == *candidate,
                VmValue::String(value) => value == candidate,
                VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => false,
            })
            .then_some(value),
        _ => None,
    }
}
