// SPDX-License-Identifier: AGPL-3.0-only
use std::{sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(crate) struct RenderQueue {
    admitted: Arc<Semaphore>,
    active: Arc<Semaphore>,
    wait: Duration,
}

pub(crate) struct RenderPermit {
    _active: OwnedSemaphorePermit,
    _admitted: OwnedSemaphorePermit,
}

#[derive(Debug, PartialEq)]
pub(crate) enum QueueError {
    Full,
    TimedOut,
}

impl RenderQueue {
    pub(crate) fn new(workers: usize, capacity: usize, wait: Duration) -> Self {
        Self {
            admitted: Arc::new(Semaphore::new(workers + capacity)),
            active: Arc::new(Semaphore::new(workers)),
            wait,
        }
    }

    pub(crate) async fn acquire(&self) -> Result<RenderPermit, QueueError> {
        // Admission happens before reading a chart: waiting requests retain
        // only their validated name/options, not up to 20 MiB of source each.
        let admitted = self
            .admitted
            .clone()
            .try_acquire_owned()
            .map_err(|_| QueueError::Full)?;
        let active = tokio::time::timeout(self.wait, self.active.clone().acquire_owned())
            .await
            .map_err(|_| QueueError::TimedOut)?
            .map_err(|_| QueueError::Full)?;
        Ok(RenderPermit {
            _active: active,
            _admitted: admitted,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{future::Future, task::Context, task::Waker};

    #[tokio::test]
    async fn parallel_jobs_wait_in_order_and_only_overflow_is_rejected() {
        let queue = RenderQueue::new(2, 2, Duration::from_secs(1));
        let first = queue.acquire().await.unwrap();
        let second = queue.acquire().await.unwrap();
        let mut third = Box::pin(queue.acquire());
        let mut fourth = Box::pin(queue.acquire());
        let mut cx = Context::from_waker(Waker::noop());
        assert!(third.as_mut().poll(&mut cx).is_pending());
        assert!(fourth.as_mut().poll(&mut cx).is_pending());
        assert!(matches!(queue.acquire().await, Err(QueueError::Full)));
        drop(first);
        assert!(fourth.as_mut().poll(&mut cx).is_pending());
        let third = third.await.unwrap();
        drop(second);
        let fourth = fourth.await.unwrap();
        drop((third, fourth));
        assert!(queue.acquire().await.is_ok());
    }

    #[tokio::test]
    async fn cancelled_and_timed_out_waiters_release_capacity() {
        let queue = RenderQueue::new(1, 1, Duration::from_millis(20));
        let active = queue.acquire().await.unwrap();
        let mut cancelled = Box::pin(queue.acquire());
        assert!(
            cancelled
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        drop(cancelled);
        assert!(matches!(queue.acquire().await, Err(QueueError::TimedOut)));
        drop(active);
        assert!(queue.acquire().await.is_ok());
    }

    #[tokio::test]
    async fn zero_capacity_disables_waiting_but_allows_parallel_workers() {
        let queue = RenderQueue::new(2, 0, Duration::from_secs(1));
        let first = queue.acquire().await.unwrap();
        let second = queue.acquire().await.unwrap();
        assert!(matches!(queue.acquire().await, Err(QueueError::Full)));
        drop(first);
        assert!(queue.acquire().await.is_ok());
        drop(second);
    }
}
