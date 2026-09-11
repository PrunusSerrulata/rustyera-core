//! Fixed-profile comparison facade. No locale, provider, or table is script supplied.
//! Casing applies only to MATCHALLEX name lookup; MAP membership/equality and
//! MATCH string elements remain ordinal. Source-derived, not oracle-verified.
#![forbid(unsafe_code)]
mod culture_search;
pub(crate) use erabasic_compat::OrdinalCasing;
mod search_boundaries;
mod search_boundary_data;

pub(crate) use crate::compat_collation::ce::TextBudget;
use crate::compat_collation::{
    ce::{CeError, CeLimits, LegacyCe32},
    fixed_data::fixed_root_data,
    raw_off::raw_off_elements_bounded,
    simple_affix::{Direction, simple_affix},
};
use crate::{ExecutionFailure, FaultCategory, ScriptFaultKind, VmFaultCode};
pub(crate) use culture_search::SearchMatch;
use culture_search::{LegacyElement, first_match_bounded};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TextError {
    Collation(CeError),
    InvalidElementOffsets,
    SubstringOutOfRange,
    UnsupportedUtf16Substring,
}
impl From<CeError> for TextError {
    fn from(error: CeError) -> Self {
        Self::Collation(error)
    }
}
impl TextError {
    /// Explicit operation-origin mapping; never classify using text or a broad VM code.
    pub(crate) fn failure(self) -> ExecutionFailure {
        match self {
            Self::Collation(CeError::MalformedProvider) | Self::InvalidElementOffsets => {
                ExecutionFailure::classified(
                    FaultCategory::InternalInvariant,
                    VmFaultCode::Native,
                    "fixed ICU72 comparison data or element offsets are invalid",
                )
            }
            Self::Collation(
                CeError::InputLimit
                | CeError::ElementLimit
                | CeError::ContextLimit
                | CeError::WorkLimit
                | CeError::ByteLimit
                | CeError::Allocation,
            ) => ExecutionFailure::classified(
                FaultCategory::ResourceLimit,
                VmFaultCode::Native,
                "fixed comparison exceeded the VM operation resource limit",
            ),
            Self::SubstringOutOfRange => ExecutionFailure::script(
                ScriptFaultKind::Argument,
                VmFaultCode::Native,
                "MAP_FROMSTRING literal separator length exceeds the entry",
            ),
            Self::UnsupportedUtf16Substring => ExecutionFailure::script(
                ScriptFaultKind::Argument,
                VmFaultCode::Native,
                "MAP_FROMSTRING substring would contain an unpaired UTF-16 surrogate; UTF-8 strings cannot represent it",
            ),
        }
    }
}

pub(crate) fn match_name_equals(left: &str, right: &str, ignore_case: bool) -> bool {
    OrdinalCasing::fixed_dotnet8_icu72().equals(left, right, ignore_case)
}

fn elements(text: &str, budget: &mut TextBudget) -> Result<(usize, Vec<LegacyCe32>), TextError> {
    let mut input = Vec::new();
    for unit in text.encode_utf16() {
        budget.step()?;
        budget.push(&mut input, unit)?;
    }
    let length = input.len();
    let limits = CeLimits {
        utf16_units: length,
        ce64: budget.remaining_work(),
        context_depth: 64,
    };
    let stream = raw_off_elements_bounded(&fixed_root_data()?, &input, limits, budget)?;
    Ok((length, stream.legacy_elements_bounded(budget)?))
}

fn search_elements(
    input: &[LegacyCe32],
    budget: &mut TextBudget,
) -> Result<Vec<LegacyElement>, TextError> {
    let mut result = Vec::new();
    for element in input {
        budget.step()?;
        budget.push(
            &mut result,
            LegacyElement {
                order: element.value,
                low_utf16: element.forward_low,
                high_utf16: element.forward_high,
            },
        )?;
    }
    Ok(result)
}

fn affix(
    source: &str,
    pattern: &str,
    prefix: bool,
    budget: &mut TextBudget,
) -> Result<bool, TextError> {
    if pattern.is_empty() {
        return Ok(true);
    }
    let (source_len, source) = elements(source, budget)?;
    let (pattern_len, pattern) = elements(pattern, budget)?;
    // Each native loop moves at least one cursor; this upper bound is charged
    // before traversal. Never filter zero CEs ahead of SimpleAffix.
    budget.charge_work(
        source
            .len()
            .checked_add(pattern.len())
            .and_then(|n| n.checked_add(2))
            .ok_or(CeError::WorkLimit)?,
    )?;
    Ok(simple_affix(
        &source,
        source_len,
        &pattern,
        pattern_len,
        if prefix {
            Direction::Forward
        } else {
            Direction::Backward
        },
    )
    .matched)
}
pub(crate) fn map_prefix(
    source: &str,
    pattern: &str,
    budget: &mut TextBudget,
) -> Result<bool, TextError> {
    affix(source, pattern, true, budget)
}
pub(crate) fn map_suffix(
    source: &str,
    pattern: &str,
    budget: &mut TextBudget,
) -> Result<bool, TextError> {
    affix(source, pattern, false, budget)
}

pub(crate) fn map_first_match(
    source: &str,
    pattern: &str,
    budget: &mut TextBudget,
) -> Result<Option<SearchMatch>, TextError> {
    if pattern.is_empty() {
        return Ok(Some(SearchMatch {
            start_utf16: 0,
            limit_utf16: 0,
        }));
    }
    let (_, source_elements) = elements(source, budget)?;
    let (_, pattern_elements) = elements(pattern, budget)?;
    // Explicitly retain zero CE32 rows and native physical cursor boundaries.
    let source_elements = search_elements(&source_elements, budget)?;
    let pattern_elements = search_elements(&pattern_elements, budget)?;
    first_match_bounded(source, &source_elements, pattern, &pattern_elements, budget)
}

/// Translate a literal UTF-16 index without replacement characters or a second
/// temporary UTF-16 allocation. A linguistic match may consume fewer units than
/// kvSep.Length; reference Substring still advances by that literal length.
fn byte_at_utf16(text: &str, target: usize, budget: &mut TextBudget) -> Result<usize, TextError> {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units == target {
            return Ok(byte);
        }
        budget.step()?;
        units += ch.len_utf16();
        if units > target {
            return Err(TextError::UnsupportedUtf16Substring);
        }
    }
    if units == target {
        Ok(text.len())
    } else {
        Err(TextError::SubstringOutOfRange)
    }
}

pub(crate) fn map_entry_at_utf16_index(
    entry: &str,
    kv_separator: &str,
    index: usize,
    budget: &mut TextBudget,
) -> Result<(String, String), TextError> {
    let mut separator_len = 0usize;
    for _ in kv_separator.encode_utf16() {
        budget.step()?;
        separator_len += 1;
    }
    let end = index
        .checked_add(separator_len)
        .ok_or(TextError::SubstringOutOfRange)?;
    let key_end = byte_at_utf16(entry, index, budget)?;
    let value_start = byte_at_utf16(entry, end, budget)?;
    Ok((
        budget.copy(&entry[..key_end])?,
        budget.copy(&entry[value_start..])?,
    ))
}
