//! Async polling helpers for waiting on NiFi component terminal states.
//!
//! NiFi REST operations are declarative: a 200 response means the request
//! was accepted, not that the new state has been reached. Creating a
//! processor returns immediately; validation takes another ~100ms-5s.
//! Every operation in the seeder that changes state must poll-to-terminal
//! with a hard timeout via `poll_until` below.

use std::{future::Future, time::Duration};

use crate::error::{Result, SeederError};

/// Poll `check` with exponential backoff until it returns `Ok(Some(t))`,
/// or until the timeout elapses. `what` and `target_state` are used in
/// the timeout error message. Returns the `T` on success.
///
/// Backoff: starts at 100ms, doubles each iteration, capped at 2s.
pub async fn poll_until<T, F, Fut>(
    what: impl Into<String>,
    target_state: impl Into<String>,
    timeout: Duration,
    mut check: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<T>>>,
{
    let what = what.into();
    let target_state = target_state.into();
    let start = std::time::Instant::now();
    let mut delay = Duration::from_millis(100);
    let max_delay = Duration::from_secs(2);

    loop {
        if let Some(t) = check().await? {
            tracing::debug!(
                %what,
                %target_state,
                elapsed_ms = start.elapsed().as_millis() as u64,
                "reached terminal state"
            );
            return Ok(t);
        }
        if start.elapsed() >= timeout {
            return Err(SeederError::StateTimeout {
                what,
                target_state,
                elapsed_secs: timeout.as_secs(),
            });
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(max_delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn returns_immediately_on_first_success() {
        let result: Result<i32> = poll_until("thing", "READY", Duration::from_secs(5), || async {
            Ok(Some(42))
        })
        .await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn polls_multiple_times_until_ready() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let result: Result<i32> = poll_until("thing", "READY", Duration::from_secs(5), move || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n >= 2 { Ok(Some(42)) } else { Ok(None) }
            }
        })
        .await;
        assert_eq!(result.unwrap(), 42);
        assert!(counter.load(Ordering::SeqCst) >= 3);
    }

    #[tokio::test]
    async fn times_out() {
        let result: Result<i32> =
            poll_until("thing", "READY", Duration::from_millis(300), || async {
                Ok(None)
            })
            .await;
        match result {
            Err(SeederError::StateTimeout {
                what, target_state, ..
            }) => {
                assert_eq!(what, "thing");
                assert_eq!(target_state, "READY");
            }
            other => panic!("expected StateTimeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn propagates_errors_from_check() {
        let result: Result<i32> = poll_until("thing", "READY", Duration::from_secs(5), || async {
            Err(SeederError::Invariant {
                message: "nope".into(),
            })
        })
        .await;
        match result {
            Err(SeederError::Invariant { message }) => assert_eq!(message, "nope"),
            other => panic!("expected Invariant error, got {other:?}"),
        }
    }
}
