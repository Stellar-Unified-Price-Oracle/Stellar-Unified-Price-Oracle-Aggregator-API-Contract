"""Security contact and disclosure process validation (#690).

Checks that the private disclosure channel, response owner, acknowledgement
target and embargo in `config/security-disclosure.json` are sound and match
`SECURITY.md`, and scores a synthetic disclosure drill. The drill works on a
digest of the synthetic report: report contents are never stored or printed in
any result.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import secrets
import sys
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_POLICY = REPO_ROOT / "config" / "security-disclosure.json"
DEFAULT_SECURITY_MD = REPO_ROOT / "SECURITY.md"

# Channels whose reports are visible only to the reporter and the responders.
PRIVATE_CHANNEL_TYPES = {"github-private-vulnerability-reporting"}
PRIVATE_CHANNEL_URL = re.compile(r"^https://github\.com/[\w.-]+/[\w.-]+/security/advisories/new$")
# Public places a reporter must never be sent to.
PUBLIC_REPORT_URL = re.compile(r"https://github\.com/[\w.-]+/[\w.-]+/(issues|discussions|pulls)(/new)?\b")
TEAM_HANDLE = re.compile(r"^@[\w.-]+/[\w.-]+$")

MAX_ACK_HOURS = 72
MAX_EMBARGO_DAYS = 90

SYNTHETIC_PREFIX = "SYNTHETIC-"
SHA256_HEX = re.compile(r"^[0-9a-f]{64}$")
# Keys that would carry report contents; an observation must not contain them.
CONTENT_KEYS = {"body", "content", "description", "details", "report", "steps"}


@dataclass
class PolicyResult:
    violations: List[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.violations


@dataclass
class DrillResult:
    report_id: str
    ack_delay_hours: Optional[float]
    ack_target_hours: int
    violations: List[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.violations

    def to_dict(self) -> Dict[str, Any]:
        return {**asdict(self), "ok": self.ok}


def validate_policy(policy: Dict[str, Any], security_md: Optional[str] = None) -> PolicyResult:
    """Checks the disclosure policy and, when given, that SECURITY.md states it."""
    v: List[str] = []
    channel = policy.get("channel") or {}
    if channel.get("type") not in PRIVATE_CHANNEL_TYPES:
        v.append(f"channel type {channel.get('type')!r} is not a private disclosure channel")
    url = channel.get("url") or ""
    if not PRIVATE_CHANNEL_URL.match(url):
        v.append("channel url must be the repository's private advisory form (/security/advisories/new)")

    owner = policy.get("response_owner") or {}
    if not owner.get("role"):
        v.append("response_owner.role is missing")
    if not TEAM_HANDLE.match(owner.get("team") or ""):
        v.append("response_owner.team must be a GitHub team handle (@org/team)")

    ack = policy.get("acknowledgement_target_hours")
    if not isinstance(ack, int) or not 1 <= ack <= MAX_ACK_HOURS:
        v.append(f"acknowledgement_target_hours must be an integer from 1 to {MAX_ACK_HOURS}")
    embargo = policy.get("embargo_days")
    if not isinstance(embargo, int) or not 1 <= embargo <= MAX_EMBARGO_DAYS:
        v.append(f"embargo_days must be an integer from 1 to {MAX_EMBARGO_DAYS}")

    if security_md is not None:
        expected = {
            "channel url": url,
            "response owner role": owner.get("role"),
            "response owner team": owner.get("team"),
            "acknowledgement target": f"{ack} hours",
            "embargo": f"{embargo} days",
        }
        for what, text in expected.items():
            if text and str(text) not in security_md:
                v.append(f"SECURITY.md does not state the {what} ({text})")
        if PUBLIC_REPORT_URL.search(security_md):
            v.append("SECURITY.md links a public issue, discussion or pull request form")
    return PolicyResult(v)


def new_synthetic_report() -> Dict[str, str]:
    """A synthetic report to submit through the real channel during a drill.

    The body is random filler with no vulnerability content. Only `report_id`
    and `content_sha256` belong in the drill observation.
    """
    report_id = SYNTHETIC_PREFIX + datetime.now(timezone.utc).strftime("%Y%m%d") + "-" + secrets.token_hex(4)
    body = f"{report_id}: synthetic disclosure drill, no vulnerability. Nonce {secrets.token_hex(16)}."
    return {
        "report_id": report_id,
        "content_sha256": hashlib.sha256(body.encode("utf-8")).hexdigest(),
        "body": body,
    }


def _parse_time(value: Any) -> Optional[datetime]:
    try:
        t = datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    except ValueError:
        return None
    return t if t.tzinfo else t.replace(tzinfo=timezone.utc)


def score_drill(policy: Dict[str, Any], observation: Dict[str, Any]) -> DrillResult:
    """Scores a synthetic disclosure drill.

    `observation` records what happened, with no report contents:
    `report_id`, `content_sha256`, `submitted_at`, `acknowledged_at`,
    `acknowledged_by`, `responders` (handles allowed to read reports), and
    `access_checks` (`{"principal", "can_read"}` probes, which must include at
    least one principal outside the responders).
    """
    v: List[str] = []
    report_id = str(observation.get("report_id") or "")
    target = int(policy.get("acknowledgement_target_hours") or 0)

    if not report_id.startswith(SYNTHETIC_PREFIX):
        v.append(f"drill report_id must start with {SYNTHETIC_PREFIX}")
    leaked = sorted(CONTENT_KEYS & set(observation))
    if leaked:
        v.append(f"observation contains report contents ({', '.join(leaked)}); record only the digest")
    if not SHA256_HEX.match(str(observation.get("content_sha256") or "")):
        v.append("content_sha256 must be the hex SHA-256 of the synthetic report")

    responders = set(observation.get("responders") or [])
    if not responders:
        v.append("no responders recorded")

    submitted = _parse_time(observation.get("submitted_at"))
    acknowledged = _parse_time(observation.get("acknowledged_at")) if observation.get("acknowledged_at") else None
    delay: Optional[float] = None
    if submitted is None:
        v.append("submitted_at is missing or not ISO-8601")
    elif acknowledged is None:
        v.append("report was not acknowledged")
    else:
        delay = round((acknowledged - submitted).total_seconds() / 3600, 2)
        if delay < 0:
            v.append("acknowledged_at is before submitted_at")
        elif delay > target:
            v.append(f"acknowledged after {delay}h, target is {target}h")
    if acknowledged is not None and observation.get("acknowledged_by") not in responders:
        v.append("report was acknowledged by someone outside the responders")

    checks = observation.get("access_checks") or []
    for check in checks:
        principal, can_read = check.get("principal"), check.get("can_read")
        if principal in responders and can_read is not True:
            v.append(f"responder {principal} cannot read the report")
        if principal not in responders and can_read is not False:
            v.append(f"non-responder {principal} can read the report")
    if not any(c.get("principal") not in responders for c in checks):
        v.append("no access check from outside the responders; private access is unverified")

    return DrillResult(report_id=report_id, ack_delay_hours=delay, ack_target_hours=target, violations=v)


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Validate the security disclosure process")
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--security-md", type=Path, default=DEFAULT_SECURITY_MD)
    parser.add_argument("--drill", type=Path, help="Score a synthetic drill observation (JSON)")
    parser.add_argument("--new-synthetic", action="store_true", help="Print a new synthetic report to submit")
    args = parser.parse_args(argv)

    if args.new_synthetic:
        print(json.dumps(new_synthetic_report(), indent=2))
        return 0

    policy = json.loads(args.policy.read_text(encoding="utf-8"))
    result = validate_policy(policy, args.security_md.read_text(encoding="utf-8"))
    for msg in result.violations:
        print(f"policy: {msg}")
    ok = result.ok
    if args.drill:
        drill = score_drill(policy, json.loads(args.drill.read_text(encoding="utf-8")))
        print(json.dumps(drill.to_dict(), indent=2))
        ok = ok and drill.ok
    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
