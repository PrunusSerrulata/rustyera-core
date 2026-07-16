use std::collections::VecDeque;

use erabasic_bytecode::{BytecodeArtifact, BytecodeEventEntry, SymbolKey};
use erabasic_vm::{FiberId, VmValue};

/// Reference system phases are runtime state, never inferred from frontend screens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SystemFlow {
    Title,
    First,
    Train,
    AfterTrain,
    Ablup,
    TurnEnd,
    Shop,
    Normal,
}

impl SystemFlow {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "TITLE" => Some(Self::Title),
            "FIRST" => Some(Self::First),
            "TRAIN" => Some(Self::Train),
            "AFTERTRAIN" => Some(Self::AfterTrain),
            "ABLUP" => Some(Self::Ablup),
            "TURNEND" => Some(Self::TurnEnd),
            "SHOP" => Some(Self::Shop),
            "NORMAL" => Some(Self::Normal),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DispatchEntry {
    function: SymbolKey,
    single: bool,
    group: u8,
}

/// Runs Emuera event groups one root fiber at a time. Keeping the sequence outside
/// the VM lets the runtime atomically commit authoritative state between handlers.
#[derive(Default)]
pub(crate) struct SystemController {
    pub(crate) flow: Option<SystemFlow>,
    pending: VecDeque<DispatchEntry>,
    active: Option<(FiberId, DispatchEntry)>,
}

impl SystemController {
    pub(crate) fn clear(&mut self) {
        self.pending.clear();
        self.active = None;
    }

    pub(crate) fn prepare_event(&mut self, artifact: &BytecodeArtifact, name: &str) -> bool {
        self.clear();
        let Some(event) = artifact
            .event_groups
            .iter()
            .find(|event| event.name.eq_ignore_ascii_case(name))
        else {
            return false;
        };
        self.prepare_group(event);
        !self.pending.is_empty()
    }

    /// Queue the post-load system hook followed by the EVENTLOAD group. This keeps the
    /// reference ordering while still running one root fiber at a time.
    pub(crate) fn prepare_load_sequence(&mut self, artifact: &BytecodeArtifact) -> bool {
        self.clear();
        if let Some(function) = artifact
            .functions
            .iter()
            .find(|function| function.name.eq_ignore_ascii_case("SYSTEM_LOADEND"))
        {
            self.pending.push_back(DispatchEntry {
                function: function.key,
                single: false,
                group: u8::MAX,
            });
        }
        if let Some(event) = artifact
            .event_groups
            .iter()
            .find(|event| event.name.eq_ignore_ascii_case("EVENTLOAD"))
        {
            self.prepare_group(event);
        }
        !self.pending.is_empty()
    }

    fn prepare_group(&mut self, event: &erabasic_bytecode::BytecodeEventGroup) {
        if event.only.is_empty() {
            self.extend(&event.priority, 1);
            self.extend(&event.normal, 2);
            self.extend(&event.later, 3);
        } else {
            self.extend(&event.only, 0);
        }
    }

    fn extend(&mut self, entries: &[BytecodeEventEntry], group: u8) {
        self.pending
            .extend(entries.iter().map(|entry| DispatchEntry {
                function: entry.function,
                single: entry.single,
                group,
            }));
    }

    pub(crate) fn next(&mut self) -> Option<SymbolKey> {
        if self.active.is_some() {
            return None;
        }
        self.pending.front().map(|entry| entry.function)
    }

    pub(crate) fn started(&mut self, fiber: FiberId) {
        let entry = self.pending.pop_front().expect("prepared event entry");
        self.active = Some((fiber, entry));
    }

    pub(crate) fn completed(&mut self, fiber: FiberId, value: Option<&VmValue>) -> bool {
        let Some((active_fiber, entry)) = self.active.take() else {
            return false;
        };
        if active_fiber != fiber {
            self.active = Some((active_fiber, entry));
            return false;
        }
        // #SINGLE only skips the rest of its current PRI/normal/LATER group when
        // the function returns exactly one.
        if entry.single && value == Some(&VmValue::Integer(1)) {
            while self
                .pending
                .front()
                .is_some_and(|candidate| candidate.group == entry.group)
            {
                self.pending.pop_front();
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use erabasic_bytecode::{BytecodeEventGroup, SymbolKey};

    use super::*;

    fn key(value: u8) -> SymbolKey {
        SymbolKey([value; 16])
    }

    #[test]
    fn single_return_one_skips_only_the_current_group() {
        let event = BytecodeEventGroup {
            name: "EVENTFIRST".into(),
            only: Vec::new(),
            priority: vec![
                BytecodeEventEntry {
                    function: key(1),
                    single: true,
                },
                BytecodeEventEntry {
                    function: key(2),
                    single: false,
                },
            ],
            normal: vec![BytecodeEventEntry {
                function: key(3),
                single: false,
            }],
            later: Vec::new(),
        };
        let mut controller = SystemController::default();
        controller.prepare_group(&event);
        assert_eq!(controller.next(), Some(key(1)));
        controller.started(FiberId(7));
        assert!(controller.completed(FiberId(7), Some(&VmValue::Integer(1))));
        assert_eq!(controller.next(), Some(key(3)));
    }

    #[test]
    fn only_group_suppresses_all_regular_event_groups() {
        let event = BytecodeEventGroup {
            name: "EVENTSHOP".into(),
            only: vec![BytecodeEventEntry {
                function: key(9),
                single: false,
            }],
            priority: vec![BytecodeEventEntry {
                function: key(1),
                single: false,
            }],
            normal: vec![BytecodeEventEntry {
                function: key(2),
                single: false,
            }],
            later: vec![BytecodeEventEntry {
                function: key(3),
                single: false,
            }],
        };
        let mut controller = SystemController::default();
        controller.prepare_group(&event);
        assert_eq!(controller.next(), Some(key(9)));
        controller.started(FiberId(1));
        assert!(controller.completed(FiberId(1), None));
        assert_eq!(controller.next(), None);
    }
}
