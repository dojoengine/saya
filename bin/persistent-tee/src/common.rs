pub const SAYA_DB_PATH: &str = "saya-tee.db";

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    };

    #[tokio::test]
    async fn succeeds_on_first_attempt() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let result = retry_with_backoff("t", 3, Duration::from_millis(1), || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok::<u32, anyhow::Error>(42)
            }
        })
        .await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retries_on_transient_failure() {
        // Fails twice, succeeds on attempt 3 — exactly at the limit.
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let result = retry_with_backoff("t", 3, Duration::from_millis(1), || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                if n < 3 { Err(anyhow::anyhow!("transient")) } else { Ok(n) }
            }
        })
        .await;
        assert_eq!(result.unwrap(), 3);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn exhaustion_returns_last_error() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let result = retry_with_backoff::<u32, _, _>("t", 3, Duration::from_millis(1), || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(anyhow::anyhow!("permanent"))
            }
        })
        .await;
        assert!(result.is_err());
        // Exactly max_attempts calls were made — no extra retries.
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn delay_doubles_each_attempt() {
        // Verify the backoff doubles: measure wall-time is ≥ initial_delay + 2×initial_delay.
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let start = tokio::time::Instant::now();
        let _ = retry_with_backoff::<u32, _, _>("t", 3, Duration::from_millis(10), || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(anyhow::anyhow!("fail"))
            }
        })
        .await;
        // 3 attempts: sleep 10ms after #1, sleep 20ms after #2 → ≥30ms total sleep.
        assert!(start.elapsed() >= Duration::from_millis(30));
    }
}

use std::{future::Future, time::Duration};

use anyhow::Result;
use log::warn;

pub const STAGE_MAX_ATTEMPTS: u32 = 3;
pub const RETRY_INITIAL_DELAY: Duration = Duration::from_secs(1);

pub async fn retry_with_backoff<T, F, Fut>(
	label: &str,
	max_attempts: u32,
	initial_delay: Duration,
	operation: F,
) -> Result<T>
where
	F: Fn() -> Fut,
	Fut: Future<Output = Result<T>>,
{
	let mut attempt = 0;
	let mut delay = initial_delay;

	loop {
		attempt += 1;
		match operation().await {
			Ok(value) => return Ok(value),
			Err(err) if attempt < max_attempts => {
				warn!(
					stage = label,
					attempt,
					max_attempts,
					error:% = err,
					backoff_ms = delay.as_millis();
					"Stage attempt failed; retrying"
				);
				tokio::time::sleep(delay).await;
				delay = delay.saturating_mul(2);
			}
			Err(err) => {
				warn!(
					stage = label,
					attempt,
					max_attempts,
					error:% = err;
					"Stage retries exhausted"
				);
				return Err(err);
			}
		}
	}
}
