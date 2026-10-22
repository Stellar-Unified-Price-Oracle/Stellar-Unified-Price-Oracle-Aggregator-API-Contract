"""Human-readable rendering of a privileged payload (#686).

A reviewer cannot meaningfully approve a base64 envelope. The rendering below
is the artifact a person actually reads, and it is stored verbatim in the
approval record: verification regenerates it from the payload and refuses an
approval whose stored text does not match. That closes the gap where a
reviewer signs off on a summary that was produced from a *different* payload.
"""

from __future__ import annotations

from services.tx_review.payload import Payload

MAX_RENDER_LINES = 64
MAX_RENDER_CHARS = 4096


def render(payload: Payload) -> str:
    """Deterministic, newline-separated summary of exactly what will be signed."""
    lines = [
        "PRIVILEGED TRANSACTION REVIEW",
        f"network: {payload.network}",
        f"source account: {payload.source_account}",
        f"fee: {payload.fee} stroops",
        f"sequence: {payload.sequence}",
        f"max time: {payload.max_time if payload.max_time else 'none'}",
        "operations:",
    ]
    for position, op in enumerate(payload.operations, start=1):
        lines.append(f"  {position}. {op.rendered()}")
    text = "\n".join(lines[:MAX_RENDER_LINES])
    return text[:MAX_RENDER_CHARS]


def describe_effect(payload: Payload) -> str:
    """One-line statement of what the payload changes, for a review comment."""
    return "; ".join(f"{op.function} on {op.contract_id}" for op in payload.operations)
