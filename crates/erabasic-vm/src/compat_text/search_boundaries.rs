//! The fixed .NET 8 ICU search breaker, using Unicode 15 property data.
//! Unlike default extended graphemes, CR and LF are separate boundaries here.
use super::{TextBudget, TextError, search_boundary_data as data};

const CONTROL: u8 = 1;
const CR: u8 = 2;
const EXTEND: u8 = 3;
const L: u8 = 4;
const LF: u8 = 5;
const LV: u8 = 6;
const LVT: u8 = 7;
const T: u8 = 8;
const V: u8 = 9;
const SPACING_MARK: u8 = 10;
const PREPEND: u8 = 11;
const REGIONAL: u8 = 12;
const ZWJ: u8 = 17;

fn in_ranges(value: u32, ranges: &[(u32, u32)]) -> bool {
    let position = ranges.partition_point(|(_, end)| *end < value);
    ranges
        .get(position)
        .is_some_and(|(start, _)| *start <= value)
}

fn grapheme_class(value: u32) -> u8 {
    let position = data::GCB.partition_point(|(_, end, _)| *end < value);
    data::GCB
        .get(position)
        .filter(|(start, _, _)| *start <= value)
        .map_or(0, |(_, _, class)| *class)
}

fn control(class: u8) -> bool {
    matches!(class, CONTROL | CR | LF)
}

#[derive(Default)]
#[allow(clippy::struct_excessive_bools)] // Each bit is an independent Unicode breaker state.
struct Context {
    previous: Option<u8>,
    regional_count: usize,
    pictograph_extends: bool,
    pictograph_zwj: bool,
    linking_consonant: bool,
    virama_seen: bool,
}

impl Context {
    fn boundary_before(&self, class: u8, pictograph: bool, consonant: bool) -> bool {
        let Some(previous) = self.previous else {
            return true;
        };
        if control(previous) || control(class) {
            return true;
        }
        if (previous == L && matches!(class, L | V | LV | LVT))
            || (matches!(previous, LV | V) && matches!(class, V | T))
            || (matches!(previous, LVT | T) && class == T)
            || matches!(class, EXTEND | ZWJ | SPACING_MARK)
            || previous == PREPEND
            || (consonant && self.linking_consonant && self.virama_seen)
            || (pictograph && self.pictograph_zwj)
            || (class == REGIONAL && previous == REGIONAL && self.regional_count % 2 == 1)
        {
            return false;
        }
        true
    }

    fn consume(&mut self, value: u32, class: u8, pictograph: bool, consonant: bool) {
        self.regional_count = if class == REGIONAL {
            self.regional_count + 1
        } else {
            0
        };
        self.pictograph_zwj = class == ZWJ && self.pictograph_extends;
        self.pictograph_extends = pictograph || (class == EXTEND && self.pictograph_extends);
        let linking_extension =
            class == ZWJ || (class == EXTEND && in_ranges(value, data::NONZERO_CCC));
        if consonant {
            self.linking_consonant = true;
            self.virama_seen = false;
        } else if linking_extension {
            self.virama_seen |= self.linking_consonant && in_ranges(value, data::VIRAMA);
        } else {
            self.linking_consonant = false;
            self.virama_seen = false;
        }
        self.previous = Some(class);
    }
}

/// Sorted UTF-16 offsets including start and end. No host locale is consulted.
pub(super) fn boundaries_bounded(
    text: &str,
    budget: &mut TextBudget,
) -> Result<Vec<usize>, TextError> {
    let mut result = Vec::new();
    let mut context = Context::default();
    let mut offset = 0;
    for character in text.chars() {
        budget.step()?;
        let value = u32::from(character);
        let class = grapheme_class(value);
        let pictograph = in_ranges(value, data::EXTENDED_PICTOGRAPHIC);
        let consonant = in_ranges(value, data::LINKING_CONSONANT);
        if context.boundary_before(class, pictograph, consonant) {
            budget.push(&mut result, offset)?;
        }
        context.consume(value, class, pictograph, consonant);
        offset += character.len_utf16();
    }
    budget.push(&mut result, offset)?;
    Ok(result)
}

#[cfg(test)]
fn boundaries(text: &str) -> Vec<usize> {
    boundaries_bounded(text, &mut TextBudget::new(1_000_000, 1_000_000)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::boundaries;

    #[test]
    fn fixed_breaker_retains_dotnet_controls_and_context_rules() {
        for (text, expected) in [
            ("", vec![0]),
            ("\r\n", vec![0, 1, 2]),
            ("a\u{301}b", vec![0, 2, 3]),
            ("\u{600}🇦🇧🇨🇩", vec![0, 5, 9]),
            ("🇦\u{301}🇧", vec![0, 3, 5]),
            ("👩\u{200d}💻", vec![0, 5]),
            ("\u{915}\u{94d}\u{915}", vec![0, 3]),
            ("\u{915}\u{94d}\u{93e}\u{915}", vec![0, 3, 4]),
        ] {
            assert_eq!(boundaries(text), expected, "{text:?}");
        }
    }
}
