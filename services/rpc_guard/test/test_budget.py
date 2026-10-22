"""Fault-injection tests for the RPC retry amplification budget (#696)."""

import threading
import time

import pytest

from services.rpc_guard.budget import (
    BudgetCode,
    BudgetExhausted,
    RetryBudget,
    RetryPolicy,
)


class FakeClock:
    def __init__(self):
        self.now = 0.0

    def __call__(self):
        return self.now

    def sleep(self, secs):
        self.now += secs


def budget(**over):
    clock = FakeClock()
    return RetryBudget(RetryPolicy(**over), clock=clock, sleep=clock.sleep), clock


def failing(timeout):
    raise ConnectionError("provider down")


# ── Normal operation ─────────────────────────────────────────────────────────

def test_success_is_one_attempt_and_releases_the_slot():
    b, _ = budget()
    assert b.call(lambda timeout: "ok") == "ok"
    snap = b.snapshot()
    assert (snap["calls"], snap["attempts"], snap["retries"], snap["in_flight"]) == (1, 1, 0, 0)


def test_transient_failure_is_retried_within_the_attempt_cap():
    b, _ = budget(max_attempts=3)
    outcomes = iter([ConnectionError("blip"), None])

    def flaky(timeout):
        error = next(outcomes)
        if error:
            raise error
        return "ok"

    assert b.call(flaky) == "ok"
    assert b.snapshot()["attempts"] == 2


def test_non_retryable_error_is_not_retried():
    b, _ = budget()
    with pytest.raises(ValueError):
        b.call(lambda timeout: (_ for _ in ()).throw(ValueError("bad request")),
               retryable=lambda e: isinstance(e, ConnectionError))
    assert b.snapshot()["attempts"] == 1


# ── Boundaries ───────────────────────────────────────────────────────────────

def test_attempts_never_exceed_max_attempts():
    b, _ = budget(max_attempts=3, retry_bucket_size=100)
    with pytest.raises(BudgetExhausted) as exc:
        b.call(failing)
    assert exc.value.code is BudgetCode.ATTEMPTS
    assert isinstance(exc.value.__cause__, ConnectionError)
    assert b.snapshot()["attempts"] == 3


def test_each_attempt_timeout_is_capped_by_attempt_timeout_and_remaining_deadline():
    b, clock = budget(max_attempts=10, attempt_timeout_secs=4.0, deadline_secs=10.0,
                      base_backoff_secs=0.0, max_backoff_secs=0.0, retry_bucket_size=100)
    seen = []

    def slow_failure(timeout):
        seen.append(timeout)
        clock.now += timeout  # the attempt uses its whole timeout, then fails
        raise TimeoutError("timed out")

    with pytest.raises(BudgetExhausted) as exc:
        b.call(slow_failure)
    assert exc.value.code is BudgetCode.DEADLINE
    assert seen == [4.0, 4.0, 2.0]
    assert clock.now <= 10.0


def test_backoff_counts_against_the_deadline():
    b, clock = budget(max_attempts=10, deadline_secs=1.0, base_backoff_secs=0.4,
                      max_backoff_secs=5.0, retry_bucket_size=100)
    with pytest.raises(BudgetExhausted) as exc:
        b.call(failing)
    assert exc.value.code is BudgetCode.DEADLINE
    assert clock.now <= 1.0 + 1e-9


def test_policy_rejects_unbounded_configuration():
    for bad in ({"max_attempts": 0}, {"max_in_flight": 0}, {"deadline_secs": 0},
                {"retry_ratio": -1}, {"base_backoff_secs": 3, "max_backoff_secs": 1}):
        with pytest.raises(ValueError):
            RetryPolicy(**bad)


# ── Fault injection: retry volume during a sustained outage ──────────────────

def test_sustained_outage_retry_volume_stays_within_the_shared_budget():
    policy = dict(max_attempts=3, retry_ratio=0.1, retry_bucket_size=10)
    b, _ = budget(**policy)
    calls = 1_000
    codes = []
    for _ in range(calls):
        with pytest.raises(BudgetExhausted) as exc:
            b.call(failing)
        codes.append(exc.value.code)

    snap = b.snapshot()
    assert snap["retries"] <= 10 + 0.1 * calls
    assert snap["attempts"] <= RetryPolicy(**policy).max_attempts_for(calls)
    # A naive loop would send 3 000 requests; the budget keeps it near 1.1x.
    assert snap["attempts"] / calls <= 1.12
    assert codes.count(BudgetCode.RETRY_BUDGET) > 0


def test_retry_budget_recovers_once_calls_succeed_again():
    b, _ = budget(retry_ratio=0.5, retry_bucket_size=2)
    for _ in range(10):
        with pytest.raises(BudgetExhausted):
            b.call(failing)
    assert b.snapshot()["retry_tokens"] < 1
    for _ in range(4):
        b.call(lambda timeout: "ok")
    assert b.snapshot()["retry_tokens"] >= 1


# ── Fault injection: concurrent requests against a slow, failing provider ────

def test_in_flight_requests_never_exceed_the_cap_under_concurrency():
    b = RetryBudget(RetryPolicy(max_in_flight=4, max_attempts=3, retry_bucket_size=1_000),
                    sleep=lambda s: None)
    lock = threading.Lock()
    observed = {"now": 0, "peak": 0}

    def slow_failing(timeout):
        with lock:
            observed["now"] += 1
            observed["peak"] = max(observed["peak"], observed["now"])
        time.sleep(0.01)
        with lock:
            observed["now"] -= 1
        raise ConnectionError("provider down")

    def worker():
        for _ in range(5):
            try:
                b.call(slow_failing)
            except BudgetExhausted:
                pass

    threads = [threading.Thread(target=worker) for _ in range(32)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    snap = b.snapshot()
    assert observed["peak"] <= 4
    assert snap["peak_in_flight"] <= 4
    assert snap["in_flight"] == 0
    assert snap["rejected"]["in_flight"] > 0
    assert snap["attempts"] <= b.policy.max_attempts_for(snap["calls"])


def test_rejection_codes_are_exported_as_bounded_metric_labels():
    b, _ = budget()
    with pytest.raises(BudgetExhausted):
        b.call(failing)
    text = b.prometheus()
    for code in BudgetCode:
        assert f'rpc_budget_rejected_total{{code="{code.value}"}}' in text
