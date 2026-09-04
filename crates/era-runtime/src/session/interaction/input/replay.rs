impl RuntimeSession {
    pub(in crate::session) fn replay_step_draft(
        &self,
        pending: &PendingInput,
        intent: &InputIntent,
        submission: &InputSubmission,
        message_skip: bool,
    ) -> Option<crate::input_replay::ReplayStepDraft> {
        let action = crate::input_replay::action_for_intent(intent)?;
        let result = match submission {
            InputSubmission::Value(value) => crate::input_replay::ReplayValue::from_vm(value),
            InputSubmission::Primitive(_) => None,
        };
        let text = match intent {
            InputIntent::AnyKey(value) | InputIntent::CommitText(value) => Some(value.clone()),
            _ => None,
        };
        let button = match intent {
            InputIntent::Activate(token) => {
                Some(self.presentation.replay_button(*token, result.clone()?)?)
            }
            _ => None,
        };
        let primitive = match (intent, submission) {
            (InputIntent::Primitive(_), InputSubmission::Primitive(result)) => {
                Some(crate::input_replay::ReplayPrimitive::from_result(
                    result.fields,
                    result
                        .selection
                        .as_ref()
                        .and_then(crate::input_replay::ReplayValue::from_vm),
                ))
            }
            _ => None,
        };
        Some(crate::input_replay::ReplayStepDraft {
            source: self.active_input_source.clone(),
            action,
            wait_kind: pending.wait.kind.into(),
            result,
            message_skip,
            text,
            button,
            primitive,
        })
    }

}
