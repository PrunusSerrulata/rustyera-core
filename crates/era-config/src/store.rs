use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{ConfigValue, catalog, tui_default, web_default};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConfigStore {
    #[serde(default)]
    pub(crate) compatibility_profile: erabasic_compat::CompatibilityProfileId,
    pub(crate) values: BTreeMap<String, ConfigValue>,
    pub(crate) fixed: BTreeMap<String, bool>,
    // Source tracking is derived from project configuration documents. It is intentionally
    // excluded from persistent compiler-cache payloads so the cache wire layout stays stable.
    #[serde(skip, default)]
    pub(crate) specified: BTreeSet<String>,
}

impl Default for ConfigStore {
    fn default() -> Self {
        let values = catalog()
            .into_iter()
            .map(|spec| (spec.code.to_ascii_uppercase(), spec.default))
            .collect();
        Self {
            compatibility_profile: erabasic_compat::CompatibilityProfileId::default(),
            values,
            fixed: BTreeMap::new(),
            specified: BTreeSet::new(),
        }
    }
}

impl ConfigStore {
    /// Language profile is project state, never a client preference.
    #[must_use]
    pub const fn compatibility_profile(&self) -> erabasic_compat::CompatibilityProfileId {
        self.compatibility_profile
    }

    pub(crate) fn assign_explicit(&mut self, code: &str, value: ConfigValue) {
        let code = code.to_ascii_uppercase();
        self.values.insert(code.clone(), value);
        self.specified.insert(code);
    }

    /// Construct the catalog with Textual-specific defaults before project files apply.
    #[must_use]
    pub fn with_tui_defaults() -> Self {
        let mut store = Self::default();
        for spec in catalog() {
            if let Some(value) = tui_default(spec.code) {
                store.values.insert(spec.code.to_ascii_uppercase(), value);
            }
        }
        store
    }

    /// Construct the catalog with browser/Tauri defaults before project files apply.
    #[must_use]
    pub fn with_web_defaults() -> Self {
        let mut store = Self::default();
        for spec in catalog() {
            if let Some(value) = web_default(spec.code) {
                store.values.insert(spec.code.to_ascii_uppercase(), value);
            }
        }
        store
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ConfigValue> {
        let code = resolve_code(name)?;
        self.values.get(&code)
    }

    #[must_use]
    pub fn get_code(&self, code: &str) -> Option<&ConfigValue> {
        self.values.get(&code.to_ascii_uppercase())
    }

    #[must_use]
    pub fn is_fixed(&self, code: &str) -> bool {
        self.fixed
            .get(&code.to_ascii_uppercase())
            .copied()
            .unwrap_or(false)
    }

    /// Whether a project configuration source explicitly assigned this catalog entry.
    #[must_use]
    pub fn is_specified(&self, code: &str) -> bool {
        self.specified.contains(&code.to_ascii_uppercase())
    }

    /// Apply one `name:value` assignment. Unknown keys and invalid values are rejected.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigParseError`] when the key is unknown or the value does not
    /// match the catalog entry's type.
    pub fn apply(&mut self, name: &str, raw: &str, fixed: bool) -> Result<(), ConfigParseError> {
        let code = resolve_code(name).ok_or(ConfigParseError::UnknownKey)?;
        if self.fixed.get(&code).copied().unwrap_or(false) {
            return Ok(());
        }
        let current = self.values.get(&code).ok_or(ConfigParseError::UnknownKey)?;
        let parsed = parse_like(&code, current, raw)?;
        self.values.insert(code.clone(), parsed);
        self.specified.insert(code.clone());
        if fixed {
            self.fixed.insert(code, true);
        }
        Ok(())
    }

    /// Apply a client-only projection even when the project source locked the setting.
    /// This changes neither the source lock nor explicit-source tracking.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigParseError`] for an unknown setting or invalid value.
    pub fn apply_client_override(&mut self, name: &str, raw: &str) -> Result<(), ConfigParseError> {
        let code = resolve_code(name).ok_or(ConfigParseError::UnknownKey)?;
        let current = self.values.get(&code).ok_or(ConfigParseError::UnknownKey)?;
        let parsed = parse_like(&code, current, raw)?;
        self.values.insert(code, parsed);
        Ok(())
    }

    /// Apply an emuera/default/fixed config assignment. Replace and debug items live
    /// in different reference files and therefore are not accepted here.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigParseError`] for an unknown, replace/debug-only, or malformed
    /// assignment.
    pub fn apply_regular(
        &mut self,
        name: &str,
        raw: &str,
        fixed: bool,
    ) -> Result<(), ConfigParseError> {
        let code = resolve_code(name).ok_or(ConfigParseError::UnknownKey)?;
        if !is_regular_code(&code) {
            return Err(ConfigParseError::UnknownKey);
        }
        self.apply(&code, raw, fixed)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &ConfigValue)> {
        self.values.iter().map(|(key, value)| (key.as_str(), value))
    }
}

#[must_use]
pub fn is_regular_code(code: &str) -> bool {
    let code = code.to_ascii_uppercase();
    !is_replace_code(&code)
        && !code.starts_with("DEBUG")
        && !matches!(
            code.as_str(),
            "AUDIOVOLUME" | "REPLACEFULLWIDTHSPACES" | "CHARACTERWIDTHMODE"
        )
}

pub(crate) fn is_replace_code(code: &str) -> bool {
    matches!(
        code,
        "MONEYLABEL"
            | "MONEYFIRST"
            | "LOADLABEL"
            | "MAXSHOPITEM"
            | "DRAWLINESTRING"
            | "BARCHAR1"
            | "BARCHAR2"
            | "TITLEMENUSTRING0"
            | "TITLEMENUSTRING1"
            | "COMABLEDEFAULT"
            | "STAINDEFAULT"
            | "TIMEUPLABEL"
            | "EXPLVDEF"
            | "PALAMLVDEF"
            | "PBANDDEF"
            | "RELATIONDEF"
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigParseError {
    UnknownKey,
    InvalidValue,
}

pub(crate) fn resolve_code(name: &str) -> Option<String> {
    let key = name.trim().to_uppercase();
    catalog()
        .into_iter()
        .find(|spec| {
            spec.code.to_ascii_uppercase() == key
                || spec.japanese.to_uppercase() == key
                || spec.english.to_uppercase() == key
        })
        .map(|spec| spec.code.to_ascii_uppercase())
}

fn parse_like(
    code: &str,
    current: &ConfigValue,
    raw: &str,
) -> Result<ConfigValue, ConfigParseError> {
    if raw.is_empty() {
        return Err(ConfigParseError::InvalidValue);
    }
    let value = raw.trim();
    if code == "USEMENU"
        && let ConfigValue::Enum { allowed, .. } = current
    {
        let migrated = match value.to_ascii_uppercase().as_str() {
            "YES" | "TRUE" | "1" | "前" => Some("AUTO"),
            "NO" | "FALSE" | "0" | "後" => Some("HIDE"),
            _ => None,
        };
        if let Some(migrated) = migrated {
            return Ok(ConfigValue::Enum {
                value: migrated.into(),
                allowed: allowed.clone(),
            });
        }
    }
    match current {
        ConfigValue::Boolean(_) => {
            if let Ok(value) = value.parse::<i32>() {
                return Ok(ConfigValue::Boolean(value != 0));
            }
            match value.to_ascii_uppercase().as_str() {
                "YES" | "TRUE" | "前" => Ok(ConfigValue::Boolean(true)),
                "NO" | "FALSE" | "後" => Ok(ConfigValue::Boolean(false)),
                _ => Err(ConfigParseError::InvalidValue),
            }
        }
        ConfigValue::Integer(_) => if matches!(code, "LASTKEY" | "PBANDDEF" | "RELATIONDEF") {
            value.parse::<i64>()
        } else {
            value.parse::<i32>().map(i64::from)
        }
        .map(ConfigValue::Integer)
        .map_err(|_| ConfigParseError::InvalidValue),
        ConfigValue::String(_) => Ok(ConfigValue::String(value.into())),
        ConfigValue::Enum { allowed, .. } => {
            let parsed = allowed
                .iter()
                .find(|candidate| candidate.eq_ignore_ascii_case(value))
                .cloned()
                .or_else(|| {
                    value.parse::<i32>().ok().map(|ordinal| {
                        usize::try_from(ordinal)
                            .ok()
                            .and_then(|index| allowed.get(index).cloned())
                            .unwrap_or_else(|| ordinal.to_string())
                    })
                })
                .ok_or(ConfigParseError::InvalidValue)?;
            Ok(ConfigValue::Enum {
                value: parsed,
                allowed: allowed.clone(),
            })
        }
        ConfigValue::Color(_) => {
            let parts = value
                .split(',')
                .take(3)
                .map(str::trim)
                .map(str::parse::<u8>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| ConfigParseError::InvalidValue)?;
            let [r, g, b, ..] = parts.as_slice() else {
                return Err(ConfigParseError::InvalidValue);
            };
            Ok(ConfigValue::Color(
                (u32::from(*r) << 16) | (u32::from(*g) << 8) | u32::from(*b),
            ))
        }
        ConfigValue::Character(_) => {
            let mut characters = value.chars();
            let character = characters.next().ok_or(ConfigParseError::InvalidValue)?;
            if characters.next().is_some() {
                return Err(ConfigParseError::InvalidValue);
            }
            Ok(ConfigValue::Character(character))
        }
        ConfigValue::IntegerList(_) => value
            .split('/')
            .map(str::trim)
            .map(str::parse::<i64>)
            .collect::<Result<Vec<_>, _>>()
            .map(ConfigValue::IntegerList)
            .map_err(|_| ConfigParseError::InvalidValue),
        ConfigValue::StringList(_) => Ok(ConfigValue::StringList(
            value.split(',').map(str::trim).map(Into::into).collect(),
        )),
    }
}
