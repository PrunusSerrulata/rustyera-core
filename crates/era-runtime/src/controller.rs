use std::collections::VecDeque;

use erabasic_bytecode::{BytecodeArtifact, BytecodeEventEntry, SymbolKey};
use erabasic_vm::{FiberId, VmValue};
use serde::{Deserialize, Serialize};

/// Reference system phases are runtime state, never inferred from frontend screens.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum SystemStep {
    #[default]
    None,
    TrainEvent,
    TrainShowStatus,
    TrainComAble,
    TrainShowUser,
    TrainUserCom,
    TrainEventCom,
    TrainCommand,
    TrainSourceCheck,
    TrainEventComEnd,
    TrainEventComEndWait,
    TrainCallTrainEnd,
    TrainBeginAfterCallTrainEnd,
    AblupShowJuel,
    AblupShowSelect,
    AblupAction,
    ShopEvent,
    ShopAutosave,
    ShopAutosaveFailureWait,
    ShopShow,
    ShopAction,
    /// A project-defined title load hook returns to the built-in title menu.
    TitleLoadOverride,
    /// Ordinary restore hooks completed; enter `SHOW_SHOP` without an immediate autosave.
    PostLoadShop,
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct DispatchEntry {
    function: SymbolKey,
    single: bool,
    group: u8,
}

/// Runs Emuera event groups one root fiber at a time. Keeping the sequence outside
/// the VM lets the runtime atomically commit authoritative state between handlers.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct SystemController {
    pub(crate) flow: Option<SystemFlow>,
    pub(crate) step: SystemStep,
    pub(crate) selected_command: Option<i64>,
    pub(crate) train_scan: usize,
    pub(crate) train_commands: Vec<i64>,
    pub(crate) continuous_commands: VecDeque<i64>,
    pub(crate) continuous_train: bool,
    pub(crate) continuous_total: usize,
    pub(crate) continuous_executed: usize,
    pub(crate) event_com_end_wait_required: bool,
    pub(crate) deferred_flow: Option<SystemFlow>,
    /// Reference autosave is only performed when BEGIN SHOP originated in Normal.
    pub(crate) shop_called_when_normal: bool,
    pending: VecDeque<DispatchEntry>,
    active: Option<(FiberId, DispatchEntry)>,
}

impl SystemController {
    pub(crate) const fn allows_dotrain(&self) -> bool {
        matches!(
            self.step,
            SystemStep::TrainEvent
                | SystemStep::TrainShowStatus
                | SystemStep::TrainShowUser
                | SystemStep::TrainUserCom
                | SystemStep::TrainEventComEnd
        )
    }
    pub(crate) fn clear(&mut self) {
        self.pending.clear();
        self.active = None;
    }

    pub(crate) fn clear_continuous_train(&mut self) {
        self.continuous_commands.clear();
        self.continuous_train = false;
        self.continuous_total = 0;
        self.continuous_executed = 0;
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

    pub(crate) fn prepare_function(&mut self, artifact: &BytecodeArtifact, name: &str) -> bool {
        self.clear();
        let Some(function) = artifact
            .functions
            .iter()
            .find(|function| function.name.eq_ignore_ascii_case(name))
        else {
            return false;
        };
        self.pending.push_back(DispatchEntry {
            function: function.key,
            single: false,
            group: u8::MAX,
        });
        true
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.active.is_none() && self.pending.is_empty()
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

    #[test]
    fn dotrain_is_limited_to_reference_train_phases() {
        let mut controller = SystemController::default();
        for step in [
            SystemStep::TrainEvent,
            SystemStep::TrainShowStatus,
            SystemStep::TrainShowUser,
            SystemStep::TrainUserCom,
            SystemStep::TrainEventComEnd,
        ] {
            controller.step = step;
            assert!(controller.allows_dotrain(), "{step:?}");
        }
        for step in [
            SystemStep::TrainComAble,
            SystemStep::TrainEventCom,
            SystemStep::TrainCommand,
            SystemStep::TrainSourceCheck,
            SystemStep::TrainEventComEndWait,
        ] {
            controller.step = step;
            assert!(!controller.allows_dotrain(), "{step:?}");
        }
    }
}
