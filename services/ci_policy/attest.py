"""Writes the deployment attestation for a workflow run (#524).

Called by the deploy job after the OIDC exchange. The federation token is
ephemeral, so this record — commit, workflow ref, run id, environment, role and
artifact digest — is the durable evidence that a deployment is attributable.

See ``docs/secretless-ci.md``.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Optional, Sequence

from services.ci_policy.policy import DeploymentAttestation


def workflow_name(workflow_ref: str) -> str:
    """Extracts ``canary.yml`` from a ``.../canary.yml@refs/heads/main`` ref."""
    path = workflow_ref.split("@", 1)[0]
    return path.rsplit("/", 1)[-1]


def main(argv: Optional[Sequence[str]] = None) -> int:
    """Writes the deployment attestation for the current workflow run."""
    p = argparse.ArgumentParser(
        description="Record which commit and workflow run produced a deployment"
    )
    p.add_argument("--repository", required=True)
    p.add_argument("--commit", required=True, help="full 40-char commit SHA")
    p.add_argument("--workflow-ref", required=True)
    p.add_argument("--run-id", required=True)
    p.add_argument("--run-attempt", default="1")
    p.add_argument("--environment", required=True)
    p.add_argument("--role", required=True, help="federated role that was assumed")
    p.add_argument("--artifact", required=True, help="sha256 of the deployed artifact")
    p.add_argument("--actor", default="")
    p.add_argument("--out", type=Path, required=True)
    args = p.parse_args(argv)

    try:
        att = DeploymentAttestation(
            repository=args.repository,
            commit_sha=args.commit,
            workflow=workflow_name(args.workflow_ref),
            workflow_ref=args.workflow_ref,
            run_id=args.run_id,
            run_attempt=args.run_attempt,
            environment=args.environment,
            role=args.role,
            artifact_sha256=args.artifact,
            actor=args.actor,
        )
    except ValueError as e:
        print(f"attest: {e}", file=sys.stderr)
        return 1

    args.out.write_text(json.dumps(att.to_dict(), indent=2) + "\n")
    print(att.as_github_step_summary())
    return 0


if __name__ == "__main__":
    sys.exit(main())
