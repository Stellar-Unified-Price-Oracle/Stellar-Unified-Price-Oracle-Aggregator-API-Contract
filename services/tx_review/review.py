"""Approval verification for production privileged transactions (#686).

The release workflow may only act on a payload that is *provably* the one that
was simulated and independently reviewed. This module is that proof, and it
fails closed on every gap:

1. **The payload is canonical** — see ``services.tx_review.payload``; unknown
   fields, untyped arguments and unmodelled functions are rejected before any
   approval is looked at.
2. **The digest matches.** The approval is bound to a SHA-256 of the canonical
   payload. Changing one argument, one address, the source account, the fee or
   the operation order changes the digest, so a "reviewed, then edited" payload
   cannot be approved.
3. **The reviewed summary matches.** The human-readable text the reviewers saw
   is regenerated from the payload; if the stored summary describes a different
   call, the approval is refused even when the digest was copied correctly.
4. **The simulation is the same payload's.** The simulation artifact carries its
   own digest and network, must report success, and its fee must be within the
   configured cap. A simulation of a cheaper or different call cannot be
   substituted.
5. **Independent review.** At least ``required_reviewers`` distinct reviewers,
   none of whom requested the transaction. Two for production; the reviewer
   count is a policy floor, not a suggestion.
6. **Freshness.** ``reviewed_at``/``expires_at`` are checked against the clock,
   and the review window itself is capped, so an old approval cannot be
   replayed against a re-created sequence.
"""

from __future__ import annotations

import datetime as dt
from dataclasses import dataclass
from enum import Enum
from typing import Any, Mapping, Optional, Sequence, Tuple

from services.tx_review.payload import Payload, PayloadError, load_payload
from services.tx_review.render import render

APPROVAL_SCHEMA = "oracle.tx_review.approval.v1"

#: Environments where the full guard applies. A non-production environment
#: still needs an independent reviewer, just not two.
GUARDED_ENVIRONMENTS = frozenset({"production", "mainnet"})
ENVIRONMENTS = GUARDED_ENVIRONMENTS | {"staging", "testnet", "development"}

APPROVAL_FIELDS = frozenset(
    {
        "schema",
        "payload_sha256",
        "rendered_summary",
        "requested_by",
        "reviewed_by",
        "reviewed_at",
        "expires_at",
        "simulation",
        "notes",
    }
)
SIMULATION_FIELDS = frozenset({"payload_sha256", "network", "success", "fee_stroops"})


class ReviewCode(str, Enum):
    MALFORMED_APPROVAL = "malformed_approval"
    UNSUPPORTED_SCHEMA = "unsupported_schema"
    UNKNOWN_FIELD = "approval_unknown_field"
    PAYLOAD_MISMATCH = "payload_mismatch"
    SUMMARY_MISMATCH = "summary_mismatch"
    SIMULATION_MISSING = "simulation_missing"
    SIMULATION_MISMATCH = "simulation_mismatch"
    SIMULATION_FAILED = "simulation_failed"
    FEE_ABOVE_CAP = "fee_above_cap"
    INSUFFICIENT_REVIEWERS = "insufficient_reviewers"
    SELF_REVIEW = "self_review"
    APPROVAL_EXPIRED = "approval_expired"
    APPROVAL_NOT_YET_VALID = "approval_not_yet_valid"
    REVIEW_WINDOW_TOO_LONG = "review_window_too_long"
    ENVIRONMENT_NOT_GUARDED = "environment_not_guarded"


class ApprovalError(ValueError):
    """An approval that does not cover the payload being released."""

    def __init__(self, code: ReviewCode, field_name: str, detail: str) -> None:
        self.code = code
        self.field = field_name
        self.detail = detail[:160]
        super().__init__(f"{code.value} field={self.field}: {self.detail}")

    def to_dict(self) -> dict:
        return {"code": self.code.value, "field": self.field, "detail": self.detail}


@dataclass(frozen=True)
class ReviewPolicy:
    """The release workflow's review requirements."""

    environment: str = "production"
    required_reviewers: int = 2
    max_review_window_secs: int = 86_400
    max_fee_stroops: int = 10_000_000

    def __post_init__(self) -> None:
        if self.environment not in ENVIRONMENTS:
            raise ValueError(f"unknown environment {self.environment!r}")
        floor = 2 if self.environment in GUARDED_ENVIRONMENTS else 1
        if self.required_reviewers < floor:
            # The floor is the point of the guard, so a policy that lowers it
            # is invalid rather than merely unusual.
            raise ValueError(
                f"{self.environment} requires at least {floor} independent reviewers"
            )
        if self.max_review_window_secs <= 0 or self.max_fee_stroops < 0:
            raise ValueError("review window and fee cap must be positive")


@dataclass(frozen=True)
class Approved:
    """The verified facts the release workflow may act on."""

    payload: Payload
    digest: str
    reviewers: Tuple[str, ...]
    reviewed_at: dt.datetime
    expires_at: dt.datetime
    environment: str

    def to_dict(self) -> dict:
        return {
            "digest": self.digest,
            "reviewers": list(self.reviewers),
            "reviewed_at": self.reviewed_at.isoformat(),
            "expires_at": self.expires_at.isoformat(),
            "environment": self.environment,
        }


def _utc_now() -> dt.datetime:
    return dt.datetime.now(dt.timezone.utc)


def _parse_time(value: Any, field_name: str, code: ReviewCode) -> dt.datetime:
    if not isinstance(value, str) or not value.strip():
        raise ApprovalError(code, field_name, "expected an ISO-8601 timestamp")
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        raise ApprovalError(code, field_name, "timestamp is not valid ISO-8601") from None
    if parsed.tzinfo is None:
        raise ApprovalError(code, field_name, "timestamp carries no timezone")
    return parsed


def verify_approval(
    payload_data: Any,
    approval: Mapping[str, Any],
    policy: Optional[ReviewPolicy] = None,
    now: Optional[dt.datetime] = None,
) -> Approved:
    """Verifies that `approval` covers exactly `payload_data`.

    Raises ``PayloadError`` if the payload itself is not reviewable and
    ``ApprovalError`` if the approval does not cover it.
    """
    policy = policy or ReviewPolicy()
    payload = load_payload(payload_data)

    if not isinstance(approval, Mapping):
        raise ApprovalError(ReviewCode.MALFORMED_APPROVAL, "approval", "expected an object")
    if approval.get("schema") != APPROVAL_SCHEMA:
        raise ApprovalError(
            ReviewCode.UNSUPPORTED_SCHEMA, "schema", "unsupported approval schema"
        )
    unknown = set(approval) - APPROVAL_FIELDS
    if unknown:
        raise ApprovalError(
            ReviewCode.UNKNOWN_FIELD,
            sorted(unknown)[0],
            "approval carries fields the guard does not check",
        )

    digest = payload.digest()

    stored_digest = approval.get("payload_sha256")
    if not isinstance(stored_digest, str) or not stored_digest:
        raise ApprovalError(
            ReviewCode.MALFORMED_APPROVAL, "payload_sha256", "approval carries no payload digest"
        )
    if stored_digest.lower() != digest:
        # The core guarantee: an edited payload keeps its old approval only if
        # the digest also changes, which it cannot without a new review.
        raise ApprovalError(
            ReviewCode.PAYLOAD_MISMATCH,
            "payload_sha256",
            "payload differs from the reviewed simulation",
        )

    stored_summary = approval.get("rendered_summary")
    if not isinstance(stored_summary, str) or not stored_summary.strip():
        raise ApprovalError(
            ReviewCode.MALFORMED_APPROVAL, "rendered_summary", "approval carries no reviewed summary"
        )
    if stored_summary.rstrip("\n") != render(payload):
        raise ApprovalError(
            ReviewCode.SUMMARY_MISMATCH,
            "rendered_summary",
            "reviewed summary does not describe this payload",
        )

    simulation = approval.get("simulation")
    if not isinstance(simulation, Mapping):
        raise ApprovalError(
            ReviewCode.SIMULATION_MISSING, "simulation", "approval carries no simulation artifact"
        )
    unknown_sim = set(simulation) - SIMULATION_FIELDS
    if unknown_sim:
        raise ApprovalError(
            ReviewCode.UNKNOWN_FIELD,
            f"simulation.{sorted(unknown_sim)[0]}",
            "simulation carries fields the guard does not check",
        )
    sim_digest = simulation.get("payload_sha256")
    if not isinstance(sim_digest, str) or sim_digest.lower() != digest:
        raise ApprovalError(
            ReviewCode.SIMULATION_MISMATCH,
            "simulation.payload_sha256",
            "simulation was produced from a different payload",
        )
    if simulation.get("network") != payload.network:
        raise ApprovalError(
            ReviewCode.SIMULATION_MISMATCH,
            "simulation.network",
            "simulation ran against a different network",
        )
    if simulation.get("success") is not True:
        raise ApprovalError(
            ReviewCode.SIMULATION_FAILED,
            "simulation.success",
            "simulation did not succeed",
        )
    sim_fee = simulation.get("fee_stroops")
    if isinstance(sim_fee, bool) or not isinstance(sim_fee, int) or sim_fee < 0:
        raise ApprovalError(
            ReviewCode.MALFORMED_APPROVAL, "simulation.fee_stroops", "fee must be a non-negative integer"
        )
    if sim_fee > policy.max_fee_stroops or payload.fee > policy.max_fee_stroops:
        raise ApprovalError(
            ReviewCode.FEE_ABOVE_CAP,
            "simulation.fee_stroops",
            f"fee exceeds the {policy.max_fee_stroops} stroop cap",
        )

    requested_by = approval.get("requested_by")
    if not isinstance(requested_by, str) or not requested_by.strip():
        raise ApprovalError(
            ReviewCode.MALFORMED_APPROVAL, "requested_by", "approval carries no requester"
        )
    reviewers = approval.get("reviewed_by")
    if (
        not isinstance(reviewers, Sequence)
        or isinstance(reviewers, str)
        or not all(isinstance(r, str) and r.strip() for r in reviewers)
    ):
        raise ApprovalError(
            ReviewCode.MALFORMED_APPROVAL, "reviewed_by", "expected a list of reviewer ids"
        )
    distinct = {r.strip() for r in reviewers}
    if len(distinct) != len(reviewers):
        raise ApprovalError(
            ReviewCode.INSUFFICIENT_REVIEWERS, "reviewed_by", "the same reviewer is listed twice"
        )
    if requested_by.strip() in distinct:
        # Self-approval is not review; it is one person agreeing with themself.
        raise ApprovalError(
            ReviewCode.SELF_REVIEW, "reviewed_by", "the requester cannot approve their own transaction"
        )
    if len(distinct) < policy.required_reviewers:
        raise ApprovalError(
            ReviewCode.INSUFFICIENT_REVIEWERS,
            "reviewed_by",
            f"{policy.environment} requires {policy.required_reviewers} independent reviewers",
        )

    reviewed_at = _parse_time(approval.get("reviewed_at"), "reviewed_at", ReviewCode.MALFORMED_APPROVAL)
    expires_at = _parse_time(approval.get("expires_at"), "expires_at", ReviewCode.MALFORMED_APPROVAL)
    if expires_at <= reviewed_at:
        raise ApprovalError(
            ReviewCode.REVIEW_WINDOW_TOO_LONG, "expires_at", "approval expires before it was reviewed"
        )
    window = (expires_at - reviewed_at).total_seconds()
    if window > policy.max_review_window_secs:
        raise ApprovalError(
            ReviewCode.REVIEW_WINDOW_TOO_LONG,
            "expires_at",
            f"review window is longer than {policy.max_review_window_secs}s",
        )

    moment = now or _utc_now()
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=dt.timezone.utc)
    if moment < reviewed_at:
        raise ApprovalError(
            ReviewCode.APPROVAL_NOT_YET_VALID, "reviewed_at", "approval is dated in the future"
        )
    if moment > expires_at:
        raise ApprovalError(ReviewCode.APPROVAL_EXPIRED, "expires_at", "approval has expired")

    return Approved(
        payload=payload,
        digest=digest,
        reviewers=tuple(sorted(distinct)),
        reviewed_at=reviewed_at,
        expires_at=expires_at,
        environment=policy.environment,
    )


def build_approval(
    payload_data: Any,
    simulation: Mapping[str, Any],
    requested_by: str,
    reviewed_by: Sequence[str],
    reviewed_at: dt.datetime,
    expires_at: dt.datetime,
) -> dict:
    """Builds the approval record a reviewer signs, then verifies it.

    Called by the CLI after a human has read ``render(payload)``; the resulting
    record is verified before it is written, so an approval that would be
    refused at execution time is never produced.
    """
    payload = load_payload(payload_data)
    approval = {
        "schema": APPROVAL_SCHEMA,
        "payload_sha256": payload.digest(),
        "rendered_summary": render(payload),
        "requested_by": requested_by,
        "reviewed_by": list(reviewed_by),
        "reviewed_at": reviewed_at.astimezone(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
        "expires_at": expires_at.astimezone(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
        "simulation": dict(simulation),
    }
    return approval
