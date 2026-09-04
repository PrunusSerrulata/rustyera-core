// Algorithm port: dotnet/runtime v8.0.28 pal_collation.c, SimpleAffix_Iterators.
// Copyright (c) .NET Foundation and Contributors; MIT license in DOTNET-LICENSE.TXT.

use super::ce::LegacyCe32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Direction {
    Forward,
    Backward,
}

/// Diagnostic trace for the CE candidate. `captured_offset` is a candidate source
/// cursor; it must not be published as a proven ICU native matched length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AffixTrace {
    pub matched: bool,
    pub captured_offset: usize,
    pub pattern_steps: usize,
    pub source_steps: usize,
}

struct Cursor<'a> {
    values: &'a [LegacyCe32],
    position: usize,
    offset: usize,
    direction: Direction,
}

impl<'a> Cursor<'a> {
    fn new(values: &'a [LegacyCe32], utf16_len: usize, direction: Direction) -> Self {
        Self {
            values,
            position: 0,
            offset: match direction {
                Direction::Forward => 0,
                Direction::Backward => utf16_len,
            },
            direction,
        }
    }
    fn next(&mut self) -> Option<u32> {
        let index = match self.direction {
            Direction::Forward => self.position,
            Direction::Backward => self.values.len().checked_sub(self.position + 1)?,
        };
        let value = self.values.get(index)?;
        self.position += 1;
        self.offset = match self.direction {
            Direction::Forward => value.forward_high,
            Direction::Backward => value.forward_low,
        };
        Some(value.value)
    }
}

/// CompareOptions.None / tertiary only. Zero CEs must remain in the stream:
/// filtering them first changes the prefix boundary combining-element rule.
/// Suffix walks the complete forward-produced CE sequence backwards, including
/// continuation halves. It never reverses source Unicode scalars.
pub(crate) fn simple_affix(
    source: &[LegacyCe32],
    source_utf16_len: usize,
    pattern: &[LegacyCe32],
    pattern_utf16_len: usize,
    direction: Direction,
) -> AffixTrace {
    let mut source = Cursor::new(source, source_utf16_len, direction);
    let mut pattern = Cursor::new(pattern, pattern_utf16_len, direction);
    let mut move_source = true;
    let mut move_pattern = true;
    let mut source_element = Some(0);
    let mut pattern_element = Some(0);
    let mut captured_offset = 0;
    let matched = loop {
        if move_pattern {
            pattern_element = pattern.next();
        }
        if move_source {
            captured_offset = source.offset;
            source_element = source.next();
        }
        move_source = true;
        move_pattern = true;
        match (pattern_element, source_element) {
            (None, None | Some(0)) => break true,
            (None, Some(source_ce)) => {
                if direction == Direction::Forward
                    && source_ce & 0xffff_0000 == 0
                    && source_ce & 0x0000_ff00 != 0
                {
                    break false;
                }
                break true;
            }
            (Some(0), _) => move_source = false,
            (_, Some(0)) => move_pattern = false,
            (Some(pattern_ce), Some(source_ce)) if pattern_ce == source_ce => {}
            _ => break false,
        }
    };
    AffixTrace {
        matched,
        captured_offset,
        pattern_steps: pattern.position,
        source_steps: source.position,
    }
}
