// Algorithm port: ICU release-72-1 collationiterator.cpp (251..684),
// utf16collationiterator.cpp (55..148), coleitr.cpp (71..137).
// Unicode/ICU license: licenses/ICU4X-LICENSE (ICU notice retained).
//! Root only, numeric OFF, normalization OFF; full canonical-closure data is
//! mandatory. No sort, NFD, replacement-scalar conversion or host OS calls.

use super::ce::{Ce64, CeError, CeLimits, CeStream, SourceUtf16Span, TextBudget};
use icu_collections::char16trie::{Char16TrieIterator, TrieResult};
use zerovec::ZeroSlice;

/// Implement only for the validated, fixed ICU72 *full closure* root dataset.
/// ICU4X testdata's root trie is NOT a valid implementation of this contract.
/// Contexts use ICU `UCharsTrie` encoding; modern Jamo indices are 0..67.
/// `ce32` takes scalar values AND lone surrogate code points (0..=0x10ffff).
/// No tailoring/fallback/base layer is accepted by this root-only dispatcher.
pub(crate) trait RawRootData {
    fn ce32(&self, cp: u32) -> Result<u32, CeError>;
    fn ce32_at(&self, index: usize) -> Result<u32, CeError>;
    fn ce_at(&self, index: usize) -> Result<u64, CeError>;
    fn contexts(&self) -> &ZeroSlice<u16>;
    fn jamo_ce32_at(&self, index: usize) -> Result<u32, CeError>;
    /// (canonical decomposition first CCC << 8) | last CCC. This metadata is
    /// used for contraction blocking even with normalization OFF.
    fn fcd16(&self, cp: u32) -> Result<u16, CeError>;
}

// Keep one validated immutable provider instance; wrappers need not copy its
// layout or tables merely to satisfy the internal generic API.
impl<T: RawRootData + ?Sized> RawRootData for &T {
    fn ce32(&self, cp: u32) -> Result<u32, CeError> {
        T::ce32(*self, cp)
    }
    fn ce32_at(&self, index: usize) -> Result<u32, CeError> {
        T::ce32_at(*self, index)
    }
    fn ce_at(&self, index: usize) -> Result<u64, CeError> {
        T::ce_at(*self, index)
    }
    fn contexts(&self) -> &ZeroSlice<u16> {
        T::contexts(*self)
    }
    fn jamo_ce32_at(&self, index: usize) -> Result<u32, CeError> {
        T::jamo_ce32_at(*self, index)
    }
    fn fcd16(&self, cp: u32) -> Result<u16, CeError> {
        T::fcd16(*self, cp)
    }
}

#[derive(Clone, Copy, Debug)]
struct Token {
    cp: u32,
    span: SourceUtf16Span,
}

struct Skipped<'a> {
    old: Vec<Token>,
    new: Vec<Token>,
    pos: usize,
    length_at_match: usize,
    trie_state: Option<Char16TrieIterator<'a>>,
}
impl Skipped<'_> {
    fn empty() -> Self {
        Self {
            old: Vec::new(),
            new: Vec::new(),
            pos: 0,
            length_at_match: 0,
            trie_state: None,
        }
    }
    fn active(&self) -> bool {
        !self.old.is_empty()
    }
    fn next(&mut self) -> Option<Token> {
        let token = *self.old.get(self.pos)?;
        self.pos += 1;
        Some(token)
    }
    fn backward(&mut self, n: usize) -> Result<usize, CeError> {
        let beyond = self.pos.saturating_sub(self.old.len());
        let normal = beyond.min(n);
        self.pos = self.pos.checked_sub(n).ok_or(CeError::MalformedProvider)?;
        Ok(normal)
    }
    fn first(&mut self, token: Token, budget: &mut TextBudget) -> Result<(), CeError> {
        self.new.clear();
        self.length_at_match = 0;
        budget.reserve(&mut self.new, 1)?;
        self.new.push(token);
        Ok(())
    }
    fn skip(&mut self, token: Token, budget: &mut TextBudget) -> Result<(), CeError> {
        budget.reserve(&mut self.new, 1)?;
        self.new.push(token);
        Ok(())
    }
    fn replace_match(&mut self, budget: &mut TextBudget) -> Result<(), CeError> {
        // UnicodeString::replace pins pos to length. This vector's positions
        // count code points rather than UTF-16 units; all moves count code
        // points, so the skipped-sequence state machine is unchanged.
        let consumed = self.pos.min(self.old.len());
        let new_len = self.old.len() - consumed + self.length_at_match;
        let mut next = Vec::new();
        budget.charge_work(new_len)?;
        budget.reserve(&mut next, new_len)?;
        next.extend_from_slice(&self.new[..self.length_at_match]);
        next.extend_from_slice(&self.old[consumed..]);
        self.old = next;
        self.pos = 0;
        Ok(())
    }
}

enum Contraction {
    Mapping { ce32: u32, origin: SourceUtf16Span },
    Buffered,
}

struct RawOff<'a, 'b, D: RawRootData> {
    data: &'a D,
    text: &'a [u16],
    pos: usize,
    skipped: Option<Skipped<'a>>,
    buffer: Vec<(u64, SourceUtf16Span)>,
    limits: CeLimits,
    total_ces: usize,
    budget: &'b mut TextBudget,
}

fn include(span: &mut SourceUtf16Span, other: SourceUtf16Span) {
    if other.start == other.end {
        return;
    }
    if span.start == span.end {
        *span = other;
        return;
    }
    span.start = span.start.min(other.start);
    span.end = span.end.max(other.end);
}
fn trie_next(
    trie: &mut Char16TrieIterator<'_>,
    cp: u32,
    budget: &mut TextBudget,
) -> Result<TrieResult, CeError> {
    budget.step()?;
    // Match the same code-point-to-UTF16 traversal as UCharsTrie::nextForCodePoint,
    // including lone surrogate units, without coercing to a Rust char.
    Ok(trie.next32(cp))
}
fn value(result: TrieResult) -> Option<u32> {
    match result {
        TrieResult::Intermediate(v) | TrieResult::FinalValue(v) => Some(v.cast_unsigned()),
        _ => None,
    }
}
fn has_next(result: TrieResult) -> bool {
    matches!(result, TrieResult::Intermediate(_) | TrieResult::NoValue)
}
fn simple_ce(ce32: u32) -> Result<u64, CeError> {
    if ce32 & 0xff < 0xc0 {
        Ok((u64::from(ce32 & 0xffff_0000) << 32)
            | (u64::from(ce32 & 0xff00) << 16)
            | (u64::from(ce32 & 0xff) << 8))
    } else {
        match ce32 & 0xf {
            1 => Ok((u64::from(ce32 & 0xffff_ff00) << 32) | 0x0500_0500),
            2 => Ok(u64::from(ce32 & 0xffff_ff00)),
            _ => Err(CeError::MalformedProvider),
        }
    }
}
fn implicit_ce(cp: u32) -> u64 {
    let mut c = cp + 1;
    let mut primary = 2 + (c % 18) * 14;
    c /= 18;
    primary |= (2 + c % 254) << 8;
    c /= 254;
    primary |= (4 + c % 251) << 16;
    primary |= 0xfe00_0000;
    (u64::from(primary) << 32) | 0x0500_0500
}

fn low_u32(value: i64) -> u32 {
    let bytes = value.to_le_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

impl<'a, D: RawRootData> RawOff<'a, '_, D> {
    fn next_cp(&mut self) -> Result<Option<Token>, CeError> {
        self.budget.step()?;
        let start = self.pos;
        let Some(&first) = self.text.get(self.pos) else {
            return Ok(None);
        };
        let first = u32::from(first);
        self.pos += 1;
        let cp = if (0xd800..=0xdbff).contains(&first) {
            if let Some(&trail) = self.text.get(self.pos) {
                if (0xdc00..=0xdfff).contains(&trail) {
                    self.pos += 1;
                    0x10000 + ((first - 0xd800) << 10) + u32::from(trail - 0xdc00)
                } else {
                    first
                }
            } else {
                first
            }
        } else {
            first
        };
        Ok(Some(Token {
            cp,
            span: SourceUtf16Span {
                start,
                end: self.pos,
            },
        }))
    }
    fn previous_cp(&mut self) -> Result<Option<Token>, CeError> {
        self.budget.step()?;
        if self.pos == 0 {
            return Ok(None);
        }
        let end = self.pos;
        self.pos -= 1;
        let last = u32::from(self.text[self.pos]);
        let cp = if (0xdc00..=0xdfff).contains(&last) && self.pos != 0 {
            let lead = u32::from(self.text[self.pos - 1]);
            if (0xd800..=0xdbff).contains(&lead) {
                self.pos -= 1;
                0x10000 + ((lead - 0xd800) << 10) + (last - 0xdc00)
            } else {
                last
            }
        } else {
            last
        };
        Ok(Some(Token {
            cp,
            span: SourceUtf16Span {
                start: self.pos,
                end,
            },
        }))
    }
    fn forward(&mut self, n: usize) -> Result<(), CeError> {
        for _ in 0..n {
            self.next_cp()?.ok_or(CeError::MalformedProvider)?;
        }
        Ok(())
    }
    fn backward(&mut self, n: usize) -> Result<(), CeError> {
        for _ in 0..n {
            self.previous_cp()?.ok_or(CeError::MalformedProvider)?;
        }
        Ok(())
    }
    fn next_skipped(&mut self) -> Result<Option<Token>, CeError> {
        self.budget.step()?;
        if let Some(skipped) = self.skipped.as_mut()
            && let Some(token) = skipped.next()
        {
            return Ok(Some(token));
        }
        let Some(token) = self.next_cp()? else {
            return Ok(None);
        };
        if let Some(skipped) = self.skipped.as_mut()
            && skipped.active()
        {
            skipped.pos += 1;
        }
        Ok(Some(token))
    }
    fn backward_skipped(&mut self, n: usize) -> Result<(), CeError> {
        self.budget.charge_work(n)?;
        let normal = match self.skipped.as_mut() {
            Some(skipped) if skipped.active() => skipped.backward(n)?,
            _ => n,
        };
        self.backward(normal)
    }
    fn context(&self, ce32: u32) -> Result<(u32, Char16TrieIterator<'a>), CeError> {
        let index = (ce32 >> 13) as usize;
        let contexts: &'a ZeroSlice<u16> = self.data.contexts();
        let first = contexts.get(index).ok_or(CeError::MalformedProvider)?;
        let second = contexts.get(index + 1).ok_or(CeError::MalformedProvider)?;
        let suffix = contexts
            .get_subslice(index + 2..contexts.len())
            .ok_or(CeError::MalformedProvider)?;
        Ok((
            (u32::from(first) << 16) | u32::from(second),
            Char16TrieIterator::new(suffix),
        ))
    }
    fn push(&mut self, ce: u64, origin: SourceUtf16Span) -> Result<(), CeError> {
        if self.total_ces == self.limits.ce64 {
            return Err(CeError::ElementLimit);
        }
        self.budget.step()?;
        self.budget.reserve(&mut self.buffer, 1)?;
        self.buffer.push((ce, origin));
        self.total_ces += 1;
        Ok(())
    }
    fn prefix(&mut self, ce32: u32) -> Result<u32, CeError> {
        // Exact source-position moves from appendCEsFromCE32 PREFIX_TAG.
        self.backward(1)?;
        let (mut result, mut trie) = self.context(ce32)?;
        let mut look_behind = 0;
        while let Some(token) = self.previous_cp()? {
            look_behind += 1;
            let matched = trie_next(&mut trie, token.cp, self.budget)?;
            if let Some(v) = value(matched) {
                result = v;
            }
            if !has_next(matched) {
                break;
            }
        }
        self.forward(look_behind + 1)?;
        Ok(result)
    }
    fn save_trie(&mut self, trie: &Char16TrieIterator<'a>) {
        if let Some(skipped) = self.skipped.as_mut() {
            skipped.trie_state = Some(trie.clone());
        }
    }
    fn active_skips(&self) -> bool {
        self.skipped.as_ref().is_some_and(Skipped::active)
    }

    fn contraction(
        &mut self,
        contraction: u32,
        origin: SourceUtf16Span,
        depth: usize,
    ) -> Result<Contraction, CeError> {
        let (mut ce32, mut suffixes) = self.context(contraction)?;
        let initial = suffixes.clone();
        let Some(mut token) = self.next_skipped()? else {
            return Ok(Contraction::Mapping { ce32, origin });
        };
        let mut look_ahead = 1usize;
        let mut since_match = 1usize;
        let mut matched_origin = origin;
        let mut attempted_origin = origin;
        if self.active_skips() {
            self.save_trie(&suffixes);
        }
        let mut matched = trie_next(&mut suffixes, token.cp, self.budget)?;
        loop {
            include(&mut attempted_origin, token.span);
            if let Some(v) = value(matched) {
                ce32 = v;
                matched_origin = attempted_origin;
                if !has_next(matched) {
                    return Ok(Contraction::Mapping {
                        ce32,
                        origin: matched_origin,
                    });
                }
                let Some(next) = self.next_skipped()? else {
                    return Ok(Contraction::Mapping {
                        ce32,
                        origin: matched_origin,
                    });
                };
                token = next;
                if self.active_skips() {
                    self.save_trie(&suffixes);
                }
                since_match = 1;
            } else {
                let next = if matches!(matched, TrieResult::NoMatch) {
                    None
                } else {
                    self.next_skipped()?
                };
                if let Some(next) = next {
                    token = next;
                    since_match += 1;
                } else {
                    if contraction & 0x400 != 0
                        && (contraction & 0x100 == 0 || since_match < look_ahead)
                    {
                        if since_match > 1 {
                            self.backward_skipped(since_match)?;
                            token = self.next_skipped()?.ok_or(CeError::MalformedProvider)?;
                            look_ahead -= since_match - 1;
                            since_match = 1;
                        }
                        if self.data.fcd16(token.cp)? > 0xff {
                            return self.discontiguous(
                                initial,
                                ce32,
                                matched_origin,
                                look_ahead,
                                token,
                                depth,
                            );
                        }
                    }
                    break;
                }
            }
            look_ahead += 1;
            matched = trie_next(&mut suffixes, token.cp, self.budget)?;
        }
        self.backward_skipped(since_match)?;
        Ok(Contraction::Mapping {
            ce32,
            origin: matched_origin,
        })
    }

    #[allow(clippy::too_many_lines)] // This is one resumable ICU contraction state machine.
    fn discontiguous(
        &mut self,
        initial: Char16TrieIterator<'a>,
        mut ce32: u32,
        mut origin: SourceUtf16Span,
        mut look_ahead: usize,
        first_skipped: Token,
        depth: usize,
    ) -> Result<Contraction, CeError> {
        let first_fcd = self.data.fcd16(first_skipped.cp)?;
        let Some(mut token) = self.next_skipped()? else {
            self.backward_skipped(1)?;
            return Ok(Contraction::Mapping { ce32, origin });
        };
        look_ahead += 1;
        let mut previous_cc =
            u8::try_from(first_fcd & 0xff).map_err(|_| CeError::MalformedProvider)?;
        let mut fcd = self.data.fcd16(token.cp)?;
        if fcd <= 0xff {
            self.backward_skipped(2)?;
            return Ok(Contraction::Mapping { ce32, origin });
        }
        let mut suffixes = if self.active_skips() {
            self.skipped
                .as_ref()
                .and_then(|s| s.trie_state.clone())
                .ok_or(CeError::MalformedProvider)?
        } else {
            if self.skipped.is_none() {
                self.skipped = Some(Skipped::empty());
            }
            let mut suffixes = initial;
            if look_ahead > 2 {
                self.backward(look_ahead)?;
                let replay = self.next_cp()?.ok_or(CeError::MalformedProvider)?;
                trie_next(&mut suffixes, replay.cp, self.budget)?;
                for _ in 3..look_ahead {
                    let replay = self.next_cp()?.ok_or(CeError::MalformedProvider)?;
                    trie_next(&mut suffixes, replay.cp, self.budget)?;
                }
                self.forward(2)?;
            }
            self.save_trie(&suffixes);
            suffixes
        };
        self.skipped
            .as_mut()
            .ok_or(CeError::MalformedProvider)?
            .first(first_skipped, self.budget)?;
        let mut since_match = 2usize;
        loop {
            let leading_cc = u8::try_from(fcd >> 8).map_err(|_| CeError::MalformedProvider)?;
            let matched = if previous_cc < leading_cc {
                trie_next(&mut suffixes, token.cp, self.budget)?
            } else {
                TrieResult::NoMatch
            };
            if let Some(v) = value(matched) {
                ce32 = v;
                include(&mut origin, token.span);
                since_match = 0;
                let skipped = self.skipped.as_mut().ok_or(CeError::MalformedProvider)?;
                skipped.length_at_match = skipped.new.len();
                if !has_next(matched) {
                    break;
                }
                self.save_trie(&suffixes);
            } else {
                let skipped = self.skipped.as_mut().ok_or(CeError::MalformedProvider)?;
                skipped.skip(token, self.budget)?;
                suffixes = skipped
                    .trie_state
                    .clone()
                    .ok_or(CeError::MalformedProvider)?;
                previous_cc = u8::try_from(fcd & 0xff).map_err(|_| CeError::MalformedProvider)?;
            }
            let Some(next) = self.next_skipped()? else {
                break;
            };
            token = next;
            since_match += 1;
            fcd = self.data.fcd16(token.cp)?;
            if fcd <= 0xff {
                break;
            }
        }
        self.backward_skipped(since_match)?;
        let top = !self.active_skips();
        self.skipped
            .as_mut()
            .ok_or(CeError::MalformedProvider)?
            .replace_match(self.budget)?;
        if top && self.active_skips() {
            self.append(None, ce32, origin, depth + 1)?;
            loop {
                let next = self.skipped.as_mut().and_then(Skipped::next);
                let Some(next) = next else {
                    break;
                };
                let mapping = self.data.ce32(next.cp)?;
                self.append(Some(next.cp), mapping, next.span, depth + 1)?;
            }
            if let Some(skipped) = self.skipped.as_mut() {
                skipped.old.clear();
                skipped.pos = 0;
            }
            Ok(Contraction::Buffered)
        } else {
            Ok(Contraction::Mapping { ce32, origin })
        }
    }

    #[allow(clippy::too_many_lines)] // The tag dispatcher mirrors one cohesive ICU state machine.
    fn append(
        &mut self,
        cp: Option<u32>,
        mut ce32: u32,
        mut origin: SourceUtf16Span,
        depth: usize,
    ) -> Result<(), CeError> {
        if depth > self.limits.context_depth {
            return Err(CeError::ContextLimit);
        }
        // A root data mapping cannot legitimately cycle among indirect CE32s.
        for _ in 0..64 {
            self.budget.step()?;
            if ce32 & 0xff < 0xc0 {
                return self.push(simple_ce(ce32)?, origin);
            }
            let index = (ce32 >> 13) as usize;
            match ce32 & 0xf {
                1 | 2 => return self.push(simple_ce(ce32)?, origin),
                4 => {
                    let first = (u64::from(ce32 & 0xff00_0000) << 32)
                        | 0x0500_0000
                        | u64::from((ce32 & 0x00ff_0000) >> 8);
                    let second = (u64::from(ce32 & 0xff00) << 16) | 0x0500;
                    self.push(first, origin)?;
                    return self.push(second, origin);
                }
                5 | 6 => {
                    let length = usize::try_from((ce32 >> 8) & 31)
                        .map_err(|_| CeError::MalformedProvider)?;
                    if length == 0 {
                        return Err(CeError::MalformedProvider);
                    }
                    for offset in 0..length {
                        let value = if ce32 & 0xf == 5 {
                            simple_ce(self.data.ce32_at(index + offset)?)?
                        } else {
                            self.data.ce_at(index + offset)?
                        };
                        self.push(value, origin)?;
                    }
                    return Ok(());
                }
                8 => ce32 = self.prefix(ce32)?,
                9 => match self.contraction(ce32, origin, depth + 1)? {
                    Contraction::Mapping {
                        ce32: next,
                        origin: next_origin,
                    } => {
                        ce32 = next;
                        origin = next_origin;
                    }
                    Contraction::Buffered => return Ok(()),
                },
                10 => ce32 = self.data.ce32_at(index)?, // numeric OFF
                11 => ce32 = self.data.ce32_at(0)?, // explicit-length input: embedded NUL is not EOF
                12 => {
                    let syllable = cp
                        .ok_or(CeError::MalformedProvider)?
                        .checked_sub(0xac00)
                        .filter(|v| *v < 11172)
                        .ok_or(CeError::MalformedProvider)?;
                    let t = syllable % 28;
                    let v = (syllable / 28) % 21;
                    let l = syllable / (28 * 21);
                    self.append(None, self.data.jamo_ce32_at(l as usize)?, origin, depth + 1)?;
                    self.append(
                        None,
                        self.data.jamo_ce32_at((19 + v) as usize)?,
                        origin,
                        depth + 1,
                    )?;
                    if t != 0 {
                        self.append(
                            None,
                            self.data.jamo_ce32_at((39 + t) as usize)?,
                            origin,
                            depth + 1,
                        )?;
                    }
                    return Ok(());
                }
                13 => {
                    let cp = cp.ok_or(CeError::MalformedProvider)?;
                    if !(0xd800..=0xdbff).contains(&cp) {
                        return Err(CeError::MalformedProvider);
                    }
                    // next_cp already combines valid pairs; only a lone lead reaches here.
                    return self.push(implicit_ce(cp), origin);
                }
                14 => {
                    let cp = cp.ok_or(CeError::MalformedProvider)?;
                    let bytes = self.data.ce_at(index)?.to_be_bytes();
                    let p = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                    let lower = i32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                    let mut offset =
                        (i64::from(cp) - i64::from(lower >> 8)) * i64::from(lower & 0x7f);
                    offset += i64::from((p >> 8) & 0xff) - 2;
                    let mut primary = low_u32(offset % 254 + 2) << 8;
                    offset /= 254;
                    if lower & 0x80 != 0 {
                        offset += i64::from((p >> 16) & 0xff) - 4;
                        primary |= low_u32(offset % 251 + 4) << 16;
                        offset /= 251;
                    } else {
                        offset += i64::from((p >> 16) & 0xff) - 2;
                        primary |= low_u32(offset % 254 + 2) << 16;
                        offset /= 254;
                    }
                    primary |= (p & 0xff00_0000).wrapping_add(low_u32(offset) << 24);
                    return self.push((u64::from(primary) << 32) | 0x0500_0500, origin);
                }
                15 => return self.push(implicit_ce(cp.ok_or(CeError::MalformedProvider)?), origin),
                _ => return Err(CeError::MalformedProvider),
            }
        }
        Err(CeError::MalformedProvider)
    }
}

/// Produces the actual OFF-dispatch source cursor at each nextCE boundary.
/// Unlike the NFD prototype, offsets are captured from physical UTF-16 pos after
/// the complete append operation, including accepted contractions and skipped
/// mark recursion. Lookahead backtracking happens before high is recorded.
/// No generic ICU4X provider may be supplied: callers must use the frozen raw
/// provider implementing the contract above.
#[cfg(test)]
pub(crate) fn raw_off_elements<D: RawRootData>(
    data: &D,
    text: &[u16],
    limits: CeLimits,
) -> Result<CeStream, CeError> {
    let mut budget = TextBudget::new(
        limits
            .utf16_units
            .saturating_add(limits.ce64)
            .saturating_mul(128),
        usize::MAX,
    );
    raw_off_elements_bounded(data, text, limits, &mut budget)
}

pub(crate) fn raw_off_elements_bounded<D: RawRootData>(
    data: &D,
    text: &[u16],
    limits: CeLimits,
    budget: &mut TextBudget,
) -> Result<CeStream, CeError> {
    if text.len() > limits.utf16_units || text.len() > i32::MAX as usize {
        return Err(CeError::InputLimit);
    }
    let mut state = RawOff {
        data,
        text,
        pos: 0,
        skipped: None,
        buffer: Vec::new(),
        limits,
        total_ces: 0,
        budget,
    };
    let mut elements = Vec::new();
    while state.pos < text.len() {
        let low = state.pos;
        let token = state.next_cp()?.ok_or(CeError::MalformedProvider)?;
        let ce32 = data.ce32(token.cp)?;
        state.append(Some(token.cp), ce32, token.span, 0)?;
        let high = state.pos;
        if high < low || state.buffer.is_empty() {
            return Err(CeError::MalformedProvider);
        }
        state.budget.reserve(&mut elements, state.buffer.len())?;
        for (index, (value, source)) in state.buffer.drain(..).enumerate() {
            elements.push(Ce64 {
                value,
                source,
                forward_low: if index == 0 { low } else { high },
                forward_high: high,
            });
        }
    }
    Ok(CeStream { elements })
}
