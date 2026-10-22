"""Validates the network, ledger and transaction fields of Soroban RPC responses.

The threat this closes (#684): the operator tooling decides *what to do* from
RPC replies — which network answered, which ledger head was observed, whether a
submitted transaction is in a block, whether an operation is safe to retry. A
compromised, misconfigured or merely lagging endpoint that returns the wrong
passphrase, a stale ledger or a transaction from a different ledger can push
that tooling into a wrong operational decision long before the contract ever
sees a transaction.

The validator is a trust boundary, not a transport:

* nothing is accepted without a policy (passphrase, protocol window, ledger
  freshness), and every check fails closed with a **typed, bounded** error;
* a validated network is a precondition for validating a ledger, and a
  validated ledger is a precondition for validating a transaction, so a caller
  cannot accidentally act on a transaction from an unvalidated context;
* rejection details are sanitised and truncated, and never echo the payload.
  Untrusted strings do not reach a log line, a metric label, or an incident
  artifact — the log-injection half of the same trust boundary (#687);
* the set of error codes is a closed enum, so it is safe as a metric label and
  its cardinality is bounded by construction.

The module is pure: no network I/O, no clock, no dependency outside the
standard library, so the whole policy is exercised by fixtures in
``services/rpc_guard/fixtures`` and replayed by ``services.rpc_guard.gate``.
"""

from __future__ import annotations

import datetime as dt
import re
from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Callable, Dict, List, Mapping, Optional

#: Longest rejection detail we will emit. Details are diagnostic hints, never
#: payload excerpts; truncation keeps an error line (and a log line) bounded.
MAX_DETAIL = 160

_HEX64 = re.compile(r"^[0-9a-fA-F]{64}$")
_CONTROL = re.compile(r"[\x00-\x1f\x7f]")
_WHITESPACE = re.compile(r"\s+")

TESTNET_PASSPHRASE = "Test SDF Network ; September 2015"
PUBLIC_PASSPHRASE = "Public Global Stellar Network ; September 2015"

#: Soroban RPC transaction statuses (`getTransaction`). `NOT_FOUND` is a
#: distinct outcome, not a failure of a transaction, and must never be folded
#: into "success" or into "failed" by a caller.
TX_STATUSES = frozenset({"SUCCESS", "FAILED", "NOT_FOUND"})


class RpcErrorCode(str, Enum):
    """Closed set of rejection reasons (safe as a metric label)."""

    MALFORMED_RESPONSE = "malformed_response"
    RPC_ERROR = "rpc_error"
    NETWORK_MISMATCH = "network_mismatch"
    NETWORK_UNVALIDATED = "network_unvalidated"
    PROTOCOL_UNSUPPORTED = "protocol_unsupported"
    LEDGER_UNVALIDATED = "ledger_unvalidated"
    LEDGER_SEQUENCE = "ledger_sequence"
    LEDGER_HASH = "ledger_hash"
    LEDGER_TIME = "ledger_time"
    LEDGER_REGRESSION = "ledger_regression"
    LEDGER_REORG = "ledger_reorg"
    TX_STATUS = "tx_status"
    TX_HASH = "tx_hash"
    TX_LEDGER = "tx_ledger"
    TX_ENVELOPE = "tx_envelope"
    TX_TIME = "tx_time"


def _sanitise(raw: str) -> str:
    """Strips control characters, collapses whitespace, truncates."""
    text = _WHITESPACE.sub(" ", _CONTROL.sub(" ", str(raw))).strip()
    if len(text) > MAX_DETAIL:
        text = text[: MAX_DETAIL - 3] + "..."
    return text


class RpcValidationError(ValueError):
    """A response field failed its policy check.

    ``code`` is one of a closed enum, ``field`` names the offending field and
    ``detail`` is a bounded, sanitised explanation that never contains the
    rejected value.
    """

    def __init__(self, code: RpcErrorCode, field_name: str, detail: str) -> None:
        self.code = code
        self.field = _sanitise(field_name)
        self.detail = _sanitise(detail)
        super().__init__(f"{code.value} field={self.field}: {self.detail}")

    def to_dict(self) -> Dict[str, str]:
        return {"code": self.code.value, "field": self.field, "detail": self.detail}


@dataclass(frozen=True)
class NetworkPolicy:
    """What the caller believes it is talking to.

    ``network_id`` is the operator-facing name (`testnet`, `mainnet`, ...).
    ``passphrase`` must match the RPC's reported passphrase exactly: a node
    that answers with another network's passphrase is not merely surprising,
    it invalidates the meaning of every ledger it reports.
    """

    passphrase: str
    network_id: str = "testnet"
    min_protocol_version: int = 22
    max_protocol_version: int = 27
    #: How far the validated ledger head may lag a transaction's ledger before
    #: the transaction is considered stale rather than merely older.
    max_tx_age_ledgers: int = 128
    #: Tolerance between a ledger's close time and a transaction's `createdAt`.
    max_clock_skew_secs: int = 300

    @classmethod
    def testnet(cls, **kwargs: Any) -> "NetworkPolicy":
        return cls(passphrase=TESTNET_PASSPHRASE, network_id="testnet", **kwargs)

    @classmethod
    def mainnet(cls, **kwargs: Any) -> "NetworkPolicy":
        return cls(passphrase=PUBLIC_PASSPHRASE, network_id="mainnet", **kwargs)


@dataclass(frozen=True)
class NetworkState:
    """A network response that passed the policy."""

    network_id: str
    passphrase_matched: bool
    protocol_version: int


@dataclass(frozen=True)
class LedgerState:
    """A ledger header that passed the policy."""

    sequence: int
    hash: str
    protocol_version: int
    closed_at: dt.datetime
    close_time: int


@dataclass(frozen=True)
class TxState:
    """A transaction response that passed the policy."""

    hash: str
    status: str
    successful: bool
    ledger: int
    fee_charged: int
    created_at: int


def _is_int(value: Any) -> bool:
    # `bool` is an `int` subclass; `True` must not be accepted as a sequence.
    return isinstance(value, int) and not isinstance(value, bool)


def _parse_time(value: Any, field_name: str) -> dt.datetime:
    if not isinstance(value, str) or not value.strip():
        raise RpcValidationError(
            RpcErrorCode.LEDGER_TIME, field_name, "expected an ISO-8601 close time"
        )
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        raise RpcValidationError(
            RpcErrorCode.LEDGER_TIME, field_name, "close time is not valid ISO-8601"
        ) from None
    if parsed.tzinfo is None:
        raise RpcValidationError(
            RpcErrorCode.LEDGER_TIME, field_name, "close time carries no timezone"
        )
    return parsed


class RpcGuard:
    """Sequential trust boundary over one RPC conversation.

    The guards are ordered on purpose: ``accept_network`` then
    ``accept_ledger`` then ``accept_transaction``. Each returns a frozen state
    object that later calls and the caller's operational code consume, so an
    unvalidated context cannot be mistaken for a validated one.
    """

    def __init__(
        self,
        policy: NetworkPolicy,
        on_reject: Optional[Callable[[RpcValidationError], None]] = None,
    ) -> None:
        self.policy = policy
        self._on_reject = on_reject
        self._network: Optional[NetworkState] = None
        self._ledger: Optional[LedgerState] = None
        #: Rejection history for observability (bounded per-process).
        self.rejections: List[RpcValidationError] = []

    # -- state -----------------------------------------------------------
    @property
    def network(self) -> Optional[NetworkState]:
        return self._network

    @property
    def ledger(self) -> Optional[LedgerState]:
        return self._ledger

    def summary(self) -> Dict[str, Any]:
        """Bounded, JSON-safe snapshot suitable for a log line or a metric."""
        counts: Dict[str, int] = {}
        for rejection in self.rejections:
            counts[rejection.code.value] = counts.get(rejection.code.value, 0) + 1
        return {
            "network": self._network.network_id if self._network else None,
            "protocol_version": self._network.protocol_version if self._network else None,
            "ledger": self._ledger.sequence if self._ledger else None,
            "rejections": dict(sorted(counts.items())),
        }

    def _fail(self, code: RpcErrorCode, field_name: str, detail: str) -> None:
        """Records and raises one rejection. Every failure path goes through here."""
        error = RpcValidationError(code, field_name, detail)
        self.rejections.append(error)
        if self._on_reject is not None:
            self._on_reject(error)
        raise error

    def _mapping(self, response: Any, field_name: str = "response") -> Mapping[str, Any]:
        if not isinstance(response, Mapping):
            self._fail(
                RpcErrorCode.MALFORMED_RESPONSE,
                field_name,
                f"expected an object, got {type(response).__name__}",
            )
        # A JSON-RPC error object is never a valid payload, and must not be
        # mistaken for an empty result by a caller that only looks at `result`.
        if "error" in response:
            detail = "node returned a JSON-RPC error"
            err = response.get("error")
            if isinstance(err, Mapping) and _is_int(err.get("code")):
                detail = f"node returned JSON-RPC error code {err['code']}"
            self._fail(RpcErrorCode.RPC_ERROR, "error", detail)
        return response

    # -- network ---------------------------------------------------------
    def accept_network(self, response: Mapping[str, Any]) -> NetworkState:
        """Validates `getNetwork` before anything else may be trusted."""
        payload = self._mapping(response)
        passphrase = payload.get("passphrase")
        if not isinstance(passphrase, str) or not passphrase:
            self._fail(
                RpcErrorCode.MALFORMED_RESPONSE,
                "passphrase",
                "network response carries no passphrase",
            )
        if passphrase != self.policy.passphrase:
            # The expected passphrase belongs to the policy, the received one
            # to the endpoint. Neither is echoed: one is a trust anchor, the
            # other is untrusted input.
            self._fail(
                RpcErrorCode.NETWORK_MISMATCH,
                "passphrase",
                f"response is not {self.policy.network_id}",
            )

        version = payload.get("protocolVersion")
        if not _is_int(version):
            self._fail(
                RpcErrorCode.MALFORMED_RESPONSE,
                "protocolVersion",
                "network response carries no integer protocol version",
            )
        if not self.policy.min_protocol_version <= version <= self.policy.max_protocol_version:
            self._fail(
                RpcErrorCode.PROTOCOL_UNSUPPORTED,
                "protocolVersion",
                f"protocol {version} is outside the supported "
                f"{self.policy.min_protocol_version}..{self.policy.max_protocol_version}",
            )

        self._network = NetworkState(
            network_id=self.policy.network_id,
            passphrase_matched=True,
            protocol_version=version,
        )
        return self._network

    # -- ledger ----------------------------------------------------------
    def accept_ledger(self, response: Mapping[str, Any]) -> LedgerState:
        """Validates a ledger header and its continuity with the last one."""
        if self._network is None:
            self._fail(
                RpcErrorCode.NETWORK_UNVALIDATED,
                "network",
                "accept_network must succeed before a ledger is accepted",
            )
        payload = self._mapping(response)

        sequence = payload.get("sequence")
        if not _is_int(sequence) or sequence <= 0:
            self._fail(
                RpcErrorCode.LEDGER_SEQUENCE,
                "sequence",
                "ledger sequence must be a positive integer",
            )

        ledger_hash = payload.get("hash")
        if not isinstance(ledger_hash, str) or not _HEX64.match(ledger_hash):
            self._fail(
                RpcErrorCode.LEDGER_HASH,
                "hash",
                "ledger hash must be 64 hexadecimal characters",
            )

        version = payload.get("protocolVersion")
        if not _is_int(version):
            self._fail(
                RpcErrorCode.MALFORMED_RESPONSE,
                "protocolVersion",
                "ledger header carries no integer protocol version",
            )
        if version != self._network.protocol_version:
            self._fail(
                RpcErrorCode.PROTOCOL_UNSUPPORTED,
                "protocolVersion",
                f"ledger reports protocol {version}, network negotiated "
                f"{self._network.protocol_version}",
            )

        closed_at = _parse_time(payload.get("closedAt"), "closedAt")

        previous = self._ledger
        if previous is not None:
            if sequence < previous.sequence:
                self._fail(
                    RpcErrorCode.LEDGER_REGRESSION,
                    "sequence",
                    f"ledger {sequence} is behind the validated head {previous.sequence}",
                )
            if sequence == previous.sequence:
                if ledger_hash.lower() != previous.hash.lower():
                    # Same height, different contents: the node's view was
                    # reorganised under us. The caller must re-read, not merge.
                    self._fail(
                        RpcErrorCode.LEDGER_REORG,
                        "hash",
                        f"ledger {sequence} reappeared with a different hash",
                    )
                return previous
            if closed_at < previous.closed_at:
                delta = int((previous.closed_at - closed_at).total_seconds())
                self._fail(
                    RpcErrorCode.LEDGER_TIME,
                    "closedAt",
                    f"close time moved backwards by {delta}s at ledger {sequence}",
                )

        state = LedgerState(
            sequence=sequence,
            hash=ledger_hash,
            protocol_version=version,
            closed_at=closed_at,
            close_time=int(closed_at.timestamp()),
        )
        self._ledger = state
        return state

    # -- transaction -----------------------------------------------------
    def accept_transaction(
        self,
        response: Mapping[str, Any],
        expected_hash: Optional[str] = None,
    ) -> TxState:
        """Validates `getTransaction` against the accepted ledger."""
        if self._ledger is None:
            self._fail(
                RpcErrorCode.LEDGER_UNVALIDATED,
                "ledger",
                "accept_ledger must succeed before a transaction is accepted",
            )
        payload = self._mapping(response)

        status = payload.get("status")
        if not isinstance(status, str) or status not in TX_STATUSES:
            self._fail(
                RpcErrorCode.TX_STATUS,
                "status",
                "status must be SUCCESS, FAILED or NOT_FOUND",
            )
        if status == "NOT_FOUND":
            # Explicitly a rejection: a caller retrying on `NOT_FOUND` is
            # correct, a caller reading it as "done" is not.
            self._fail(
                RpcErrorCode.TX_STATUS,
                "status",
                "transaction is not in the validated ledger yet",
            )

        tx_hash = payload.get("hash")
        if not isinstance(tx_hash, str) or not _HEX64.match(tx_hash):
            self._fail(
                RpcErrorCode.TX_HASH,
                "hash",
                "transaction hash must be 64 hexadecimal characters",
            )
        if expected_hash is not None and tx_hash.lower() != expected_hash.lower():
            self._fail(
                RpcErrorCode.TX_HASH,
                "hash",
                "response hash does not match the submitted transaction",
            )

        ledger = payload.get("ledger")
        if not _is_int(ledger) or ledger <= 0:
            self._fail(
                RpcErrorCode.TX_LEDGER,
                "ledger",
                "transaction ledger must be a positive integer",
            )
        head = self._ledger.sequence
        if ledger > head:
            self._fail(
                RpcErrorCode.TX_LEDGER,
                "ledger",
                f"transaction is in ledger {ledger}, beyond the validated head {head}",
            )
        if head - ledger > self.policy.max_tx_age_ledgers:
            self._fail(
                RpcErrorCode.TX_LEDGER,
                "ledger",
                f"transaction is {head - ledger} ledgers behind the validated head",
            )

        successful = payload.get("successful")
        if not isinstance(successful, bool):
            self._fail(
                RpcErrorCode.MALFORMED_RESPONSE,
                "successful",
                "transaction response carries no boolean result",
            )
        if successful != (status == "SUCCESS"):
            self._fail(
                RpcErrorCode.TX_STATUS,
                "successful",
                f"result flag disagrees with status {status}",
            )

        # A successful result without its envelope and meta is not a result we
        # can attribute to the operation we submitted.
        for required in ("envelopeXdr", "resultXdr", "resultMetaXdr"):
            value = payload.get(required)
            if not isinstance(value, str) or not value:
                self._fail(
                    RpcErrorCode.TX_ENVELOPE,
                    required,
                    "response is missing the transaction envelope/meta",
                )

        created_at = payload.get("createdAt")
        if not _is_int(created_at) or created_at <= 0:
            self._fail(
                RpcErrorCode.TX_TIME,
                "createdAt",
                "transaction response carries no positive creation time",
            )
        skew = created_at - self._ledger.close_time
        if abs(skew) > self.policy.max_clock_skew_secs:
            self._fail(
                RpcErrorCode.TX_TIME,
                "createdAt",
                f"creation time is {abs(skew)}s from the validated ledger close",
            )

        fee_charged = payload.get("feeCharged", 0)
        if not _is_int(fee_charged) or fee_charged < 0:
            self._fail(
                RpcErrorCode.MALFORMED_RESPONSE,
                "feeCharged",
                "fee charged must be a non-negative integer",
            )

        return TxState(
            hash=tx_hash,
            status=status,
            successful=successful,
            ledger=ledger,
            fee_charged=fee_charged,
            created_at=created_at,
        )


@dataclass
class ValidationReport:
    """Accumulates rejections without raising, for batch/gate use."""

    rejections: List[RpcValidationError] = field(default_factory=list)

    def record(self, error: RpcValidationError) -> None:
        self.rejections.append(error)

    def codes(self) -> Dict[str, int]:
        counts: Dict[str, int] = {}
        for rejection in self.rejections:
            counts[rejection.code.value] = counts.get(rejection.code.value, 0) + 1
        return dict(sorted(counts.items()))
