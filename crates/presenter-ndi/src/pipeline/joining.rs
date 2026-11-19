//! The `joining` consumer counter of an `NdiPipeline` (#979).

/// Counts one `add_consumer` call in `joining` for as long as it runs — also
/// when it returns early with an error or its future is dropped (#979).
pub(super) struct JoiningGuard<'a>(&'a std::sync::atomic::AtomicUsize);

impl<'a> JoiningGuard<'a> {
    pub(super) fn enter(counter: &'a std::sync::atomic::AtomicUsize) -> Self {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self(counter)
    }
}

impl Drop for JoiningGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::JoiningGuard;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn a_joining_consumer_counts_until_its_call_ends() {
        let joining = AtomicUsize::new(0);
        let first = JoiningGuard::enter(&joining);
        let second = JoiningGuard::enter(&joining);
        assert_eq!(joining.load(Ordering::SeqCst), 2);
        drop(first);
        assert_eq!(joining.load(Ordering::SeqCst), 1);
        drop(second);
        assert_eq!(joining.load(Ordering::SeqCst), 0);
    }
}
