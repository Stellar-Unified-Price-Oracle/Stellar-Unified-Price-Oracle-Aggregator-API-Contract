use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Configuration for the source submission backpressure protocol.
///
/// Bounds both the in-memory submission queue and the retry behavior so that
/// a constrained RPC or contract cannot cause unbounded memory growth or
/// replay of stale submissions.
#[derive(Debug, Clone)]
pub struct BackpressureConfig {
    /// Maximum number of submissions buffered in memory. New submissions are
    /// rejected (not queued) once this limit is reached.
    pub max_queue_len: usize,
    /// Maximum number of retry attempts for a single submission before it is
    /// dropped.
    pub max_retries: u32,
    /// Base delay used for exponential backoff between retries.
    pub base_backoff: Duration,
    /// Maximum backoff delay (cap for exponential growth).
    pub max_backoff: Duration,
    /// A submission older than this is considered stale and is dropped rather
    /// than retried or replayed.
    pub max_submission_age: Duration,
}

impl Default for BackpressureConfig {
    fn default() -> Self {
        Self {
            max_queue_len: 256,
            max_retries: 5,
            base_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(5),
            max_submission_age: Duration::from_secs(60),
        }
    }
}

/// A pending source submission awaiting delivery.
#[derive(Debug, Clone)]
pub struct PendingSubmission {
    pub payload: Vec<u8>,
    pub enqueued_at: Instant,
    pub attempts: u32,
}

/// Outcome of attempting to enqueue a submission.
#[derive(Debug, PartialEq, Eq)]
pub enum EnqueueOutcome {
    /// Submission was accepted into the bounded queue.
    Accepted,
    /// Submission was rejected because the queue is at capacity.
    RejectedFull,
}

/// Outcome of a delivery attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum DeliveryOutcome {
    /// Submission delivered successfully.
    Delivered,
    /// Submission failed and should be retried after the given delay.
    RetryAfter(Duration),
    /// Submission was dropped (stale or retries exhausted).
    Dropped(DropReason),
}

/// Reason a submission was dropped.
#[derive(Debug, PartialEq, Eq)]
pub enum DropReason {
    /// Submission exceeded `max_submission_age`.
    Stale,
    /// Submission exhausted `max_retries`.
    RetriesExhausted,
}

/// Bounded submission queue implementing the backpressure protocol.
#[derive(Debug)]
pub struct SourceSubmissionQueue {
    config: BackpressureConfig,
    queue: VecDeque<PendingSubmission>,
}

impl SourceSubmissionQueue {
    pub fn new(config: BackpressureConfig) -> Self {
        Self {
            config,
            queue: VecDeque::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Enqueue a submission, enforcing the bounded queue limit.
    pub fn enqueue(&mut self, payload: Vec<u8>, now: Instant) -> EnqueueOutcome {
        if self.queue.len() >= self.config.max_queue_len {
            return EnqueueOutcome::RejectedFull;
        }
        self.queue.push_back(PendingSubmission {
            payload,
            enqueued_at: now,
            attempts: 0,
        });
        EnqueueOutcome::Accepted
    }

    /// Pop the next submission for delivery, dropping any stale entries first.
    ///
    /// Stale submissions are never returned for delivery, guaranteeing that
    /// stale submissions are not replayed.
    pub fn next_for_delivery(&mut self, now: Instant) -> Option<PendingSubmission> {
        self.drop_stale(now);
        self.queue.pop_front()
    }

    /// Record the result of a delivery attempt for a submission.
    ///
    /// Returns the outcome and, when retrying, re-enqueues the submission at
    /// the back of the queue (bounded by `max_queue_len`). Stale submissions
    /// are dropped instead of retried.
    pub fn record_result(
        &mut self,
        mut submission: PendingSubmission,
        success: bool,
        now: Instant,
    ) -> DeliveryOutcome {
        if success {
            return DeliveryOutcome::Delivered;
        }

        if now.duration_since(submission.enqueued_at) >= self.config.max_submission_age {
            return DeliveryOutcome::Dropped(DropReason::Stale);
        }

        submission.attempts += 1;
        if submission.attempts > self.config.max_retries {
            return DeliveryOutcome::Dropped(DropReason::RetriesExhausted);
        }

        let delay = self.backoff_for(submission.attempts);

        // Re-enqueue only if capacity allows; otherwise drop to bound memory.
        if self.queue.len() >= self.config.max_queue_len {
            return DeliveryOutcome::Dropped(DropReason::RetriesExhausted);
        }
        self.queue.push_back(submission);

        DeliveryOutcome::RetryAfter(delay)
    }

    /// Compute bounded exponential backoff for the given attempt count.
    fn backoff_for(&self, attempts: u32) -> Duration {
        let factor = 1u32.checked_shl(attempts.saturating_sub(1)).unwrap_or(u32::MAX);
        let delay = self
            .config
            .base_backoff
            .checked_mul(factor)
            .unwrap_or(self.config.max_backoff);
        delay.min(self.config.max_backoff)
    }

    /// Drop all submissions older than `max_submission_age`.
    fn drop_stale(&mut self, now: Instant) {
        let max_age = self.config.max_submission_age;
        self.queue
            .retain(|s| now.duration_since(s.enqueued_at) < max_age);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> BackpressureConfig {
        BackpressureConfig {
            max_queue_len: 3,
            max_retries: 2,
            base_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(40),
            max_submission_age: Duration::from_secs(5),
        }
    }

    #[test]
    fn queue_bounds_memory_under_fault_injection() {
        // Fault injection: RPC/contract capacity is constrained, so every
        // delivery fails. The queue must never exceed max_queue_len.
        let mut q = SourceSubmissionQueue::new(config());
        let now = Instant::now();

        for i in 0..100u32 {
            let outcome = q.enqueue(vec![i as u8], now);
            if i < 3 {
                assert_eq!(outcome, EnqueueOutcome::Accepted);
            } else {
                assert_eq!(outcome, EnqueueOutcome::RejectedFull);
            }
            assert!(q.len() <= config().max_queue_len);
        }

        // Drain with failing deliveries; retries must not grow the queue.
        for _ in 0..10 {
            if let Some(sub) = q.next_for_delivery(now) {
                let _ = q.record_result(sub, false, now);
            }
            assert!(q.len() <= config().max_queue_len);
        }
    }

    #[test]
    fn stale_submissions_are_not_replayed() {
        let mut q = SourceSubmissionQueue::new(config());
        let start = Instant::now();
        assert_eq!(q.enqueue(vec![1, 2, 3], start), EnqueueOutcome::Accepted);

        // Advance past max_submission_age; the stale submission must be dropped
        // and never returned for delivery.
        let later = start + Duration::from_secs(10);
        assert!(q.next_for_delivery(later).is_none());
        assert!(q.is_empty());
    }

    #[test]
    fn retries_are_bounded_and_backoff_capped() {
        let mut q = SourceSubmissionQueue::new(config());
        let now = Instant::now();
        q.enqueue(vec![9], now);

        let mut sub = q.next_for_delivery(now).unwrap();
        for attempt in 1..=config().max_retries {
            let outcome = q.record_result(sub, false, now);
            match outcome {
                DeliveryOutcome::RetryAfter(delay) => {
                    assert!(delay <= config().max_backoff);
                    sub = q.next_for_delivery(now).unwrap();
                    assert_eq!(sub.attempts, attempt);
                }
                other => panic!("unexpected outcome: {:?}", other),
            }
        }

        // Next failure exhausts retries and drops the submission.
        let outcome = q.record_result(sub, false, now);
        assert_eq!(outcome, DeliveryOutcome::Dropped(DropReason::RetriesExhausted));
        assert!(q.is_empty());
    }
}
