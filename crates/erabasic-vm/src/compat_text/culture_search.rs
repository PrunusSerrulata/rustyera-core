//! ICU 72 exact forward search at tertiary, non-shifted strength, with the
//! explicit .NET breaker. The caller must supply the fixed provider's actual
//! legacy CE32 stream and iterator offsets, not sort keys or scalar spans.
use super::{TextBudget, TextError, search_boundaries::boundaries_bounded};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LegacyElement {
    pub order: u32,
    pub low_utf16: usize,
    pub high_utf16: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SearchMatch {
    pub start_utf16: usize,
    pub limit_utf16: usize,
}

type SearchError = TextError;

fn validate(elements: &[LegacyElement], text_length: usize) -> Result<(), SearchError> {
    let mut offset = 0;
    for element in elements {
        if element.order == u32::MAX
            || element.low_utf16 < offset
            || element.low_utf16 > element.high_utf16
            || element.high_utf16 > text_length
        {
            return Err(SearchError::InvalidElementOffsets);
        }
        offset = element.high_utf16;
    }
    Ok(())
}

/// `UCollationPCE::processCE` is injective over CE32 for tertiary/non-shifted
/// comparison: it spreads primary/secondary/tertiary into separate u16 lanes.
/// Consequently exact PCE equality is exactly CE32 equality, including the
/// 0xc0 continuation marker. Only zero orders are discarded.
fn relevant(
    elements: &[LegacyElement],
    budget: &mut TextBudget,
) -> Result<Vec<LegacyElement>, TextError> {
    let mut result = Vec::new();
    for element in elements {
        budget.step()?;
        if element.order != 0 {
            budget.push(&mut result, *element)?;
        }
    }
    Ok(result)
}

fn prefix_lengths(
    pattern: &[LegacyElement],
    budget: &mut TextBudget,
) -> Result<Vec<usize>, TextError> {
    let mut result = Vec::new();
    budget.reserve(&mut result, pattern.len())?;
    result.resize(pattern.len(), 0);
    for position in 1..pattern.len() {
        budget.step()?;
        let mut matched = result[position - 1];
        while matched > 0 && pattern[position].order != pattern[matched].order {
            budget.step()?;
            matched = result[matched - 1];
        }
        if pattern[position].order == pattern[matched].order {
            matched += 1;
        }
        result[position] = matched;
    }
    Ok(result)
}

fn character_match(
    target: &[LegacyElement],
    start: usize,
    count: usize,
    text_length: usize,
    breaks: &[usize],
) -> Option<SearchMatch> {
    let first = target[start];
    let last = target[start + count - 1];
    let next = target.get(start + count);
    if next.is_some_and(|element| element.low_utf16 == element.high_utf16)
        || first.low_utf16 == first.high_utf16
        || breaks.binary_search(&first.low_utf16).is_err()
    {
        return None;
    }
    let minimum = last.low_utf16;
    let maximum = next.map_or(text_length, |element| element.low_utf16);
    let mut limit = maximum;
    if minimum < maximum {
        if minimum == last.high_utf16 && breaks.binary_search(&minimum).is_ok() {
            limit = minimum;
        } else {
            let next_boundary = *breaks.get(breaks.partition_point(|value| *value <= minimum))?;
            if next_boundary >= last.high_utf16 {
                limit = next_boundary;
            }
        }
    }
    // .NET always installs an explicit breaker, so ICU's optional default-
    // breaker midcluster exception is never enabled for this entry point.
    if limit > maximum || breaks.binary_search(&limit).is_err() {
        return None;
    }
    Some(SearchMatch {
        start_utf16: first.low_utf16,
        limit_utf16: limit,
    })
}

/// First match, retaining UTF-16 boundaries. KMP enumerates the same ascending
/// CE-start candidates as ICU's exact loop without quadratic rescanning.
pub(super) fn first_match_bounded(
    text: &str,
    text_elements: &[LegacyElement],
    pattern: &str,
    pattern_elements: &[LegacyElement],
    budget: &mut TextBudget,
) -> Result<Option<SearchMatch>, SearchError> {
    budget.charge_work(
        text.len()
            .saturating_add(pattern.len())
            .saturating_add(text_elements.len())
            .saturating_add(pattern_elements.len()),
    )?;
    let length = text.encode_utf16().count();
    validate(text_elements, length)?;
    validate(pattern_elements, pattern.encode_utf16().count())?;
    let target = relevant(text_elements, budget)?;
    let pattern = relevant(pattern_elements, budget)?;
    // Managed empty-pattern handling and ICU's zero-CE pattern branch both
    // return the initial offset; empty source is .NET's equality special case.
    if pattern.is_empty() {
        return Ok(Some(SearchMatch {
            start_utf16: 0,
            limit_utf16: 0,
        }));
    }
    let failure = prefix_lengths(&pattern, budget)?;
    let breaks = boundaries_bounded(text, budget)?;
    let mut matched = 0;
    for (position, element) in target.iter().enumerate() {
        budget.step()?;
        while matched > 0 && element.order != pattern[matched].order {
            budget.step()?;
            matched = failure[matched - 1];
        }
        if element.order == pattern[matched].order {
            matched += 1;
        }
        if matched == pattern.len() {
            if let Some(found) =
                character_match(&target, position + 1 - matched, matched, length, &breaks)
            {
                return Ok(Some(found));
            }
            matched = failure[matched - 1];
        }
    }
    Ok(None)
}

#[cfg(test)]
fn first_match(
    text: &str,
    elements: &[LegacyElement],
    pattern: &str,
    pattern_elements: &[LegacyElement],
) -> Result<Option<SearchMatch>, TextError> {
    first_match_bounded(
        text,
        elements,
        pattern,
        pattern_elements,
        &mut TextBudget::new(1_000_000, 1_000_000),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(order: u32, low_utf16: usize, high_utf16: usize) -> LegacyElement {
        LegacyElement {
            order,
            low_utf16,
            high_utf16,
        }
    }

    #[test]
    fn search_kernel_rejects_partial_expansions_and_cluster_slices() {
        // Synthetic CE streams isolate search boundary rules; these values do
        // not claim to be the real collation weights of the example strings.
        let expanded = [element(1, 0, 1), element(2, 1, 1)];
        let a = [element(1, 0, 1)];
        let b = [element(2, 0, 1)];
        assert_eq!(first_match("æ", &expanded, "a", &a), Ok(None));
        assert_eq!(first_match("æ", &expanded, "b", &b), Ok(None));
        let ab = [element(1, 0, 1), element(2, 1, 2)];
        assert_eq!(
            first_match("æ", &expanded, "ab", &ab),
            Ok(Some(SearchMatch {
                start_utf16: 0,
                limit_utf16: 1
            }))
        );
        let combining = [element(1, 0, 1), element(2, 1, 2)];
        assert_eq!(first_match("a\u{301}", &combining, "a", &a), Ok(None));
        assert_eq!(first_match("a\u{301}", &combining, "\u{301}", &b), Ok(None));
    }

    #[test]
    fn search_kernel_skips_zero_weights_without_losing_source_offsets() {
        let target = [element(1, 0, 1), element(0, 1, 2), element(2, 2, 3)];
        assert_eq!(
            first_match("a\0b", &target, "b", &[element(2, 0, 1)]),
            Ok(Some(SearchMatch {
                start_utf16: 2,
                limit_utf16: 3
            }))
        );
        assert_eq!(
            first_match("a\0b", &target, "\0", &[element(0, 0, 1)]),
            Ok(Some(SearchMatch {
                start_utf16: 0,
                limit_utf16: 0
            }))
        );
        assert_eq!(
            first_match("", &[], "\0", &[element(0, 0, 1)]),
            Ok(Some(SearchMatch {
                start_utf16: 0,
                limit_utf16: 0
            }))
        );
        assert_eq!(
            first_match("a", &[element(1, 0, 2)], "a", &[element(1, 0, 1)]),
            Err(SearchError::InvalidElementOffsets)
        );
    }
}
