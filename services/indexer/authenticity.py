"""Authenticity checks for ingested oracle events (#685).

The indexer turns an untrusted event export into the price history that
provenance queries, the explorer and the anomaly/volatility pipelines read. If
an event from another contract, another network or an unrelated topic is
accepted, the resulting history is silently wrong — and because every event is
keyed by ``(ledger, event_index)``, a spoofed event is indistinguishable from a
real one once it is stored.

So authenticity is enforced *before* the first write:

* **contract identity** — the event's ``contract_id`` must match the configured
  contract, or the contract the store was first populated from (pinned once, on
  first ingest, so a store cannot silently switch identity between runs);
* **topic shape** — an optional allowlist of topics rejects anything the
  indexer was not built to interpret;
* **ledger range** — a positive ledger at or above the configured floor;
* **close-time monotonicity** — a ledger's events may not claim a close time
  earlier than the last event already indexed at or before that ledger. This is
  the cheap, local contradiction check: it catches a replayed or
  out-of-sequence export without needing a trusted clock.

A rejected event is never written and never advances the cursor. Rejections are
persisted in the ``rejected`` table with a bounded reason code, so a stuck
ingest is diagnosable instead of silent (see ``docs/indexer.md``).
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Any, Mapping, Optional, Tuple

#: Longest reason detail persisted for a rejected event.
MAX_DETAIL = 160


class AuthenticityCode(str, Enum):
    """Closed set of rejection reasons, safe as a metric label."""

    MALFORMED_EVENT = "malformed_event"
    CONTRACT_MISMATCH = "contract_mismatch"
    UNKNOWN_TOPIC = "unknown_topic"
    LEDGER_RANGE = "ledger_range"
    LEDGER_HASH_MISSING = "ledger_hash_missing"
    LEDGER_HASH_INCONSISTENT = "ledger_hash_inconsistent"
    LEDGER_TIME_REGRESSION = "ledger_time_regression"


class EventAuthenticityError(RuntimeError):
    """An event failed authentication and must not be indexed."""

    def __init__(self, code: AuthenticityCode, ledger: Any, detail: str) -> None:
        self.code = code
        self.ledger = ledger if isinstance(ledger, int) and not isinstance(ledger, bool) else None
        self.detail = detail[:MAX_DETAIL]
        super().__init__(f"{code.value} at ledger {self.ledger}: {self.detail}")

    def to_dict(self) -> dict:
        return {"code": self.code.value, "ledger": self.ledger, "detail": self.detail}


@dataclass(frozen=True)
class TrustPolicy:
    """What an event must prove before it may be indexed.

    ``expected_contract_id`` pins the oracle contract. When it is left unset the
    indexer pins the identity of the first event it ever accepts, which is the
    right default for a single-oracle deployment and still catches a later
    spoofed event.
    """

    expected_contract_id: Optional[str] = None
    min_ledger: int = 1
    require_ledger_hash: bool = False
    allowed_topics: Tuple[str, ...] = ()


@dataclass(frozen=True)
class VerifiedEvent:
    """An event that passed authenticity checks, with fields coerced once."""

    ledger: int
    event_index: int
    ledger_hash: Optional[str]
    timestamp: int
    contract_id: str
    topic: str
    data: Mapping[str, Any]


def _int(value: Any) -> Optional[int]:
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    return None


def verify_event(
    row: Mapping[str, Any],
    policy: TrustPolicy,
    pinned_contract_id: Optional[str] = None,
    last_close_time: Optional[int] = None,
    event_index: Optional[int] = None,
) -> VerifiedEvent:
    """Coerces and authenticates one raw envelope.

    ``pinned_contract_id`` is the identity the store already committed to (from
    the policy or from the first accepted event). ``last_close_time`` is the
    close time of the last event already indexed at or before this ledger.
    """
    if not isinstance(row, Mapping):
        raise EventAuthenticityError(
            AuthenticityCode.MALFORMED_EVENT, None, f"expected an object, got {type(row).__name__}"
        )

    ledger = _int(row.get("ledger"))
    if ledger is None or ledger < policy.min_ledger:
        raise EventAuthenticityError(
            AuthenticityCode.LEDGER_RANGE,
            ledger,
            f"ledger must be an integer >= {policy.min_ledger}",
        )

    raw_index = row.get("event_index")
    index = _int(raw_index if raw_index is not None else event_index)
    if index is None or index < 0:
        raise EventAuthenticityError(
            AuthenticityCode.MALFORMED_EVENT, ledger, "event_index must be a non-negative integer"
        )

    timestamp = _int(row.get("timestamp"))
    if timestamp is None or timestamp <= 0:
        raise EventAuthenticityError(
            AuthenticityCode.MALFORMED_EVENT, ledger, "timestamp must be a positive integer"
        )

    contract_id = row.get("contract_id")
    if not isinstance(contract_id, str) or not contract_id.strip():
        raise EventAuthenticityError(
            AuthenticityCode.MALFORMED_EVENT, ledger, "event carries no contract id"
        )
    expected_contract = policy.expected_contract_id or pinned_contract_id
    if expected_contract is not None and contract_id != expected_contract:
        # The received id is untrusted input and is not echoed into the store's
        # rejection record; the ledger and the divergence are enough to triage.
        raise EventAuthenticityError(
            AuthenticityCode.CONTRACT_MISMATCH,
            ledger,
            "event was emitted by a contract this store does not index",
        )

    topic = row.get("topic")
    if not isinstance(topic, str) or not topic.strip():
        raise EventAuthenticityError(
            AuthenticityCode.MALFORMED_EVENT, ledger, "event carries no topic"
        )
    if policy.allowed_topics and topic not in policy.allowed_topics:
        raise EventAuthenticityError(
            AuthenticityCode.UNKNOWN_TOPIC, ledger, "topic is not one this indexer interprets"
        )

    ledger_hash = row.get("ledger_hash")
    if ledger_hash is not None:
        if not isinstance(ledger_hash, str) or not ledger_hash.strip():
            raise EventAuthenticityError(
                AuthenticityCode.MALFORMED_EVENT, ledger, "ledger_hash must be a non-empty string"
            )
        ledger_hash = ledger_hash.strip()
    elif policy.require_ledger_hash:
        raise EventAuthenticityError(
            AuthenticityCode.LEDGER_HASH_MISSING,
            ledger,
            "ledger_hash is required to detect reorgs on this stream",
        )

    if last_close_time is not None and timestamp < last_close_time:
        raise EventAuthenticityError(
            AuthenticityCode.LEDGER_TIME_REGRESSION,
            ledger,
            f"close time is {last_close_time - timestamp}s before the last indexed ledger",
        )

    data = row.get("data")
    if not isinstance(data, Mapping):
        raise EventAuthenticityError(
            AuthenticityCode.MALFORMED_EVENT, ledger, "event data must be an object"
        )

    return VerifiedEvent(
        ledger=ledger,
        event_index=index,
        ledger_hash=ledger_hash,
        timestamp=timestamp,
        contract_id=contract_id,
        topic=topic,
        data=data,
    )
