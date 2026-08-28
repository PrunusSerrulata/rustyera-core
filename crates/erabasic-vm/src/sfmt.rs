//! Reference-compatible SFMT-19937 state used by `EraBasic` RAND.

pub(crate) const STATE_WORDS: usize = 624;

#[derive(Clone)]
pub(crate) struct Sfmt19937 {
    state: [u32; STATE_WORDS],
    index: usize,
}

impl Sfmt19937 {
    pub(crate) fn new(seed: u64) -> Self {
        let mut result = Self {
            state: [0; STATE_WORDS],
            index: STATE_WORDS,
        };
        // Emuera casts the signed long seed to uint in an unchecked context.
        result.state[0] = u32::from_le_bytes(seed.to_le_bytes()[..4].try_into().expect("low word"));
        for index in 1..STATE_WORDS {
            let previous = result.state[index - 1];
            result.state[index] = 1_812_433_253_u32
                .wrapping_mul(previous ^ (previous >> 30))
                .wrapping_add(u32::try_from(index).expect("SFMT state index fits u32"));
        }
        result.certify_period();
        result
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        // MTRandom.NextUInt64 consumes the high word first.
        (u64::from(self.next_u32()) << 32) | u64::from(self.next_u32())
    }

    pub(crate) fn reseed(&mut self, seed: u64) {
        *self = Self::new(seed);
    }

    pub(crate) fn era_values(&self) -> Vec<i64> {
        self.state
            .iter()
            .map(|word| i64::from(*word))
            .chain(std::iter::once(
                i64::try_from(self.index).expect("SFMT index fits i64"),
            ))
            .collect()
    }

    pub(crate) fn from_era_values(values: &[i64]) -> Result<Self, String> {
        if values.len() != STATE_WORDS + 1 {
            return Err("RANDDATA must contain exactly 625 values".into());
        }
        let mut state = [0_u32; STATE_WORDS];
        for (target, value) in state.iter_mut().zip(values) {
            // Reference SetRand uses an unchecked low-32-bit conversion.
            *target = u32::from_le_bytes(value.to_le_bytes()[..4].try_into().expect("low word"));
        }
        let index =
            usize::try_from(values[STATE_WORDS]).map_err(|_| "RANDDATA index is negative")?;
        if index > STATE_WORDS {
            return Err("RANDDATA index exceeds 624".into());
        }
        Ok(Self { state, index })
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(STATE_WORDS * 4 + 4);
        for word in self.state {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.extend_from_slice(
            &u32::try_from(self.index)
                .expect("SFMT state index fits u32")
                .to_le_bytes(),
        );
        bytes
    }

    pub(crate) fn decode(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() != STATE_WORDS * 4 + 4 {
            return Err("SFMT snapshot has an invalid length".into());
        }
        let index = u32::from_le_bytes(
            bytes[STATE_WORDS * 4..]
                .try_into()
                .expect("four-byte index"),
        ) as usize;
        if index > STATE_WORDS {
            return Err("SFMT snapshot index is out of range".into());
        }
        // Validate every fallible field before replacing any part of the live stream.
        for (word, chunk) in self
            .state
            .iter_mut()
            .zip(bytes[..STATE_WORDS * 4].chunks_exact(4))
        {
            *word = u32::from_le_bytes(chunk.try_into().expect("four-byte chunk"));
        }
        self.index = index;
        Ok(())
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= STATE_WORDS {
            self.generate_all();
            self.index = 0;
        }
        let value = self.state[self.index];
        self.index += 1;
        value
    }

    fn certify_period(&mut self) {
        const PARITY: [u32; 4] = [1, 0, 0, 0x13c9_e684];
        let mut inner = self.state[0] & PARITY[0]
            ^ self.state[1] & PARITY[1]
            ^ self.state[2] & PARITY[2]
            ^ self.state[3] & PARITY[3];
        for shift in [16, 8, 4, 2, 1] {
            inner ^= inner >> shift;
        }
        if inner & 1 == 1 {
            return;
        }
        for (word, parity) in self.state[..4].iter_mut().zip(PARITY) {
            let mut bit = 1_u32;
            for _ in 0..32 {
                if bit & parity != 0 {
                    *word ^= bit;
                    return;
                }
                bit <<= 1;
            }
        }
    }

    #[allow(clippy::many_single_char_names)]
    fn generate_all(&mut self) {
        const POSITION: usize = 122 * 4;
        const MASKS: [u32; 4] = [0xdfff_ffef, 0xddfe_cb7f, 0xbffa_ffff, 0xbfff_fff6];
        let mut a = 0;
        let mut b = POSITION;
        let mut c = (19937 / 128 - 1) * 4;
        let mut d = (19937 / 128) * 4;
        while a < STATE_WORDS {
            let p = &mut self.state;
            p[a + 3] = p[a + 3]
                ^ p[a + 3].wrapping_shl(8)
                ^ (p[a + 2] >> 24)
                ^ (p[c + 3] >> 8)
                ^ ((p[b + 3] >> 11) & MASKS[3])
                ^ p[d + 3].wrapping_shl(18);
            p[a + 2] = p[a + 2]
                ^ p[a + 2].wrapping_shl(8)
                ^ (p[a + 1] >> 24)
                ^ p[c + 3].wrapping_shl(24)
                ^ (p[c + 2] >> 8)
                ^ ((p[b + 2] >> 11) & MASKS[2])
                ^ p[d + 2].wrapping_shl(18);
            p[a + 1] = p[a + 1]
                ^ p[a + 1].wrapping_shl(8)
                ^ (p[a] >> 24)
                ^ p[c + 2].wrapping_shl(24)
                ^ (p[c + 1] >> 8)
                ^ ((p[b + 1] >> 11) & MASKS[1])
                ^ p[d + 1].wrapping_shl(18);
            p[a] = p[a]
                ^ p[a].wrapping_shl(8)
                ^ p[c + 1].wrapping_shl(24)
                ^ (p[c] >> 8)
                ^ ((p[b] >> 11) & MASKS[0])
                ^ p[d].wrapping_shl(18);
            c = d;
            d = a;
            a += 4;
            b += 4;
            if b >= STATE_WORDS {
                b = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_round_trip_preserves_stream() {
        let mut source = Sfmt19937::new(1234);
        let _ = source.next_u64();
        let bytes = source.encode();
        let mut restored = Sfmt19937::new(0);
        restored.decode(&bytes).expect("valid state");
        assert_eq!(source.next_u64(), restored.next_u64());
    }

    #[test]
    fn era_randdata_round_trip_is_exact_and_validates_index() {
        let mut source = Sfmt19937::new(1234);
        let _ = source.next_u64();
        let values = source.era_values();
        let mut restored = Sfmt19937::from_era_values(&values).expect("valid RANDDATA");
        assert_eq!(source.next_u64(), restored.next_u64());

        let mut invalid = values;
        invalid[STATE_WORDS] = 625;
        assert!(Sfmt19937::from_era_values(&invalid).is_err());
    }

    #[test]
    fn rejected_snapshot_preserves_the_existing_random_stream() {
        let mut source = Sfmt19937::new(1234);
        let _ = source.next_u64();
        let before = source.encode();
        let mut invalid = Sfmt19937::new(4321).encode();
        invalid[STATE_WORDS * 4..].copy_from_slice(&625_u32.to_le_bytes());
        assert_eq!(
            source.decode(&invalid),
            Err("SFMT snapshot index is out of range".into()),
        );
        assert_eq!(source.encode(), before);
        assert!(source.decode(&invalid[..invalid.len() - 1]).is_err());
        assert_eq!(source.encode(), before);
    }

    #[test]
    fn seed_uses_only_low_32_bits() {
        let mut low = Sfmt19937::new(7);
        let mut wide = Sfmt19937::new((1_u64 << 32) | 7);
        assert_eq!(low.next_u64(), wide.next_u64());
    }

    #[test]
    fn seed_1234_matches_reference_first_words() {
        // These are the first two uint32 values returned by Emuera's pinned SFMT.cs.
        let mut random = Sfmt19937::new(1234);
        let value = random.next_u64();
        assert_eq!(value, 14_775_466_168_785_827_287);
        assert_eq!(value % 1_000_000, 827_287);
    }
}
