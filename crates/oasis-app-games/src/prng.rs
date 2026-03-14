//! Deterministic LCG PRNG (no external crate).

/// Advance a 64-bit LCG state and return the new value.
pub(crate) fn next_random(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state
}

/// Return a random value in `[0, bound)`.
pub(crate) fn random_range(state: &mut u64, bound: u64) -> u64 {
    if bound == 0 {
        return 0;
    }
    next_random(state) % bound
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
    fn random_range_zero_bound() {
        let mut s = 42u64;
        assert_eq!(random_range(&mut s, 0), 0);
    }
}
