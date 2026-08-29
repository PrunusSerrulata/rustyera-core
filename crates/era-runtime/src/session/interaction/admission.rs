//! All text origins reuse `input_set` expansion and the ordinary wait validator.
#[allow(clippy::wildcard_imports)]
use super::super::*;

pub(in crate::session) enum SequenceSubmission {
    None,
    Value(InputSubmission),
    Command(String),
}

pub(in crate::session) fn fragment_intent(
    pending: &PendingInput,
    fragment: &QueuedInput,
) -> InputIntent {
    if fragment.source.macro_enabled
        && (fragment.source.fragment != 0 || !matches!(fragment.source.root, InputRoot::External))
    {
        queued_text_intent(&pending.wait, fragment.text.clone())
    } else {
        InputIntent::CommitText(fragment.text.clone())
    }
}

impl RuntimeSession {
    pub(in crate::session) fn prepare_text_submission(
        &mut self,
        pending: &PendingInput,
        source: &InputSource,
    ) -> Result<Option<InputSubmission>, RuntimeError> {
        self.prepare_text_fragment(pending, source, 0)
    }

    // An undo history records value waits. Earlier invalid/non-value fragments
    // are not new frontend commands: validate the stored fragment against the
    // same expansion, then queue only its suffix.
    pub(in crate::session) fn prepare_text_fragment(
        &mut self,
        pending: &PendingInput,
        source: &InputSource,
        fragment_start: u32,
    ) -> Result<Option<InputSubmission>, RuntimeError> {
        crate::input_set::ensure_size(source.raw.len()).map_err(RuntimeError::ResourceLimit)?;
        let pieces = if source.macro_enabled {
            preprocess_input(&source.raw).map_err(RuntimeError::ResourceLimit)?
        } else {
            vec![crate::input_set::InputSegment {
                text: source.raw.as_ref().clone(),
                message_skip: false,
            }]
        };
        let bytes = pieces
            .iter()
            .map(|piece| piece.text.len())
            .chain(self.queued_input.iter().map(|piece| piece.text.len()))
            .try_fold(0_usize, usize::checked_add)
            .ok_or(RuntimeError::ResourceLimit("input queue size overflow"))?;
        let mut admissions = BTreeSet::new();
        let retained_sources = self
            .queued_input
            .iter()
            .filter(|piece| admissions.insert(piece.source.admission))
            .try_fold(source.raw.len(), |total, piece| {
                total.checked_add(piece.source.raw.len())
            })
            .ok_or(RuntimeError::ResourceLimit(
                "input provenance size overflow",
            ))?;
        let storage = bytes
            .checked_add(retained_sources)
            .ok_or(RuntimeError::ResourceLimit("input queue storage overflow"))?;
        if bytes > 1024 * 1024
            || storage > 2 * 1024 * 1024
            || pieces.len().saturating_add(self.queued_input.len()) > 65_536
        {
            return Err(RuntimeError::ResourceLimit(
                "input queue exceeds its shared limit",
            ));
        }
        let mut pieces = (0_u32..).zip(pieces).map(|(fragment, piece)| {
            let mut source = source.clone();
            source.fragment = fragment;
            QueuedInput {
                text: piece.text,
                message_skip: source.message_skip || piece.message_skip,
                source,
            }
        });
        let first = pieces
            .nth(usize::try_from(fragment_start).map_err(|_| {
                RuntimeError::Internal("replay fragment cannot address this platform".into())
            })?)
            .ok_or_else(|| {
                RuntimeError::Internal("replay fragment is outside input expansion".into())
            })?;
        // A sequence's expansion precedes, rather than replaces, old macro tails.
        let tails = pieces.collect::<Vec<_>>();
        for piece in tails.into_iter().rev() {
            self.queued_input.push_front(piece);
        }
        Ok(self.prepare_fragment(pending, first))
    }

    pub(in crate::session) fn prepare_fragment(
        &mut self,
        pending: &PendingInput,
        fragment: QueuedInput,
    ) -> Option<InputSubmission> {
        self.active_input_source = None;
        let intent = fragment_intent(pending, &fragment);
        let result = input_value(pending, pending.wait.submission_token, intent, false);
        if result.is_some() {
            self.message_skip = fragment.message_skip;
            self.active_input_source = Some(fragment.source);
        }
        result
    }

    pub(in crate::session) fn sequence_submission(
        &mut self,
        pending: &PendingInput,
    ) -> Result<SequenceSubmission, RuntimeError> {
        let Some(sequence) = self.input_controller.pending_sequence.take() else {
            return Ok(SequenceSubmission::None);
        };
        // Clear first, including Some(""); nested script admission can set a new slot.
        let source = self
            .input_controller
            .admit(InputRoot::Sequence(sequence.site), sequence.text, false)
            .map_err(RuntimeError::ResourceLimit)?;
        if source.macro_enabled
            && source.raw.len() > 1
            && source.raw.starts_with('@')
            && !pending.wait.one_input
        {
            return Ok(SequenceSubmission::Command(source.raw.as_ref().clone()));
        }
        self.prepare_text_submission(pending, &source)
            .map(|value| value.map_or(SequenceSubmission::None, SequenceSubmission::Value))
    }

    pub(in crate::session) fn verify_replayed_input(
        &mut self,
        value: &VmValue,
    ) -> Result<(), RuntimeError> {
        let Some(replay) = self.undo_replay.as_mut() else {
            return Ok(());
        };
        let Some(expected) = replay.remaining.front() else {
            return Ok(());
        };
        let actual = match value {
            VmValue::Integer(value) => value.to_string(),
            VmValue::String(value) => value.clone(),
            _ => return Ok(()),
        };
        let same_source = match (&expected.source, &self.active_input_source) {
            (Some(expected), Some(actual)) => expected.same_replay_origin(actual),
            (None, None) => true,
            _ => false,
        };
        if expected.value != actual || !same_source {
            return Err(RuntimeError::Internal(
                "input replay differs in source, fragment or accepted value".into(),
            ));
        }
        replay.remaining.pop_front();
        Ok(())
    }
}
