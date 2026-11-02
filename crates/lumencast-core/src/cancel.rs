//! Cooperative cancellation shared between the caller and a capture loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_clear_and_clones_share_cancellation() {
        let flag = CancelFlag::new();
        let observer = flag.clone();
        assert!(!observer.is_cancelled());
        flag.cancel();
        flag.cancel();
        assert!(observer.is_cancelled());
        assert!(flag.is_cancelled());
    }

    #[test]
    fn independent_flags_do_not_affect_each_other() {
        let first = CancelFlag::new();
        let second = CancelFlag::new();
        first.cancel();
        assert!(!second.is_cancelled());
    }
}
