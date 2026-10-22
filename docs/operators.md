# Operator Guide

This guide covers running and operating the oracle service in production.

## Sources

Sources submit price data to the oracle contract. Each source client is
responsible for delivering submissions reliably while respecting RPC and
contract capacity limits.

### Submission backpressure protocol

When RPC or contract capacity is constrained, source clients apply the
following bounded backpressure protocol:

- **Bounded queue.** Each source client keeps an in-memory submission queue
  capped at a configured maximum (`max_queue_size`). When the queue is full,
  new submissions are rejected rather than buffered, so memory use stays
  bounded regardless of RPC latency or outages.
- **Bounded retry with backoff.** Failed submissions are retried a bounded
  number of times (`max_retries`) with exponential backoff between attempts.
  Once the retry budget is exhausted the submission is dropped and the failure
  is surfaced in logs/metrics; the client does not retry indefinitely.
- **Stale submissions are dropped.** Every submission carries the timestamp
  of the price it reports. Before sending (and before each retry), the client
  checks whether the submission is still fresh relative to the configured
  staleness window (`max_submission_age`). Stale submissions are discarded and
  never replayed, so a backlog cannot cause the oracle to publish outdated
  prices.

### Recovery

When capacity recovers, the client resumes accepting new submissions. Because
stale entries are dropped and the queue is bounded, recovery does not replay a
backlog of old submissions; operators should expect a brief gap in submissions
that corresponds to the constrained period rather than a burst of stale data.

Tune `max_queue_size`, `max_retries`, and `max_submission_age` to match the
capacity of the RPC endpoint and the contract's submission rate limits.
