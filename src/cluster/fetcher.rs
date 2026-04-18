//! Helpers used by per-endpoint fetch task loops.
//!
//! Task 1 introduces the helpers; Task 9 retrofits all fetch tasks to
//! use them. Exposed as free functions so each fetcher's control flow
//! stays obvious.

use std::sync::atomic::Ordering;
use std::time::Duration;

use rand::Rng;
use tokio::sync::Notify;

/// Next-interval formula per spec §Scale Protection strategy A:
/// `clamp(2.0 * last_dur, base, max)`.
pub fn adaptive_interval(base: Duration, last_dur: Duration, max: Duration) -> Duration {
    let doubled = last_dur.saturating_mul(2);
    doubled.clamp(base, max)
}

/// Sleep for `interval * (1.0 ± jitter_percent/100)`. Returns
/// immediately if `force` is signaled.
pub async fn sleep_with_jitter(interval: Duration, jitter_percent: u8, force: &Notify) {
    let jitter_frac = f64::from(jitter_percent) / 100.0;
    let lo = 1.0 - jitter_frac;
    let hi = 1.0 + jitter_frac;
    let factor: f64 = rand::thread_rng().gen_range(lo..=hi);
    let dur = interval.mul_f64(factor.max(0.0));
    tokio::select! {
        _ = tokio::time::sleep(dur) => {}
        _ = force.notified() => {}
    }
}

/// Observes the subscriber count for `endpoint` without touching the
/// canonical registry. Used in the `gated` branch of a fetch task.
pub fn subscribers_present(counter: &std::sync::atomic::AtomicUsize) -> bool {
    counter.load(Ordering::Relaxed) > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn adaptive_interval_clamps_down_to_base() {
        let base = Duration::from_secs(10);
        let max = Duration::from_secs(60);
        assert_eq!(
            adaptive_interval(base, Duration::from_millis(200), max),
            base
        );
    }

    #[test]
    fn adaptive_interval_scales_with_latency() {
        let base = Duration::from_secs(10);
        let max = Duration::from_secs(60);
        assert_eq!(
            adaptive_interval(base, Duration::from_secs(8), max),
            Duration::from_secs(16)
        );
    }

    #[test]
    fn adaptive_interval_caps_at_max() {
        let base = Duration::from_secs(10);
        let max = Duration::from_secs(60);
        assert_eq!(adaptive_interval(base, Duration::from_secs(120), max), max);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn jitter_stays_within_bounds() {
        let force = Notify::new();
        let interval = Duration::from_secs(10);
        for _ in 0..20 {
            let t0 = tokio::time::Instant::now();
            sleep_with_jitter(interval, 20, &force).await;
            let elapsed = t0.elapsed();
            assert!(
                elapsed >= Duration::from_secs(8),
                "elapsed {:?} below 0.8×base",
                elapsed
            );
            assert!(
                elapsed <= Duration::from_secs(12),
                "elapsed {:?} above 1.2×base",
                elapsed
            );
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn force_notify_short_circuits_sleep() {
        let force = Notify::new();
        force.notify_one(); // pre-arm
        let t0 = tokio::time::Instant::now();
        sleep_with_jitter(Duration::from_secs(60), 20, &force).await;
        assert!(t0.elapsed() < Duration::from_millis(50));
    }

    #[test]
    fn subscribers_present_tracks_atomic() {
        let counter = AtomicUsize::new(0);
        assert!(!subscribers_present(&counter));
        counter.fetch_add(1, Ordering::Relaxed);
        assert!(subscribers_present(&counter));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn gated_fetch_parks_until_subscribed() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicUsize;

        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let force = Arc::new(Notify::new());
                let counter = Arc::new(AtomicUsize::new(0));
                let fire_count = Arc::new(AtomicUsize::new(0));

                let task = {
                    let force = force.clone();
                    let counter = counter.clone();
                    let fire_count = fire_count.clone();
                    tokio::task::spawn_local(async move {
                        loop {
                            if !subscribers_present(&counter) {
                                force.notified().await;
                                continue;
                            }
                            fire_count.fetch_add(1, Ordering::Relaxed);
                            sleep_with_jitter(Duration::from_secs(10), 20, &force).await;
                        }
                    })
                };

                // Advance 30s with no subscribers — task must park.
                tokio::time::advance(Duration::from_secs(30)).await;
                tokio::task::yield_now().await;
                assert_eq!(fire_count.load(Ordering::Relaxed), 0);

                // Subscribe (0→1) — fetch fires.
                counter.fetch_add(1, Ordering::Relaxed);
                force.notify_one();
                tokio::task::yield_now().await;
                assert_eq!(fire_count.load(Ordering::Relaxed), 1);

                task.abort();
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn ungated_fetch_fires_without_subscribers() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicUsize;

        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let force = Arc::new(Notify::new());
                let fire_count = Arc::new(AtomicUsize::new(0));

                let task = {
                    let force = force.clone();
                    let fire_count = fire_count.clone();
                    tokio::task::spawn_local(async move {
                        loop {
                            // Not gated — skip the guard entirely.
                            fire_count.fetch_add(1, Ordering::Relaxed);
                            sleep_with_jitter(Duration::from_secs(10), 20, &force).await;
                        }
                    })
                };

                // Fire at t=0, advance to t=12s (clears the 20% jitter
                // upper bound of 12s), expect at least 2 fires.
                tokio::task::yield_now().await;
                tokio::time::advance(Duration::from_secs(12)).await;
                tokio::task::yield_now().await;
                assert!(fire_count.load(Ordering::Relaxed) >= 2);

                task.abort();
            })
            .await;
    }
}
