// ICU72 CE records and legacy splitting. See licenses/ICU4X-LICENSE.
//! Fixed ICU72 raw CE stream. The OFF dispatcher records physical UTF-16
//! cursor boundaries; the rejected ICU4X/NFD prototype must not construct this
//! production stream. Runtime/oracle parity is pending validation.

/// Half-open offsets in the original, well-formed UTF-16 input.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceUtf16Span {
    pub start: usize,
    pub end: usize,
}

/// A CE64 emitted by the fixed ICU72 normalization-OFF dispatcher.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ce64 {
    pub value: u64,
    /// Enclosing source range of the accepted original scalar(s). A
    /// discontiguous contraction can enclose skipped marks; this is not a slice
    /// whose characters all necessarily contributed to this CE.
    pub source: SourceUtf16Span,
    pub forward_low: usize,
    pub forward_high: usize,
}

/// An old-style `ucol_next`/`ucol_previous` weight, not an ICU data-table CE32.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LegacyCe32 {
    pub value: u32,
    pub ce64_index: usize,
    pub continuation_half: bool,
    pub source: SourceUtf16Span,
    pub forward_low: usize,
    pub forward_high: usize,
}

/// Explicit capacity limits are caller policy, not a Unicode semantic fallback.
#[derive(Clone, Copy, Debug)]
pub struct CeLimits {
    pub utf16_units: usize,
    pub ce64: usize,
    pub context_depth: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CeError {
    InputLimit,
    WorkLimit,
    ByteLimit,
    ElementLimit,
    ContextLimit,
    Allocation,
    MalformedProvider,
}

#[derive(Clone, Debug)]
pub struct CeStream {
    pub elements: Vec<Ce64>,
}

impl CeStream {
    /// Splitting is from ICU release-72-1 `coleitr.cpp`, getFirstHalf /
    /// getSecondHalf / next. Quaternary bits are deliberately not exported.
    #[cfg(test)]
    pub fn legacy_elements(&self) -> Result<Vec<LegacyCe32>, CeError> {
        let capacity = self
            .elements
            .len()
            .checked_mul(2)
            .ok_or(CeError::ElementLimit)?;
        let mut result = Vec::new();
        result
            .try_reserve(capacity)
            .map_err(|_| CeError::Allocation)?;
        for (ce64_index, ce) in self.elements.iter().enumerate() {
            let primary = u32::try_from(ce.value >> 32).map_err(|_| CeError::MalformedProvider)?;
            let bytes = ce.value.to_le_bytes();
            let lower = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let first = (primary & 0xffff_0000) | ((lower >> 16) & 0xff00) | ((lower >> 8) & 0xff);
            let second = (primary << 16) | ((lower >> 8) & 0xff00) | (lower & 0x3f);
            result.push(LegacyCe32 {
                value: first,
                ce64_index,
                continuation_half: false,
                source: ce.source,
                forward_low: ce.forward_low,
                forward_high: ce.forward_high,
            });
            if second != 0 {
                result.push(LegacyCe32 {
                    value: second | 0xc0,
                    ce64_index,
                    continuation_half: true,
                    source: ce.source,
                    forward_low: ce.forward_high,
                    forward_high: ce.forward_high,
                });
            }
        }
        Ok(result)
    }
}

/// Private, cumulative budget for one synchronous MAP operation. The VM supplies
/// existing operand and snapshot limits; no script/provider can raise them.
/// Allocation charges are monotonic across temporary buffers and entry commits.
pub(crate) struct TextBudget {
    remaining_work: usize,
    remaining_bytes: usize,
}
impl TextBudget {
    pub(crate) const fn new(work: usize, bytes: usize) -> Self {
        Self {
            remaining_work: work,
            remaining_bytes: bytes,
        }
    }
    pub(crate) const fn remaining_work(&self) -> usize {
        self.remaining_work
    }
    pub(crate) fn step(&mut self) -> Result<(), CeError> {
        self.charge_work(1)
    }
    pub(crate) fn charge_work(&mut self, amount: usize) -> Result<(), CeError> {
        self.remaining_work = self
            .remaining_work
            .checked_sub(amount)
            .ok_or(CeError::WorkLimit)?;
        Ok(())
    }
    fn charge_bytes(&mut self, amount: usize) -> Result<(), CeError> {
        self.remaining_bytes = self
            .remaining_bytes
            .checked_sub(amount)
            .ok_or(CeError::ByteLimit)?;
        Ok(())
    }
    pub(crate) fn reserve<T>(
        &mut self,
        values: &mut Vec<T>,
        additional: usize,
    ) -> Result<(), CeError> {
        let required = values
            .len()
            .checked_add(additional)
            .ok_or(CeError::ByteLimit)?;
        if required <= values.capacity() {
            return Ok(());
        }
        let target = required.max(values.capacity().saturating_mul(2));
        let extra = target
            .checked_sub(values.capacity())
            .ok_or(CeError::ByteLimit)?;
        self.charge_bytes(
            extra
                .checked_mul(core::mem::size_of::<T>())
                .ok_or(CeError::ByteLimit)?,
        )?;
        values
            .try_reserve_exact(target - values.len())
            .map_err(|_| CeError::Allocation)
    }
    pub(crate) fn push<T>(&mut self, values: &mut Vec<T>, value: T) -> Result<(), CeError> {
        self.reserve(values, 1)?;
        values.push(value);
        Ok(())
    }
    pub(crate) fn append(&mut self, output: &mut String, value: &str) -> Result<(), CeError> {
        self.charge_work(value.len())?;
        let required = output
            .len()
            .checked_add(value.len())
            .ok_or(CeError::ByteLimit)?;
        if required > output.capacity() {
            let target = required.max(output.capacity().saturating_mul(2));
            self.charge_bytes(target - output.capacity())?;
            output
                .try_reserve_exact(target - output.len())
                .map_err(|_| CeError::Allocation)?;
        }
        output.push_str(value);
        Ok(())
    }
    pub(crate) fn copy(&mut self, value: &str) -> Result<String, CeError> {
        let mut output = String::new();
        self.append(&mut output, value)?;
        Ok(output)
    }
}

impl CeStream {
    pub(crate) fn legacy_elements_bounded(
        &self,
        budget: &mut TextBudget,
    ) -> Result<Vec<LegacyCe32>, CeError> {
        let mut result = Vec::new();
        for (ce64_index, ce) in self.elements.iter().enumerate() {
            budget.step()?;
            let primary = (ce.value >> 32) as u32;
            let bytes = ce.value.to_le_bytes();
            let lower = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let first = (primary & 0xffff_0000) | ((lower >> 16) & 0xff00) | ((lower >> 8) & 0xff);
            let second = (primary << 16) | ((lower >> 8) & 0xff00) | (lower & 0x3f);
            budget.push(
                &mut result,
                LegacyCe32 {
                    value: first,
                    ce64_index,
                    continuation_half: false,
                    source: ce.source,
                    forward_low: ce.forward_low,
                    forward_high: ce.forward_high,
                },
            )?;
            if second != 0 {
                budget.step()?;
                budget.push(
                    &mut result,
                    LegacyCe32 {
                        value: second | 0xc0,
                        ce64_index,
                        continuation_half: true,
                        source: ce.source,
                        forward_low: ce.forward_high,
                        forward_high: ce.forward_high,
                    },
                )?;
            }
        }
        Ok(result)
    }
}
