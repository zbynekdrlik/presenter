//! A concurrent batch of Resolume requests is drained to the end (#841 review).
//!
//! Clip connects, text PUTs and opacity PUTs each run one batch of requests
//! concurrently through a `FuturesUnordered`. Returning on the first error
//! dropped the batch's other requests mid-flight, and one already on the wire
//! could still reach Arena after the batch had returned (CI run 38176143346: a
//! fade value landed after the clear and was counted with the next verse). So
//! a batch is collected to its last request, and only then does the first
//! error win.

use futures_util::{Stream, StreamExt};

/// Await every request of `batch`. Returns each success in completion order,
/// or the first error in completion order once every request has finished.
pub(super) async fn drain<S, T>(batch: S) -> anyhow::Result<Vec<T>>
where
    S: Stream<Item = anyhow::Result<T>>,
{
    let results: Vec<anyhow::Result<T>> = batch.collect().await;
    results.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use futures_util::stream::FuturesUnordered;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn a_failed_request_does_not_cut_the_batch_short() {
        let finished = Arc::new(AtomicUsize::new(0));
        let batch = FuturesUnordered::new();
        for (delay_ms, fails) in [(0_u64, true), (30, false), (60, false)] {
            let finished = finished.clone();
            batch.push(async move {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                finished.fetch_add(1, Ordering::SeqCst);
                if fails {
                    Err(anyhow!("refused"))
                } else {
                    Ok(delay_ms)
                }
            });
        }

        let err = drain(batch).await.expect_err("the failure is reported");
        assert_eq!(err.to_string(), "refused");
        assert_eq!(
            finished.load(Ordering::SeqCst),
            3,
            "every request finished before the batch failed"
        );
    }

    #[tokio::test]
    async fn a_batch_without_errors_returns_its_successes_in_completion_order() {
        let batch = FuturesUnordered::new();
        for delay_ms in [60_u64, 0, 30] {
            batch.push(async move {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                anyhow::Ok(delay_ms)
            });
        }
        assert_eq!(drain(batch).await.expect("all succeed"), vec![0, 30, 60]);
    }
}
