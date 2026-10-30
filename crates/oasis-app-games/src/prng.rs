//! Deterministic LCG PRNG (no external crate).

/// Advance a 64-bit LCG state and return the new value.
pub(crate) fn next_random(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state
}

/// Return a random value in `[0, bound)`.
///
/// Scales by the *high* bits of the LCG output: the low bits of a
/// power-of-two-modulus LCG have tiny periods (bit `k` repeats every
/// `2^(k+1)` steps), so `x % 4` would cycle with period 4 for every seed.
pub(crate) fn random_range(state: &mut u64, bound: u64) -> u64 {
    if bound == 0 {
        return 0;
    }
    ((u128::from(next_random(state)) * u128::from(bound)) >> 64) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prng_deterministic_same_seed() {
        let mut s1 = 123u64;
        let mut s2 = 123u64;
        let a = next_random(&mut s1);
        let b = next_random(&mut s2);
        assert_eq!(a, b);
    }

    #[test]
    fn prng_different_seeds_differ() {
        let mut s1 = 1u64;
        let mut s2 = 2u64;
        let a = next_random(&mut s1);
        let b = next_random(&mut s2);
        assert_ne!(a, b);
    }

    #[test]
    fn prng_sequence_not_constant() {
        let mut s = 42u64;
        let a = next_random(&mut s);
        let b = next_random(&mut s);
        assert_ne!(a, b);
    }

    #[test]
    fn random_range_within_bound() {
        let mut s = 99u64;
        for _ in 0..100 {
            let v = random_range(&mut s, 10);
            assert!(v < 10);
        }
    }

    #[test]
    fn small_ranges_are_not_periodic_and_depend_on_the_seed() {
        let seq = |seed: u64| {
            let mut s = seed;
            (0..32).map(|_| random_range(&mut s, 4)).collect::<Vec<_>>()
        };
        for seed in [1u64, 42, 43, 44, 1000] {
            let v = seq(seed);
            assert!(
                (4..v.len()).any(|i| v[i] != v[i - 4]),
                "seed {seed}: period-4 sequence {v:?}"
            );
        }
        assert_ne!(seq(42), seq(43));
        assert_ne!(seq(43), seq(44));
    }

    #[test]
    fn random_range_zero_bound() {
        let mut s = 42u64;
        assert_eq!(random_range(&mut s, 0), 0);
    }
}
