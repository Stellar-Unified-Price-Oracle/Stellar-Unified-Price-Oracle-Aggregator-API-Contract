"""Retry, timeout and concurrency budget for outbound RPC calls (#696).

A naive retry loop turns a provider failure into an upstream load multiplier:
every caller that would have sent one request sends ``max_attempts`` of them,
at the exact moment the provider is least able to absorb them, and every
caller that would have waited for one slot opens another connection. This
module bounds all three dimensions of that amplification:

* **Attempts** — a call makes at most ``max_attempts`` attempts.
* **Retry volume** — retries are paid for from a shared token bucket. Every
  admitted call deposits ``retry_ratio`` tokens (capped at
  ``retry_bucket_size``) and every retry spends one, so across *all* callers
  ``retries <= retry_bucket_size + retry_ratio * calls``. During a sustained
  outage the fleet degrades to roughly one attempt per call instead of
  ``max_attempts`` per call.
* **Time** — each attempt is handed a timeout of at most
  ``attempt_timeout_secs``, never more than what remains of the call's
  ``deadline_secs``; backoff sleeps count against the same deadline.
* **Concurrency** — at most ``max_in_flight`` calls hold a slot at once. A call
  that cannot get a slot is rejected immediately rather than queued, so a slow
  provider cannot build an unbounded backlog of waiting requests.

Every rejection is a :class:`BudgetExhausted` with a closed-enum ``code``
(safe as a metric label, like :class:`~services.rpc_guard.validator.RpcErrorCode`).

The clock and sleep are injected, so the budget is exercised deterministically
by the fault-injection tests in ``services/rpc_guard/test/test_budget.py``.

See ``docs/rpc-retry-budget.md``.
"""

from __future__ import annotations

import threading
import time
from dataclasses import dataclass
from enum import Enum
from typing import Callable, Dict, TypeVar

T = TypeVar("T")

#: An outbound call: receives the timeout (seconds) it must honour.
Operation = Callable[[float], T]


class BudgetCode(str, Enum):
    """Closed set of budget rejections (safe as a metric label)."""

    IN_FLIGHT = "in_flight"
    ATTEMPTS = "attempts"
    RETRY_BUDGET = "retry_budget"
    DEADLINE = "deadline"


class BudgetExhausted(RuntimeError):
    """A call was refused or abandoned because a budget cap was reached."""

    def __init__(self, code: BudgetCode, detail: str) -> None:
        self.code = code
        self.detail = detail
        super().__init__(f"{code.value}: {detail}")


@dataclass(frozen=True)
class RetryPolicy:
    """The caps one RPC client is held to."""

    #: Total attempts per call, including the first.
    max_attempts: int = 3
    #: Timeout handed to each attempt.
    attempt_timeout_secs: float = 5.0
    #: Wall-clock budget for the whole call, attempts and backoff included.
    deadline_secs: float = 15.0
    #: Calls allowed to hold a slot concurrently.
    max_in_flight: int = 8
    #: Retry tokens deposited per admitted call (0.1 = 10 % retry overhead).
    retry_ratio: float = 0.1
    #: Retry token bucket capacity, and its initial balance.
    retry_bucket_size: float = 10.0
    base_backoff_secs: float = 0.2
    max_backoff_secs: float = 2.0

    def __post_init__(self) -> None:
        if self.max_attempts < 1:
            raise ValueError("max_attempts must be >= 1")
        if self.max_in_flight < 1:
            raise ValueError("max_in_flight must be >= 1")
        if self.attempt_timeout_secs <= 0 or self.deadline_secs <= 0:
            raise ValueError("timeouts must be positive")
        if self.retry_ratio < 0 or self.retry_bucket_size < 0:
            raise ValueError("retry budget must be non-negative")
        if self.base_backoff_secs < 0 or self.max_backoff_secs < self.base_backoff_secs:
            raise ValueError("backoff must satisfy 0 <= base <= max")

    def backoff(self, attempt: int) -> float:
        """Backoff after the ``attempt``-th failed attempt (1-based)."""
        return min(self.max_backoff_secs, self.base_backoff_secs * 2 ** (attempt - 1))

    def max_attempts_for(self, calls: int) -> float:
        """Upper bound on total attempts across ``calls`` admitted calls."""
        retries = min(
            calls * (self.max_attempts - 1),
            self.retry_bucket_size + self.retry_ratio * calls,
        )
        return calls + retries


class RetryBudget:
    """Applies a :class:`RetryPolicy` to calls, across every caller sharing it.

    One instance is shared by every caller of the same provider, because the
    point of the budget is to cap the load the *fleet* sends to that provider,
    not the load each call site sends.
    """

    def __init__(
        self,
        policy: RetryPolicy,
        clock: Callable[[], float] = time.monotonic,
        sleep: Callable[[float], None] = time.sleep,
    ) -> None:
        self.policy = policy
        self.clock = clock
        self.sleep = sleep
        self._lock = threading.Lock()
        self._tokens = policy.retry_bucket_size
        self.in_flight = 0
        self.peak_in_flight = 0
        self.calls = 0
        self.attempts = 0
        self.retries = 0
        self.rejected: Dict[BudgetCode, int] = {code: 0 for code in BudgetCode}

    # -- slots and tokens ----------------------------------------------
    def _acquire(self) -> None:
        with self._lock:
            if self.in_flight >= self.policy.max_in_flight:
                self.rejected[BudgetCode.IN_FLIGHT] += 1
                raise BudgetExhausted(
                    BudgetCode.IN_FLIGHT,
                    f"{self.in_flight} calls already in flight (cap {self.policy.max_in_flight})",
                )
            self.in_flight += 1
            self.peak_in_flight = max(self.peak_in_flight, self.in_flight)
            self.calls += 1
            self._tokens = min(
                self.policy.retry_bucket_size, self._tokens + self.policy.retry_ratio
            )

    def _release(self) -> None:
        with self._lock:
            self.in_flight -= 1

    def _take_retry_token(self) -> bool:
        with self._lock:
            if self._tokens < 1:
                return False
            self._tokens -= 1
            self.retries += 1
            return True

    def _reject(self, code: BudgetCode, detail: str) -> BudgetExhausted:
        with self._lock:
            self.rejected[code] += 1
        return BudgetExhausted(code, detail)

    # -- calls ---------------------------------------------------------
    def call(
        self,
        op: Operation,
        retryable: Callable[[BaseException], bool] = lambda e: True,
    ) -> T:
        """Runs ``op`` within the budget and returns its result.

        ``op`` receives the timeout it must apply to its own I/O. An exception
        for which ``retryable`` is false is re-raised immediately; a retryable
        one is retried only while attempts, retry tokens and the deadline all
        allow it, and otherwise surfaces as :class:`BudgetExhausted` chained to
        the last error.
        """
        policy = self.policy
        self._acquire()
        try:
            start = self.clock()
            attempt = 0
            while True:
                remaining = policy.deadline_secs - (self.clock() - start)
                if remaining <= 0:
                    raise self._reject(BudgetCode.DEADLINE, f"deadline {policy.deadline_secs}s spent")
                attempt += 1
                with self._lock:
                    self.attempts += 1
                try:
                    return op(min(policy.attempt_timeout_secs, remaining))
                except Exception as error:
                    if not retryable(error):
                        raise
                    if attempt >= policy.max_attempts:
                        raise self._reject(
                            BudgetCode.ATTEMPTS, f"{attempt} attempts failed"
                        ) from error
                    if not self._take_retry_token():
                        raise self._reject(
                            BudgetCode.RETRY_BUDGET, "shared retry budget is empty"
                        ) from error
                    remaining = policy.deadline_secs - (self.clock() - start)
                    self.sleep(max(0.0, min(policy.backoff(attempt), remaining)))
        finally:
            self._release()

    # -- observability -------------------------------------------------
    def snapshot(self) -> Dict[str, object]:
        with self._lock:
            return {
                "calls": self.calls,
                "attempts": self.attempts,
                "retries": self.retries,
                "in_flight": self.in_flight,
                "peak_in_flight": self.peak_in_flight,
                "retry_tokens": round(self._tokens, 4),
                "rejected": {code.value: n for code, n in self.rejected.items()},
            }

    def prometheus(self) -> str:
        snap = self.snapshot()
        lines = [
            "# TYPE rpc_budget_calls_total counter",
            f"rpc_budget_calls_total {snap['calls']}",
            "# TYPE rpc_budget_attempts_total counter",
            f"rpc_budget_attempts_total {snap['attempts']}",
            "# TYPE rpc_budget_retries_total counter",
            f"rpc_budget_retries_total {snap['retries']}",
            "# TYPE rpc_budget_in_flight gauge",
            f"rpc_budget_in_flight {snap['in_flight']}",
            "# TYPE rpc_budget_rejected_total counter",
        ]
        for code, n in snap["rejected"].items():  # type: ignore[union-attr]
            lines.append(f'rpc_budget_rejected_total{{code="{code}"}} {n}')
        return "\n".join(lines) + "\n"
