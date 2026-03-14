//! Shared helpers for command argument validation.
//!
//! Eliminates repeated argument-count checks and usage-string error
//! construction across the 90+ commands in oasis-terminal.

use oasis_types::error::{OasisError, Result};

/// Require at least `min` arguments, returning a usage error if fewer are provided.
///
/// ```ignore
/// require_args(args, 1, "cat <file>")?;
/// ```
pub fn require_args(args: &[&str], min: usize, usage: &str) -> Result<()> {
    if args.len() < min {
        return Err(OasisError::Command(format!("usage: {usage}").into()));
    }
    Ok(())
}

/// Require exactly `n` arguments, returning a usage error otherwise.
///
/// ```ignore
/// require_args_exact(args, 2, "cp <src> <dst>")?;
/// ```
pub fn require_args_exact(args: &[&str], n: usize, usage: &str) -> Result<()> {
    if args.len() != n {
        return Err(OasisError::Command(format!("usage: {usage}").into()));
    }
    Ok(())
}

/// Platform-safe seed for PRNG. Uses `TimeService` when available (required on
/// WASM where `std::time::SystemTime::now()` panics), falls back to std on native.
pub fn time_seed(env: &crate::Environment<'_>) -> u64 {
    if let Some(time) = env.time
        && let Ok(now) = time.now()
    {
        return (now.year as u64) << 40
            | (now.month as u64) << 32
            | (now.day as u64) << 24
            | (now.hour as u64) << 16
            | (now.minute as u64) << 8
            | (now.second as u64);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
    }
    #[cfg(target_arch = "wasm32")]
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_args_ok_when_enough() {
        assert!(require_args(&["a", "b"], 1, "test <arg>").is_ok());
        assert!(require_args(&["a", "b"], 2, "test <a> <b>").is_ok());
    }

    #[test]
    fn require_args_err_when_too_few() {
        let err = require_args(&[], 1, "cat <file>").unwrap_err();
        assert!(format!("{err}").contains("usage: cat <file>"));
    }

    #[test]
    fn require_args_zero_always_ok() {
        assert!(require_args(&[], 0, "test").is_ok());
    }

    #[test]
    fn require_args_exact_ok() {
        assert!(require_args_exact(&["a", "b"], 2, "cp <src> <dst>").is_ok());
    }

    #[test]
    fn require_args_exact_too_few() {
        let err = require_args_exact(&["a"], 2, "cp <src> <dst>").unwrap_err();
        assert!(format!("{err}").contains("usage: cp <src> <dst>"));
    }

    #[test]
    fn require_args_exact_too_many() {
        let err = require_args_exact(&["a", "b", "c"], 2, "cp <src> <dst>").unwrap_err();
        assert!(format!("{err}").contains("usage:"));
    }
}
