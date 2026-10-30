//! Process-wide embedder hooks.
//!
//! The PSP backend installs progress-logging and cooperative-yield
//! callbacks into the tokenizer, tree builder and cascade at boot (see
//! `oasis-backend-psp/src/main.rs`), so a long synchronous page load
//! stays observable and doesn't starve its other threads. Every hook is
//! a plain `fn` pointer in a [`Hook`] static; firing an unset hook is a
//! no-op.

use std::sync::OnceLock;

/// A process-wide, install-once `fn`-pointer hook.
///
/// The first [`set`](Self::set) wins and later calls are ignored: hooks
/// are wired once at boot, and a lock-free [`get`](Self::get) matters
/// more on the hot paths that poll them than re-installation does.
pub(crate) struct Hook<F: Copy + Send + Sync + 'static>(OnceLock<F>);

impl<F: Copy + Send + Sync + 'static> Hook<F> {
    /// An empty hook.
    pub(crate) const fn new() -> Self {
        Self(OnceLock::new())
    }

    /// Install `hook` unless one is already installed.
    pub(crate) fn set(&self, hook: F) {
        let _ = self.0.set(hook);
    }

    /// The installed hook, if any.
    #[inline]
    pub(crate) fn get(&self) -> Option<F> {
        self.0.get().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_HOOK: Hook<fn(u32) -> u32> = Hook::new();

    #[test]
    fn unset_hook_is_none_and_first_set_wins() {
        let local: Hook<fn(u32) -> u32> = Hook::new();
        assert!(local.get().is_none());
        local.set(|x| x + 1);
        local.set(|x| x * 100);
        assert_eq!(local.get().map(|f| f(1)), Some(2));
    }

    #[test]
    fn static_hook_fires() {
        TEST_HOOK.set(|x| x * 2);
        assert_eq!(TEST_HOOK.get().map(|f| f(21)), Some(42));
    }
}
