# RPC Retry Amplification Budget (#696)

When an RPC provider degrades, an unbounded retry loop multiplies the load sent
to it: every call becomes `max_attempts` requests, sent exactly when the
provider can least absorb them, and every waiting caller opens another
connection. The budget in
[`services/rpc_guard/budget.py`](../services/rpc_guard/budget.py) caps retries,
timeouts and concurrency so a provider failure cannot multiply upstream load
without limit.

One `RetryBudget` is shared by every caller of the same provider. The caps
apply to the fleet's load on that provider, not to one call site.

```python
from services.rpc_guard.budget import RetryBudget, RetryPolicy

budget = RetryBudget(RetryPolicy(max_attempts=3, max_in_flight=8))
ledger = budget.call(lambda timeout: rpc.get_latest_ledger(timeout=timeout))
```

The operation receives the timeout it must apply to its own I/O.

## Caps

| Setting | Default | Bound it enforces |
|---|---|---|
| `max_attempts` | 3 | attempts per call, including the first |
| `attempt_timeout_secs` | 5.0 | timeout handed to each attempt |
| `deadline_secs` | 15.0 | total time for a call; attempts and backoff both count, and an attempt's timeout never exceeds what is left |
| `max_in_flight` | 8 | calls holding a slot at once; a call with no free slot is rejected immediately, never queued |
| `retry_ratio` | 0.1 | retry tokens each admitted call deposits |
| `retry_bucket_size` | 10 | token bucket capacity and initial balance |
| `base_backoff_secs` / `max_backoff_secs` | 0.2 / 2.0 | exponential backoff between attempts |

Each retry spends one token from the shared bucket. Across `N` admitted calls:

```
retries  <= retry_bucket_size + retry_ratio * N
attempts <= N + min(N * (max_attempts - 1), retry_bucket_size + retry_ratio * N)
```

`RetryPolicy.max_attempts_for(N)` returns that bound. With the defaults, a total
outage costs about 1.1 requests per call instead of 3. Once calls start
succeeding again the bucket refills and transient errors get retried as usual.

## Rejections

Every refusal raises `BudgetExhausted` with a closed-enum `code`, which is safe
to use as a metric label:

| Code | Meaning |
|---|---|
| `in_flight` | no free concurrency slot; the call was not sent |
| `attempts` | `max_attempts` attempts failed |
| `retry_budget` | the shared retry bucket is empty; the provider is failing fleet-wide |
| `deadline` | the call's `deadline_secs` ran out |

A non-retryable error (per the `retryable` predicate passed to `call`) is
re-raised unchanged after one attempt. When a retryable error is abandoned,
the `BudgetExhausted` is chained to the last error.

## Metrics

`RetryBudget.prometheus()` exports:

```
rpc_budget_calls_total
rpc_budget_attempts_total
rpc_budget_retries_total
rpc_budget_in_flight
rpc_budget_rejected_total{code="in_flight|attempts|retry_budget|deadline"}
```

If `rpc_budget_rejected_total{code="retry_budget"}` keeps rising, the provider
is failing for everyone. Fail over to another provider; raising the caps would
only put the amplification back.

## Verification

```bash
pytest services/rpc_guard/test/test_budget.py
```

The fault-injection tests drive the budget against a provider that is always
down, and against one that is slow and failing under 32 concurrent workers.
They check that observed in-flight requests never exceed `max_in_flight`, that
total retry volume stays within the bucket bound, that each attempt's timeout
respects both caps, and that backoff never runs past the deadline.
