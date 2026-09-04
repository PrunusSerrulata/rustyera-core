//! Bounded bit work over an already captured backing. The VM scheduler, not a
//! second evaluator, owns calls and services between capture and this operation.
use super::array_leases::ArrayLeaseId;
use crate::{Fiber, Vm, VmError};
use erabasic_bytecode::{BitCallSpec, BitOperation};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum BitWork {
    Done(i64),
    Get {
        word: usize,
        bit: u32,
    },
    Toggle {
        word: usize,
        bit: u32,
    },
    Set {
        next_word: usize,
        start: u64,
        end: u64,
        value: bool,
    },
    Find {
        next_word: usize,
        length: usize,
        value: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct PendingBitCall {
    pub begin: usize,
    pub stack_index: usize,
    pub spec: BitCallSpec,
    pub lease: ArrayLeaseId,
    pub work: Option<BitWork>,
}

pub(crate) enum BitProgress {
    Complete(i64),
    Continue,
}

fn invalid(message: &str) -> VmError {
    VmError::InvalidState(message.into())
}

fn bad_argument(message: &str) -> VmError {
    VmError::ScriptFailure(crate::ExecutionFailure::script(
        crate::ScriptFaultKind::Argument,
        crate::VmFaultCode::Native,
        message,
    ))
}

impl BitWork {
    /// Only already evaluated present values are supplied. No sentinel integer
    /// represents omission, and no array read happens during this construction.
    pub(crate) fn new(
        spec: BitCallSpec,
        present_values: &[i64],
        words: usize,
    ) -> Result<Self, VmError> {
        if present_values.len() != spec.evaluated_arguments() {
            return Err(VmError::InvalidState(
                "bit-call value count differs from its validated spec".into(),
            ));
        }
        let mut values = present_values.iter().copied();
        let mut args = [None; 3];
        for (index, argument) in args
            .iter_mut()
            .enumerate()
            .take(usize::from(spec.tail_count))
        {
            if spec.present & (1 << index) != 0 {
                *argument = values.next();
            }
        }
        let capacity = u64::try_from(words)
            .ok()
            .and_then(|words| words.checked_mul(64))
            .filter(|capacity| i64::try_from(*capacity).is_ok())
            .ok_or_else(|| {
                VmError::ScriptFailure(crate::ExecutionFailure::new(
                    crate::VmFaultCode::ResourceLimit,
                    "bit-array capacity is not representable",
                ))
            })?;
        if spec.operation == BitOperation::IndexOfFirst {
            return Ok(Self::Find {
                next_word: 0,
                length: words,
                value: args[0].unwrap_or(0) != 0,
            });
        }
        let index = args[0].ok_or_else(|| bad_argument("bit index argument is omitted"))?;
        match spec.operation {
            BitOperation::Get | BitOperation::Toggle => {
                let Some(index) = u64::try_from(index).ok().filter(|index| *index < capacity)
                else {
                    return Ok(Self::Done(if spec.operation == BitOperation::Get {
                        -1
                    } else {
                        0
                    }));
                };
                let word = usize::try_from(index / 64).expect("capacity was bounded by words");
                let bit = (index % 64) as u32;
                Ok(if spec.operation == BitOperation::Get {
                    Self::Get { word, bit }
                } else {
                    Self::Toggle { word, bit }
                })
            }
            BitOperation::Set => {
                let length = args[2].unwrap_or(1);
                if length <= 0 {
                    return Ok(Self::Done(1));
                }
                if index < -1 || index == -1 && words == 0 {
                    return Err(VmError::ScriptFailure(crate::ExecutionFailure::script(
                        crate::ScriptFaultKind::Bounds,
                        crate::VmFaultCode::Native,
                        "bit set starts outside its backing",
                    )));
                }
                // Fixed BIT[0] is the zero mask. At -1 the first iteration
                // reads word zero without changing it; subsequent bits start at 0.
                let start = u64::try_from(index).unwrap_or(0);
                let count = length.cast_unsigned() - u64::from(index == -1);
                let end = start.saturating_add(count).min(capacity);
                if start >= end {
                    return Ok(Self::Done(1));
                }
                Ok(Self::Set {
                    next_word: usize::try_from(start / 64).expect("within capacity"),
                    start,
                    end,
                    value: args[1].unwrap_or(1) != 0,
                })
            }
            BitOperation::IndexOfFirst => unreachable!("handled before index argument"),
        }
    }

    /// Validate every immutable range against the original scalar operands.
    /// A saved cursor may only move forward within that range; no underflow or
    /// forged detached length may turn a resumed chunk into arbitrary writes.
    pub(crate) fn valid_for(&self, spec: BitCallSpec, values: &[i64], words: usize) -> bool {
        let Ok(initial) = Self::new(spec, values, words) else {
            return false;
        };
        match (self, initial) {
            (
                Self::Set {
                    next_word,
                    start,
                    end,
                    value,
                },
                Self::Set {
                    next_word: first,
                    start: a,
                    end: b,
                    value: v,
                },
            ) => {
                *start == a
                    && *end == b
                    && *value == v
                    && *next_word >= first
                    && (*next_word as u128) * 64 < u128::from(*end)
            }
            (
                Self::Find {
                    next_word,
                    length,
                    value,
                },
                Self::Find {
                    length: n,
                    value: v,
                    ..
                },
            ) => *length == n && *value == v && *next_word <= n,
            (actual, initial) => *actual == initial,
        }
    }

    pub(crate) fn has_progress(&self, spec: BitCallSpec, values: &[i64], words: usize) -> bool {
        if !self.valid_for(spec, values, words) {
            return false;
        }
        match (self, Self::new(spec, values, words)) {
            (
                Self::Set { next_word, .. },
                Ok(Self::Set {
                    next_word: first, ..
                }),
            ) => *next_word > first,
            (
                Self::Find {
                    next_word, length, ..
                },
                _,
            ) => *next_word > 0 && *next_word < *length,
            _ => false,
        }
    }

    pub(crate) fn advance(
        &mut self,
        vm: &mut Vm,
        fiber: &mut Fiber,
        lease: ArrayLeaseId,
        limit: usize,
    ) -> Result<(BitProgress, usize), VmError> {
        let quantum = limit.clamp(1, 256);
        match self {
            Self::Done(value) => Ok((BitProgress::Complete(*value), 0)),
            Self::Get { word, bit } => {
                let bits =
                    u64::from_ne_bytes(vm.bit_array_word(fiber, lease, *word)?.to_ne_bytes());
                Ok((BitProgress::Complete(i64::from(bits & (1 << *bit) != 0)), 1))
            }
            Self::Toggle { word, bit } => {
                let old = u64::from_ne_bytes(vm.bit_array_word(fiber, lease, *word)?.to_ne_bytes());
                vm.commit_bit_words(
                    fiber,
                    lease,
                    &[(*word, i64::from_ne_bytes((old ^ (1 << *bit)).to_ne_bytes()))],
                )?;
                Ok((BitProgress::Complete(1), 1))
            }
            Self::Set {
                next_word,
                start,
                end,
                value,
            } => {
                let last = usize::try_from((*end - 1) / 64).expect("validated bit range");
                let limit = next_word.saturating_add(quantum).min(last + 1);
                let mut updates = Vec::with_capacity(limit - *next_word);
                for word in *next_word..limit {
                    let old =
                        u64::from_ne_bytes(vm.bit_array_word(fiber, lease, word)?.to_ne_bytes());
                    let word_start = word as u64 * 64;
                    let low = u32::try_from(start.saturating_sub(word_start))
                        .map_err(|_| invalid("bit offset exceeds one word"))?;
                    let high = u32::try_from((*end - word_start).min(64))
                        .map_err(|_| invalid("bit offset exceeds one word"))?;
                    let mask = (u64::MAX >> (64 - (high - low))) << low;
                    let new = if *value { old | mask } else { old & !mask };
                    updates.push((word, i64::from_ne_bytes(new.to_ne_bytes())));
                }
                vm.commit_bit_words(fiber, lease, &updates)?;
                *next_word = limit;
                Ok((
                    if limit > last {
                        BitProgress::Complete(1)
                    } else {
                        BitProgress::Continue
                    },
                    updates.len(),
                ))
            }
            Self::Find {
                next_word,
                length,
                value,
            } => {
                let limit = next_word.saturating_add(quantum).min(*length);
                let visited = limit - *next_word;
                for word in *next_word..limit {
                    let bits =
                        u64::from_ne_bytes(vm.bit_array_word(fiber, lease, word)?.to_ne_bytes());
                    let candidates = if *value { bits } else { !bits };
                    if candidates != 0 {
                        return Ok((
                            BitProgress::Complete(
                                i64::try_from(
                                    word as u64 * 64 + u64::from(candidates.trailing_zeros()),
                                )
                                .expect("capacity validated before scanning"),
                            ),
                            word - *next_word + 1,
                        ));
                    }
                }
                *next_word = limit;
                Ok((
                    if limit == *length {
                        BitProgress::Complete(-1)
                    } else {
                        BitProgress::Continue
                    },
                    visited,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn set_spec() -> BitCallSpec {
        BitCallSpec {
            operation: BitOperation::Set,
            input: erabasic_bytecode::SymbolKey::default(),
            tail_count: 3,
            present: 7,
        }
    }
    #[test]
    fn saved_bit_work_rejects_changed_operands_ranges_and_repeated_initial_chunk() {
        let spec = set_spec();
        let initial = BitWork::new(spec, &[63, 1, 40000], 1000).unwrap();
        assert!(!initial.has_progress(spec, &[63, 1, 40000], 1000));
        let progress = BitWork::Set {
            next_word: 256,
            start: 63,
            end: 40063,
            value: true,
        };
        assert!(progress.has_progress(spec, &[63, 1, 40000], 1000));
        assert!(!progress.has_progress(spec, &[64, 1, 40000], 1000));
        assert!(!progress.has_progress(spec, &[63, 0, 40000], 1000));
        assert!(!progress.has_progress(spec, &[63, 1, 40000], 10));
        let backwards = BitWork::Set {
            next_word: 0,
            start: 63,
            end: 40063,
            value: true,
        };
        assert!(!backwards.has_progress(spec, &[63, 1, 40000], 1000));
        let past_end = BitWork::Set {
            next_word: 626,
            start: 63,
            end: 40063,
            value: true,
        };
        assert!(!past_end.valid_for(spec, &[63, 1, 40000], 1000));
    }
    #[test]
    fn bit_negative_set_special_case_and_missing_get_stay_distinct() {
        assert_eq!(
            BitWork::new(set_spec(), &[-2, 1, 0], 0).unwrap(),
            BitWork::Done(1)
        );
        assert!(BitWork::new(set_spec(), &[-1, 1, 1], 0).is_err());
        assert_eq!(
            BitWork::new(set_spec(), &[-1, 1, 1], 1).unwrap(),
            BitWork::Done(1)
        );
        let missing = BitCallSpec {
            operation: BitOperation::Get,
            input: erabasic_bytecode::SymbolKey::default(),
            tail_count: 0,
            present: 0,
        };
        assert!(BitWork::new(missing, &[], 1).is_err());
    }
}
