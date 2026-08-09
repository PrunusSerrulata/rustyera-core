use std::fmt::Write as _;

use era_runtime_protocol::{KEY_MACRO_GROUPS, KEY_MACRO_SLOTS, KeyMacroState};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct KeyMacros {
    enabled: bool,
    selected_group: u8,
    group_names: Vec<String>,
    entries: Vec<String>,
}

impl Default for KeyMacros {
    fn default() -> Self {
        Self {
            enabled: true,
            selected_group: 0,
            group_names: (0..KEY_MACRO_GROUPS)
                .map(|group| format!("マクログループ{group}に設定"))
                .collect(),
            entries: vec![String::new(); KEY_MACRO_GROUPS * KEY_MACRO_SLOTS],
        }
    }
}

impl KeyMacros {
    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub(crate) fn load(&mut self, text: &str) {
        let mut loaded = Self {
            enabled: self.enabled,
            ..Self::default()
        };
        for raw in text.trim_start_matches('\u{feff}').lines() {
            let line = raw.trim_end_matches('\r');
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("グループ")
                && let Some((group, name)) = rest.split_once(':')
                && let Ok(group) = group.parse::<usize>()
                && group < KEY_MACRO_GROUPS
            {
                loaded.group_names[group] = name.into();
                continue;
            }
            for group in 0..KEY_MACRO_GROUPS {
                for slot in 0..KEY_MACRO_SLOTS {
                    let prefixes = if group == 0 {
                        [
                            format!("マクロキーF{}:", slot + 1),
                            format!("Macro Key F{}:", slot + 1),
                        ]
                    } else {
                        [
                            format!("G{group}:マクロキーF{}:", slot + 1),
                            format!("G{group}:Macro Key F{}:", slot + 1),
                        ]
                    };
                    if let Some(value) =
                        prefixes.iter().find_map(|prefix| line.strip_prefix(prefix))
                    {
                        loaded.entries[group * KEY_MACRO_SLOTS + slot] = value.into();
                    }
                }
            }
        }
        *self = loaded;
    }

    pub(crate) fn select_group(&mut self, group: u8) -> bool {
        if usize::from(group) >= KEY_MACRO_GROUPS {
            return false;
        }
        self.selected_group = group;
        true
    }

    pub(crate) fn store(&mut self, group: u8, slot: u8, text: String) -> bool {
        let Some(index) = index(group, slot) else {
            return false;
        };
        self.entries[index] = text;
        true
    }

    pub(crate) fn recall(&self, group: u8, slot: u8) -> Option<&str> {
        if !self.enabled {
            return None;
        }
        self.entries.get(index(group, slot)?).map(String::as_str)
    }

    pub(crate) fn state(&self) -> KeyMacroState {
        KeyMacroState {
            enabled: self.enabled,
            selected_group: self.selected_group,
            group_names: self.group_names.clone(),
            entries: self.entries.clone(),
            serialized: self.serialize(),
        }
    }

    fn serialize(&self) -> String {
        let mut output = String::new();
        for (group, name) in self.group_names.iter().enumerate() {
            writeln!(output, "グループ{group}:{name}").expect("writing to a String cannot fail");
        }
        for group in 0..KEY_MACRO_GROUPS {
            for slot in 0..KEY_MACRO_SLOTS {
                if group == 0 {
                    write!(output, "マクロキーF{}:", slot + 1)
                        .expect("writing to a String cannot fail");
                } else {
                    write!(output, "G{group}:マクロキーF{}:", slot + 1)
                        .expect("writing to a String cannot fail");
                }
                output.push_str(&self.entries[group * KEY_MACRO_SLOTS + slot]);
                output.push('\n');
            }
        }
        output
    }
}

fn index(group: u8, slot: u8) -> Option<usize> {
    let group = usize::from(group);
    let slot = usize::from(slot);
    (group < KEY_MACRO_GROUPS && slot < KEY_MACRO_SLOTS).then_some(group * KEY_MACRO_SLOTS + slot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_profile_round_trips() {
        let mut macros = KeyMacros::default();
        macros.load("グループ2:custom\nG2:マクロキーF3:abc\\n(def)*2\\e");
        assert_eq!(macros.recall(2, 2), Some("abc\\n(def)*2\\e"));
        let serialized = macros.state().serialized;
        let mut round_trip = KeyMacros::default();
        round_trip.load(&serialized);
        assert_eq!(round_trip.recall(2, 2), macros.recall(2, 2));
    }
}
