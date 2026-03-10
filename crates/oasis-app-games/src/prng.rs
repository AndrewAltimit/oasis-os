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
