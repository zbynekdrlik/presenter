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

/// Counts one WHEP POST in `joining` from the moment it took the pipeline out
/// of the manager's active map (under that lock) until the POST ends, so a
/// park that checks [`NdiPipeline::has_consumers_now`] under the same lock can
/// never stop a pipeline a viewer is about to join (#979 review).
pub struct JoinTicket(std::sync::Arc<super::NdiPipeline>);

impl JoinTicket {
    pub fn pipeline(&self) -> &std::sync::Arc<super::NdiPipeline> {
        &self.0
    }
}

impl Drop for JoinTicket {
    fn drop(&mut self) {
        self.0
            .joining
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl super::NdiPipeline {
    /// Take a [`JoinTicket`]: the caller is counted as a joining consumer
    /// until the ticket drops.
    pub fn join_ticket(self: &std::sync::Arc<Self>) -> JoinTicket {
        self.joining
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        JoinTicket(std::sync::Arc::clone(self))
    }

    /// Whether a consumer is present or joining right now, without waiting:
    /// a sessions map someone holds counts as in use (#979 review — the park
    /// re-checks this under the manager's `active` lock).
    pub fn has_consumers_now(&self) -> bool {
        if self.joining.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            return true;
        }
        match self.sessions.try_lock() {
            Ok(sessions) => !sessions.is_empty(),
            Err(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::JoiningGuard;
    use crate::pipeline::NdiPipeline;
    use std::sync::Arc;

    #[tokio::test]
    async fn a_join_ticket_counts_as_a_consumer_until_it_drops() {
        let pipeline = Arc::new(NdiPipeline::stopped_for_test());
        assert!(!pipeline.has_consumers_now());
        let ticket = pipeline.join_ticket();
        assert!(Arc::ptr_eq(ticket.pipeline(), &pipeline));
        assert!(pipeline.has_consumers_now());
        assert_eq!(pipeline.consumer_count().await, 1);
        drop(ticket);
        assert!(!pipeline.has_consumers_now());
        assert_eq!(pipeline.consumer_count().await, 0);
    }

    #[tokio::test]
    async fn a_held_sessions_map_counts_as_in_use() {
        let pipeline = Arc::new(NdiPipeline::stopped_for_test());
        let held = pipeline.sessions.lock().await;
        assert!(pipeline.has_consumers_now());
        drop(held);
        assert!(!pipeline.has_consumers_now());
    }
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
