use std::fmt::Write as _;

use era_runtime_protocol::{KEY_MACRO_GROUPS, KEY_MACRO_SLOTS, KeyMacroState};
use serde::{Deserialize, Serialize};

const MAX_EXPANDED_BYTES: usize = 1024 * 1024;

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

/// Expand Emuera's keyboard-input mini language before wait-specific validation.
pub(crate) fn preprocess_input(text: &str) -> Result<Vec<(String, bool)>, &'static str> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut position = 0;
    let expanded = expand_sequence(&chars, &mut position, false)?;
    if position != chars.len() || expanded.len() > MAX_EXPANDED_BYTES {
        return Err("input macro expansion exceeds its limit");
    }
    let mut pieces = vec![(String::new(), false)];
    let mut chars = expanded.chars();
    while let Some(character) = chars.next() {
        if character == '\\' {
            match chars.next() {
                Some('n' | 'r') => pieces.push((String::new(), false)),
                Some('e') => pieces.last_mut().expect("one piece").1 = true,
                Some(other) => pieces.last_mut().expect("one piece").0.push(other),
                None => pieces.last_mut().expect("one piece").0.push('\\'),
            }
        } else if matches!(character, '\n' | '\r') {
            pieces.push((String::new(), false));
        } else {
            pieces.last_mut().expect("one piece").0.push(character);
        }
    }
    Ok(pieces)
}

fn expand_sequence(
    chars: &[char],
    position: &mut usize,
    nested: bool,
) -> Result<String, &'static str> {
    let mut output = String::new();
    while *position < chars.len() {
        match chars[*position] {
            ')' if nested => break,
            '(' => {
                *position += 1;
                let group = expand_sequence(chars, position, true)?;
                if chars.get(*position) != Some(&')') {
                    return Err("unclosed input repetition");
                }
                *position += 1;
                let mut count = 1usize;
                if chars.get(*position) == Some(&'*') {
                    *position += 1;
                    let start = *position;
                    while chars.get(*position).is_some_and(char::is_ascii_digit) {
                        *position += 1;
                    }
                    if start == *position {
                        return Err("input repetition has no count");
                    }
                    count = chars[start..*position]
                        .iter()
                        .collect::<String>()
                        .parse()
                        .map_err(|_| "invalid input repetition count")?;
                }
                if group
                    .len()
                    .saturating_mul(count)
                    .saturating_add(output.len())
                    > MAX_EXPANDED_BYTES
                {
                    return Err("input macro expansion exceeds its limit");
                }
                for _ in 0..count {
                    output.push_str(&group);
                }
            }
            character => {
                output.push(character);
                *position += 1;
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_profile_round_trips_and_input_language_expands() {
        let mut macros = KeyMacros::default();
        macros.load("グループ2:custom\nG2:マクロキーF3:abc\\n(def)*2\\e");
        assert_eq!(macros.recall(2, 2), Some("abc\\n(def)*2\\e"));
        assert_eq!(
            preprocess_input(macros.recall(2, 2).unwrap()).unwrap(),
            vec![("abc".into(), false), ("defdef".into(), true)]
        );
        let serialized = macros.state().serialized;
        let mut round_trip = KeyMacros::default();
        round_trip.load(&serialized);
        assert_eq!(round_trip.recall(2, 2), macros.recall(2, 2));
    }
}
