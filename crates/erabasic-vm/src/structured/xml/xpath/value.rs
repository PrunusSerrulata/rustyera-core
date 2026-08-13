//! Pure `XPath` comparison and conversion helpers.

use super::XPathComparison;

pub(super) fn xpath_name_matches(expected: &str, actual: &str) -> bool {
    expected == "*" || expected == actual
}

pub(super) fn xpath_string_comparison(
    comparison: XPathComparison,
    left: &str,
    right: &str,
) -> bool {
    match comparison {
        XPathComparison::Equal => left == right,
        XPathComparison::NotEqual => left != right,
        XPathComparison::Less
        | XPathComparison::LessOrEqual
        | XPathComparison::Greater
        | XPathComparison::GreaterOrEqual => false,
    }
}

pub(super) fn xpath_bool_comparison(comparison: XPathComparison, left: bool, right: bool) -> bool {
    match comparison {
        XPathComparison::Equal => left == right,
        XPathComparison::NotEqual => left != right,
        XPathComparison::Less
        | XPathComparison::LessOrEqual
        | XPathComparison::Greater
        | XPathComparison::GreaterOrEqual => false,
    }
}

pub(super) fn xpath_number_comparison(comparison: XPathComparison, left: f64, right: f64) -> bool {
    // XPath 1.0 specifies exact IEEE-754 equality, including NaN behavior.
    #[allow(clippy::float_cmp)]
    match comparison {
        XPathComparison::Equal => left == right,
        XPathComparison::NotEqual => left != right,
        XPathComparison::Less => left < right,
        XPathComparison::LessOrEqual => left <= right,
        XPathComparison::Greater => left > right,
        XPathComparison::GreaterOrEqual => left >= right,
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn xpath_context_number(value: usize) -> f64 {
    // DOM positions and sizes are XPath numbers; XPath 1.0 defines those as f64.
    value as f64
}

pub(super) fn xpath_parse_number(value: &str) -> f64 {
    value.trim().parse().unwrap_or(f64::NAN)
}
