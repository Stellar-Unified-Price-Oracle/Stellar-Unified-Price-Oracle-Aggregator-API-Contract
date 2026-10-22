"""OIDC trust-policy evaluation and secret-hygiene audit (#524).

The trust relationship between a GitHub Actions workflow and a deploy role is
the security boundary that replaces a stored credential, so it is expressed as
data (:func:`deploy_policies`) and evaluated by code (:func:`evaluate`) rather
than configured by hand and hoped over. See ``docs/secretless-ci.md``.
"""
from __future__ import annotations

import argparse
import fnmatch
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Sequence, Tuple

UPSTREAM_REPO = (
    "Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract"
)

#: Claims a trust policy MUST pin. Each one closes a distinct way an
#: unintended workflow could otherwise assume the role:
#:   repository    — a fork cannot mint a token the role accepts;
#:   workflow_ref  — an unrelated workflow in the *same* repo cannot either;
#:   environment   — a job outside the reviewer-gated environment cannot.
REQUIRED_CLAIM_MATCHES = ("repository", "workflow_ref", "environment")


@dataclass(frozen=True)
class ClaimMatcher:
    """An exact claim requirement.

    ``values`` is a glob set (``fnmatch`` syntax, so a prefix wildcard such as
    ``refs/tags/*`` is expressible), e.g.
    ``ClaimMatcher("repository", ("Stellar-Unified-Price-Oracle/*",))``.
    """

    claim: str
    values: Tuple[str, ...]

    def matches(self, claims: Dict[str, str]) -> bool:
        actual = claims.get(self.claim)
        if actual is None:
            return False
        return any(fnmatch.fnmatchcase(actual, v) for v in self.values)

    def describe(self) -> str:
        return f"{self.claim} in {{{', '.join(self.values)}}}"


@dataclass(frozen=True)
class TrustPolicy:
    """The OIDC trust relationship for one deploy role."""

    role: str
    matchers: Tuple[ClaimMatcher, ...]
    #: Human-readable justification, surfaced in docs and audit output.
    rationale: str = ""

    def pinned_claims(self) -> Tuple[str, ...]:
        return tuple(sorted(m.claim for m in self.matchers))

    def validate(self) -> List[str]:
        """Returns the reasons this policy is unsafe (empty == safe)."""
        errors: List[str] = []
        pinned = set(self.pinned_claims())
        for claim in REQUIRED_CLAIM_MATCHES:
            if claim not in pinned:
                errors.append(f"role {self.role}: trust policy does not pin '{claim}'")
        for m in self.matchers:
            if not m.values:
                errors.append(f"role {self.role}: matcher for '{m.claim}' has no values")
            elif all(v == "*" for v in m.values):
                # A bare wildcard pins nothing at all and is worse than not
                # listing the claim, because it reads as if it were scoped.
                errors.append(
                    f"role {self.role}: matcher for '{m.claim}' is a bare wildcard "
                    f"and pins nothing"
                )
        return errors

    def to_dict(self) -> dict:
        return {
            "role": self.role,
            "rationale": self.rationale,
            "matchers": [{"claim": m.claim, "values": list(m.values)} for m in self.matchers],
        }


def evaluate(policy: TrustPolicy, claims: Dict[str, str]) -> Tuple[bool, str]:
    """Returns ``(allowed, reason)`` for a token carrying these claims."""
    errors = policy.validate()
    if errors:
        return False, f"policy is unsafe: {errors[0]}"
    for m in policy.matchers:
        if not m.matches(claims):
            actual = claims.get(m.claim, "<absent>")
            return False, f"claim {m.claim}={actual!r} does not satisfy {m.describe()}"
    return True, "all pinned claims satisfied"


def deploy_policies() -> Tuple[TrustPolicy, ...]:
    """The roles CI assumes, and the tight subject scope each one requires.

    ``price-oracle-testnet-deploy`` covers testnet canary / lifecycle jobs and
    is scoped to this repository, to the named workflow files on ``main``, and
    to the reviewer-gated ``testnet`` environment.

    ``price-oracle-mainnet-deploy`` covers mainnet promotion only: same pinning
    plus the ``production`` environment, so the approval gate applies.
    """
    lifecycle = f"{UPSTREAM_REPO}/.github/workflows/testnet-lifecycle.yml@refs/heads/main"
    canary = f"{UPSTREAM_REPO}/.github/workflows/canary.yml@refs/heads/main"
    return (
        TrustPolicy(
            role="price-oracle-testnet-deploy",
            matchers=(
                ClaimMatcher("repository", (UPSTREAM_REPO,)),
                ClaimMatcher("workflow_ref", (lifecycle, canary)),
                ClaimMatcher("environment", ("testnet",)),
            ),
            rationale=(
                "Testnet deploys are performed by the lifecycle or canary "
                "workflow, from main, acting on the reviewer-gated testnet "
                "environment only."
            ),
        ),
        TrustPolicy(
            role="price-oracle-mainnet-deploy",
            matchers=(
                ClaimMatcher("repository", (UPSTREAM_REPO,)),
                ClaimMatcher("workflow_ref", (canary,)),
                ClaimMatcher("environment", ("production",)),
            ),
            rationale=(
                "Mainnet deploys additionally require the production "
                "environment, which is configured with required reviewers."
            ),
        ),
    )


def sample_claims(**overrides: str) -> Dict[str, str]:
    """A well-formed token from ``testnet-lifecycle.yml`` on main.

    Tests override one claim at a time to build the near-miss corpus.
    """
    claims = {
        "sub": f"repo:{UPSTREAM_REPO}:environment:testnet",
        "repository": UPSTREAM_REPO,
        "ref": "refs/heads/main",
        "environment": "testnet",
        "workflow": "Testnet Lifecycle",
        "workflow_ref": (
            f"{UPSTREAM_REPO}/.github/workflows/testnet-lifecycle.yml@refs/heads/main"
        ),
        "event_name": "push",
        "actor": "a-maintainer",
        "ref_protected": "true",
    }
    claims.update(overrides)
    return claims


# --------------------------------------------------------------------------
# Secret inventory and scan
# --------------------------------------------------------------------------


@dataclass(frozen=True)
class SecretUse:
    """One ``secrets.NAME`` reference found in the repository."""

    name: str
    path: str
    line: int
    line_text: str

    def to_dict(self) -> dict:
        return {"name": self.name, "path": self.path, "line": self.line}


#: ``${{ secrets.NAME }}`` / ``secrets.NAME`` in any CI or script file.
_SECRET_REF = re.compile(r"secrets\.([A-Z0-9_]+)")

#: Files scanned for secret references. ``.github/workflows`` is the CI
#: configuration proper; the rest are the deploy/ops entrypoints that CI runs.
SCAN_GLOBS = (
    ".github/workflows/*.yml",
    ".github/workflows/*.yaml",
    "scripts/*.sh",
    "scripts/*.py",
    "Makefile",
)

#: Long-lived credentials this repository must not carry. Each entry is a
#: pattern over the secret's *name*; the classification is the migration
#: record required by the issue ("enumerate current CI/deploy secrets and
#: classify by necessity").
RETIRED_SECRET_PATTERNS = (
    "CANARY_SECRET_KEY",
    "*_PRIVATE_KEY",
    "*_SECRET_KEY",
    "DEPLOY_*",
    "*_API_KEY",
    "*_ACCESS_KEY*",
    "*_TOKEN",
)

#: Short-lived / non-credential references that are fine to keep: a metrics
#: push endpoint is a URL with credentials-in-path managed outside the repo,
#: and the OIDC role ARN is an identifier, not a secret.
ALLOWED_SECRET_NAMES = (
    "SOAK_PROMETHEUS_PUSH_URL",
    "DEPLOY_ROLE_ARN",
    "AWS_ROLE_ARN",
)


def is_retired(name: str) -> bool:
    if name in ALLOWED_SECRET_NAMES:
        return False
    return any(fnmatch.fnmatchcase(name, p) for p in RETIRED_SECRET_PATTERNS)


def iter_scannable(root: Path) -> Iterable[Path]:
    for pattern in SCAN_GLOBS:
        yield from sorted(root.glob(pattern))


def find_long_lived_secrets(root: Path) -> List[SecretUse]:
    """Every long-lived credential reference still present under ``root``."""
    found: List[SecretUse] = []
    for path in iter_scannable(root):
        try:
            text = path.read_text()
        except (UnicodeDecodeError, OSError):  # pragma: no cover - defensive
            continue
        for n, line in enumerate(text.splitlines(), start=1):
            for name in _SECRET_REF.findall(line):
                if is_retired(name):
                    found.append(
                        SecretUse(
                            name=name,
                            path=str(path.relative_to(root)),
                            line=n,
                            line_text=line.strip(),
                        )
                    )
    return found


def secret_inventory(root: Path) -> Dict[str, List[str]]:
    """All ``secrets.*`` references, classified retired vs. allowed."""
    retired: Dict[str, List[str]] = {}
    allowed: Dict[str, List[str]] = {}
    for path in iter_scannable(root):
        try:
            text = path.read_text()
        except (UnicodeDecodeError, OSError):  # pragma: no cover - defensive
            continue
        rel = str(path.relative_to(root))
        for n, line in enumerate(text.splitlines(), start=1):
            for name in _SECRET_REF.findall(line):
                bucket = retired if is_retired(name) else allowed
                bucket.setdefault(name, []).append(f"{rel}:{n}")
    return {"retired": retired, "allowed": allowed}


# --------------------------------------------------------------------------
# Attributable deployments
# --------------------------------------------------------------------------


@dataclass(frozen=True)
class DeploymentAttestation:
    """Binds one deployment to a commit and a workflow run.

    Emitted by the deploy job after the OIDC exchange, and stored alongside
    the release attestation. This is the "every deployment is attributable to
    a commit and workflow run" requirement: the federation token itself is
    ephemeral, so the durable record has to be written by the job.
    """

    repository: str
    commit_sha: str
    workflow: str
    workflow_ref: str
    run_id: str
    run_attempt: str
    environment: str
    role: str
    #: Digest of the artifact that was deployed.
    artifact_sha256: str
    actor: str = ""

    def __post_init__(self) -> None:
        if not _is_hex_sha(self.commit_sha):
            raise ValueError(f"commit_sha must be a 40-char hex sha, got {self.commit_sha!r}")
        if not _is_hex_sha(self.artifact_sha256):
            raise ValueError(
                f"artifact_sha256 must be a 64-char hex digest, got {self.artifact_sha256!r}"
            )
        if not self.run_id:
            raise ValueError("run_id is required to attribute a deployment")

    def to_dict(self) -> dict:
        return {
            "repository": self.repository,
            "commit_sha": self.commit_sha,
            "workflow": self.workflow,
            "workflow_ref": self.workflow_ref,
            "run_id": self.run_id,
            "run_attempt": self.run_attempt,
            "environment": self.environment,
            "role": self.role,
            "artifact_sha256": self.artifact_sha256,
            "actor": self.actor,
        }

    def as_github_step_summary(self) -> str:
        """The record as it is written into the run summary."""
        return "\n".join(
            [
                "### Deployment attestation",
                "",
                f"- repository: `{self.repository}`",
                f"- commit: `{self.commit_sha}`",
                f"- workflow: `{self.workflow_ref}`",
                f"- run: `{self.run_id}/{self.run_attempt}`",
                f"- environment: `{self.environment}`",
                f"- role: `{self.role}` (OIDC-federated, short-lived)",
                f"- artifact sha256: `{self.artifact_sha256}`",
                f"- actor: `{self.actor or 'unattributed'}`",
            ]
        )


def _is_hex_sha(value: str) -> bool:
    if not value or len(value) not in (40, 64):
        return False
    try:
        int(value, 16)
    except ValueError:
        return False
    return True


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------


def audit(root: Path) -> List[str]:
    """Returns every problem found; empty list means the policy holds."""
    problems: List[str] = []
    for policy in deploy_policies():
        problems += policy.validate()
    for use in find_long_lived_secrets(root):
        problems.append(
            f"long-lived secret {use.name} referenced at {use.path}:{use.line}"
        )
    return problems


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = argparse.ArgumentParser(description="OIDC trust-policy and secret audit")
    p.add_argument("--root", type=Path, default=Path("."), help="repository root")
    p.add_argument(
        "--print-policies", action="store_true", help="emit the trust policies as JSON"
    )
    args = p.parse_args(argv)

    if args.print_policies:
        print(json.dumps([pol.to_dict() for pol in deploy_policies()], indent=2))
        return 0

    problems = audit(args.root)
    inventory = secret_inventory(args.root)
    print("Secret inventory:")
    print(f"  retired (long-lived): {sorted(inventory['retired']) or 'none'}")
    print(f"  allowed (short-lived/identifier): {sorted(inventory['allowed']) or 'none'}")
    print("Trust policies:")
    for pol in deploy_policies():
        print(f"  {pol.role}: {', '.join(m.describe() for m in pol.matchers)}")
    if problems:
        for problem in problems:
            print(f"AUDIT FAIL: {problem}", file=sys.stderr)
        return 1
    print("AUDIT PASS: no long-lived deploy secret, all trust policies scoped.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
