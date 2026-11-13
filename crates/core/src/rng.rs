//! Entropy and hashing primitives.
//!
//! Gameplay randomness goes through the [`Rng`] trait so the session owns a *seeded, reproducible*
//! generator (docs/adr/0009). [`Fnv`] exists for the same reason: the determinism test needs a cheap
//! state fingerprint and we refuse to pull a hashing crate into the dependency-free core.

/// Source of 64-bit randomness. Implement this to inject a scripted or fixed sequence in tests.
pub trait Rng {
    fn next_u64(&mut self) -> u64;
}

/// Extension helpers over any [`Rng`].
pub trait RngExt: Rng {
    /// Value in `0..n` (returns 0 for `n <= 1`).
    ///
    /// Plain modulo: for the tiny `n` we use (`<= 7`, bag shuffling) the bias is on the order of
    /// `n / 2^64`, i.e. far below any measurable effect — see `below_is_not_obviously_biased`.
    /// A rejection sampler would be strictly correct but adds a loop that can never help here.
    fn below(&mut self, n: u64) -> u64 {
        if n <= 1 {
            return 0;
        }
        self.next_u64() % n
    }

    /// Fisher–Yates, in place.
    fn shuffle<T>(&mut self, items: &mut [T]) {
        if items.len() < 2 {
            return;
        }
        for i in (1..items.len()).rev() {
            let j = self.below(i as u64 + 1) as usize;
            items.swap(i, j);
        }
    }
}

impl<R: Rng + ?Sized> RngExt for R {}

/// Classic xorshift64 (Vigna). Tiny, deterministic, good enough to shuffle seven tiles fairly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    /// Seed 0 is mapped to a fixed non-zero state (xorshift64 has 0 as an absorbing state).
    pub const fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    pub const fn state(&self) -> u64 {
        self.state
    }
}

impl Rng for XorShift64 {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}

/// FNV-1a over a `u64` accumulator, used for state fingerprints.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Fnv(u64);

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x100_0000_01b3;

impl Fnv {
    pub fn new() -> Self {
        Self(FNV_OFFSET_BASIS)
    }

    pub fn hash(&mut self, byte: u8) -> &mut Self {
        self.0 ^= byte as u64;
        self.0 = self.0.wrapping_mul(FNV_PRIME);
        self
    }

    pub fn hash_u64(&mut self, value: u64) -> &mut Self {
        for i in 0..8 {
            self.hash((value >> (i * 8)) as u8);
        }
        self
    }

    pub fn hash_i32(&mut self, value: i32) -> &mut Self {
        self.hash_u64(value as u64)
    }

    pub fn hash_bool(&mut self, value: bool) -> &mut Self {
        self.hash(value as u8)
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xorshift_is_deterministic() {
        let mut a = XorShift64::new(1234);
        let mut b = XorShift64::new(1234);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = XorShift64::new(1);
        let mut b = XorShift64::new(2);
        let same: u32 = (0..100).filter(|_| a.next_u64() == b.next_u64()).count() as u32;
        assert!(same < 5, "seeds produced {same} identical draws out of 100");
    }

    #[test]
    fn zero_seed_still_produces_numbers() {
        let mut r = XorShift64::new(0);
        let mut seen_nonzero = false;
        for _ in 0..10 {
            if r.next_u64() != 0 {
                seen_nonzero = true;
            }
        }
        assert!(seen_nonzero, "zero seed collapsed to all zeros");
    }

    #[test]
    fn below_respects_bounds() {
        let mut r = XorShift64::new(0xDEAD_BEEF);
        for _ in 0..10_000 {
            assert!(r.below(7) < 7);
        }
        assert_eq!(r.below(0), 0);
        assert_eq!(r.below(1), 0);
    }

    #[test]
    fn below_is_not_obviously_biased() {
        // 6 buckets, 60_000 draws: each bucket within ±25% of expectation.
        let mut r = XorShift64::new(42);
        let mut counts = [0u32; 6];
        for _ in 0..60_000 {
            counts[r.below(6) as usize] += 1;
        }
        let expected = 10_000.0;
        for (i, c) in counts.iter().enumerate() {
            let dev = (*c as f64 - expected).abs() / expected;
            assert!(dev < 0.25, "bucket {i} count {c} deviates {dev:.2}");
        }
    }

    #[test]
    fn shuffle_permutes_and_terminates() {
        let mut r = XorShift64::new(7);
        let mut v: Vec<u32> = (0..64).collect();
        r.shuffle(&mut v);
        let mut sorted = v.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..64).collect::<Vec<_>>());

        // Degenerate inputs must not panic.
        let empty: [u8; 0] = [];
        let mut e = empty;
        r.shuffle(&mut e);
        let mut one = [9u8];
        r.shuffle(&mut one);
        assert_eq!(one, [9]);
    }

    #[test]
    fn fnv_distinguishes_small_changes() {
        let mut a = Fnv::new();
        a.hash_u64(1).hash_u64(2);
        let mut b = Fnv::new();
        b.hash_u64(2).hash_u64(1);
        assert_ne!(a.finish(), b.finish());

        let mut c = Fnv::new();
        c.hash_u64(1).hash_u64(2);
        assert_eq!(c.finish(), a.finish());

        let mut d = Fnv::new();
        d.hash_u64(1).hash_u64(2).hash_bool(true);
        assert_ne!(d.finish(), a.finish());
    }
}
