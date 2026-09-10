//! Dense u16 opcode counters, touched only at an existing 1/1024 dispatch sample.
use serde::Serialize;

const OPCODE_SLOTS: usize = u16::MAX as usize + 1;

#[derive(Debug, Default)]
pub(super) struct OpcodeProfile {
    counts: Option<Box<[u64]>>,
    incomplete: bool,
    dropped_samples: u64,
}

impl OpcodeProfile {
    #[cfg(test)]
    pub(in crate::instruction_profile) fn allocated(&self) -> bool {
        self.counts.is_some()
    }
    pub(super) fn mark_incomplete(&mut self) {
        self.incomplete = true;
    }

    pub(super) fn sample(&mut self, opcode: u16) {
        let counts = self
            .counts
            .get_or_insert_with(|| vec![0; OPCODE_SLOTS].into_boxed_slice());
        let count = &mut counts[usize::from(opcode)];
        if let Some(next) = count.checked_add(1) {
            *count = next;
        } else {
            self.incomplete = true;
            self.dropped_samples = self.dropped_samples.saturating_add(1);
        }
    }

    pub(super) fn snapshot(&self) -> OpcodeSnapshot {
        OpcodeSnapshot {
            incomplete: self.incomplete,
            dropped_samples: self.dropped_samples.to_string(),
            counts: self
                .counts
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .enumerate()
                .filter(|(_, samples)| **samples != 0)
                .map(|(opcode, &samples)| OpcodeCount {
                    opcode: u16::try_from(opcode).expect("u16 opcode table"),
                    samples: samples.to_string(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OpcodeSnapshot {
    incomplete: bool,
    dropped_samples: String,
    counts: Vec<OpcodeCount>,
}

#[derive(Debug, Serialize)]
struct OpcodeCount {
    opcode: u16,
    samples: String,
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub(in crate::instruction_profile) fn maximum_snapshot() -> OpcodeSnapshot {
        OpcodeSnapshot {
            incomplete: true,
            dropped_samples: u64::MAX.to_string(),
            counts: (0..=u16::MAX)
                .map(|opcode| OpcodeCount {
                    opcode,
                    samples: u64::MAX.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn all_encoded_opcodes_are_bounded_and_overflow_is_explicit() {
        let mut profile = OpcodeProfile::default();
        assert!(!profile.allocated());
        assert!(profile.snapshot().counts.is_empty());
        assert!(!profile.allocated());
        profile.sample(0);
        let allocation = profile.counts.as_ref().unwrap().as_ptr();
        profile.counts.as_mut().unwrap()[0] = 0;
        for opcode in 0..=u16::MAX {
            profile.sample(opcode);
        }
        assert_eq!(profile.snapshot().counts.len(), OPCODE_SLOTS);
        assert_eq!(profile.counts.as_ref().unwrap().as_ptr(), allocation);
        assert!(
            profile
                .counts
                .as_ref()
                .unwrap()
                .iter()
                .all(|count| *count == 1)
        );
        profile.counts.as_mut().unwrap()[0] = u64::MAX;
        profile.sample(0);
        assert!(profile.incomplete);
        assert_eq!(profile.dropped_samples, 1);
        assert_eq!(profile.counts.as_ref().unwrap()[0], u64::MAX);
        assert_eq!(profile.counts.as_ref().unwrap()[usize::from(u16::MAX)], 1);
    }
}
