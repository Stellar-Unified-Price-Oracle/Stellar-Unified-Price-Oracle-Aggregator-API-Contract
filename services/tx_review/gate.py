"""Release-workflow gate for privileged transaction payloads (#686).

    # replay the tamper corpus (CI, no network)
    python3 -m services.tx_review.gate replay

    # show exactly what a reviewer is being asked to approve
    python3 -m services.tx_review.gate render --payload payload.json

    # record an approval after two humans have read the rendering
    python3 -m services.tx_review.gate approve \
        --payload payload.json --simulation simulation.json \
        --requested-by ops-bot --reviewer alice --reviewer bob \
        --reviewed-at 2026-10-20T09:00:00Z --expires-at 2026-10-20T10:00:00Z \
        --out approval.json

    # the release workflow refuses to release unless this prints APPROVED
    python3 -m services.tx_review.gate verify \
        --payload payload.json --approval approval.json --environment production

The release workflow calls ``verify``: a payload that differs from its reviewed
simulation exits non-zero, so it cannot be approved by the workflow at all.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import sys
from pathlib import Path
from typing import Optional, Sequence

from services.tx_review.cases import load_manifest, render_report, run
from services.tx_review.payload import PayloadError, load_payload
from services.tx_review.render import render
from services.tx_review.review import (
    ApprovalError,
    ReviewPolicy,
    build_approval,
    verify_approval,
)


def _read_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def _timestamp(value: str) -> dt.datetime:
    parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    return parsed if parsed.tzinfo else parsed.replace(tzinfo=dt.timezone.utc)


def _cmd_replay(args: argparse.Namespace) -> int:
    results = run(load_manifest())
    for result in results:
        print(result.line())
    passed = sum(1 for r in results if r.ok)
    print(f"\n{passed}/{len(results)} tamper cases are refused or approved as documented")
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(render_report(results), encoding="utf-8")
        print(f"report written to {args.report}")
    return 0 if passed == len(results) else 1


def _cmd_render(args: argparse.Namespace) -> int:
    try:
        print(render(load_payload(_read_json(args.payload))))
    except PayloadError as error:
        print(f"REJECTED {error.to_dict()}", file=sys.stderr)
        return 1
    return 0


def _cmd_approve(args: argparse.Namespace) -> int:
    payload = _read_json(args.payload)
    simulation = _read_json(args.simulation)
    try:
        approval = build_approval(
            payload,
            simulation,
            requested_by=args.requested_by,
            reviewed_by=args.reviewer,
            reviewed_at=_timestamp(args.reviewed_at),
            expires_at=_timestamp(args.expires_at),
        )
        # Never write an approval that the execution-time check would refuse.
        verify_approval(payload, approval, policy=ReviewPolicy(environment=args.environment))
    except (ApprovalError, PayloadError) as error:
        print(f"REJECTED {error}", file=sys.stderr)
        return 1

    text = json.dumps(approval, indent=2) + "\n"
    if args.out:
        args.out.write_text(text, encoding="utf-8")
        print(f"approval written to {args.out}")
    else:
        print(text)
    print("\nReviewed summary:\n" + approval["rendered_summary"])
    return 0


def _cmd_verify(args: argparse.Namespace) -> int:
    payload = _read_json(args.payload)
    approval = _read_json(args.approval)
    policy = ReviewPolicy(
        environment=args.environment,
        required_reviewers=args.required_reviewers,
        max_fee_stroops=args.max_fee_stroops,
    )
    try:
        approved = verify_approval(
            payload,
            approval,
            policy=policy,
            now=_timestamp(args.now) if args.now else None,
        )
    except (ApprovalError, PayloadError) as error:
        print(f"REFUSED {error}", file=sys.stderr)
        return 1

    print("APPROVED")
    print(json.dumps(approved.to_dict(), indent=2))
    return 0


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Privileged transaction review gate (#686)")
    sub = parser.add_subparsers(dest="command")

    replay = sub.add_parser("replay", help="replay the tamper corpus (default)")
    replay.add_argument("--report", type=Path, default=None)
    replay.set_defaults(func=_cmd_replay)

    render_cmd = sub.add_parser("render", help="print the human-readable payload summary")
    render_cmd.add_argument("--payload", type=Path, required=True)
    render_cmd.set_defaults(func=_cmd_render)

    approve = sub.add_parser("approve", help="record a reviewed approval")
    approve.add_argument("--payload", type=Path, required=True)
    approve.add_argument("--simulation", type=Path, required=True)
    approve.add_argument("--requested-by", required=True)
    approve.add_argument("--reviewer", action="append", required=True)
    approve.add_argument("--reviewed-at", required=True)
    approve.add_argument("--expires-at", required=True)
    approve.add_argument("--environment", default="production")
    approve.add_argument("--out", type=Path, default=None)
    approve.set_defaults(func=_cmd_approve)

    verify = sub.add_parser("verify", help="refuse a payload that differs from its review")
    verify.add_argument("--payload", type=Path, required=True)
    verify.add_argument("--approval", type=Path, required=True)
    verify.add_argument("--environment", default="production")
    verify.add_argument("--required-reviewers", type=int, default=2)
    verify.add_argument("--max-fee-stroops", type=int, default=10_000_000)
    verify.add_argument("--now", default=None, help="ISO-8601 override of the check time")
    verify.set_defaults(func=_cmd_verify)

    args = parser.parse_args(argv)
    if not getattr(args, "command", None):
        # Bare invocation is the CI path.
        return _cmd_replay(parser.parse_args(["replay"]))
    return args.func(args)


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
