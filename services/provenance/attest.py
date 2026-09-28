"""SLSA provenance generation and verification (#525).

See the package docstring in ``services/provenance/__init__.py`` and
``docs/release-provenance.md``.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Sequence, Tuple

STATEMENT_TYPE = "https://in-toto.io/Statement/v1"
PREDICATE_TYPE = "https://slsa.dev/provenance/v1"

#: SLSA builder identity for a GitHub Actions build. The ``?`` template is
#: expanded by ``cosign``/``gh`` at verification time with the workflow ref.
DEFAULT_BUILDER = "https://github.com/actions/runner/github-hosted"

#: The repository whose releases are attested.
UPSTREAM_REPO = (
    "Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract"
)

#: Subject name of the released contract artifact.
WASM_NAME = "price_oracle.wasm"


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def signer_identity(workflow: str, ref: str) -> str:
    """The keyless signer identity for a workflow run.

    Matches the certificate's SAN that Sigstore issues for a GitHub Actions
    OIDC identity, which is what a verifier pins against.
    """
    return f"{UPSTREAM_REPO}/{workflow}@{ref}"


@dataclass(frozen=True)
class BuildEnvironment:
    """The environment the artifact was built in.

    Recorded in the provenance predicate so a rebuild can be shown to have used
    the same toolchain, not merely the same source.
    """

    builder_id: str = DEFAULT_BUILDER
    runner_image: str = "ubuntu-latest"
    toolchain: str = "rust 1.91.0"
    target: str = "wasm32v1-none"
    build_command: str = "make build"

    def to_dict(self) -> dict:
        return asdict(self)


@dataclass(frozen=True)
class Provenance:
    """An in-toto Statement v1 carrying a SLSA v1.0 provenance predicate."""

    subject_name: str
    subject_sha256: str
    commit_sha: str
    repository: str
    workflow_ref: str
    run_id: str
    environment: BuildEnvironment = field(default_factory=BuildEnvironment)
    invocation_id: str = ""

    def _invocation(self) -> str:
        return self.invocation_id or f"{self.repository}/.github/workflows/build@refs/heads/main"

    def to_statement(self) -> dict:
        """The serialisable in-toto Statement."""
        return {
            "_type": STATEMENT_TYPE,
            "subject": [
                {
                    "name": self.subject_name,
                    "digest": {"sha256": self.subject_sha256},
                }
            ],
            "predicateType": PREDICATE_TYPE,
            "predicate": {
                "buildDefinition": {
                    "buildType": "https://slsa.dev/container-based-build/v1",
                    "externalParameters": {
                        "repository": self.repository,
                        "workflowRef": self.workflow_ref,
                        "commit": self.commit_sha,
                    },
                    "internalParameters": {},
                    "resolvedDependencies": [
                        {
                            "uri": f"git+{self.repository}@{self.commit_sha}",
                            "digest": {"gitCommit": self.commit_sha},
                        }
                    ],
                },
                "runDetails": {
                    "builder": {"id": self.environment.builder_id},
                    "metadata": {
                        "invocationId": self._invocation(),
                        "environment": self.environment.to_dict(),
                    },
                },
            },
        }

    def seal(self) -> "ProvenanceBundle":
        return ProvenanceBundle(statement=self.to_statement())


@dataclass(frozen=True)
class ProvenanceBundle:
    """A provenance statement plus the keyless signing parameters for it.

    The signature itself is produced and verified by ``cosign`` (Fulcio
    certificate + Rekor transparency-log entry). Carrying the *parameters*
    here means the release job, the CI verification job and any third party
    all derive the same expected identity, instead of each re-deriving it by
    hand and drifting.
    """

    statement: dict
    signer_identity: str = ""
    transparency_log: str = "https://rekor.sigstore.dev"

    def to_dict(self) -> dict:
        return {
            "statement": self.statement,
            "signerIdentity": self.signer_identity,
            "transparencyLog": self.transparency_log,
        }

    def digest(self) -> str:
        """Digest over the canonical statement, for CI to compare."""
        canonical = json.dumps(self.statement, sort_keys=True, separators=(",", ":"))
        return hashlib.sha256(canonical.encode()).hexdigest()


# --------------------------------------------------------------------------
# Verification
# --------------------------------------------------------------------------


@dataclass
class VerificationResult:
    """Outcome of checking a provenance bundle against a local artifact."""

    ok: bool
    errors: List[str] = field(default_factory=list)
    checked: List[str] = field(default_factory=list)

    def add(self, name: str, ok: bool, error: str = "") -> None:
        self.checked.append(name)
        if not ok:
            self.errors.append(error or f"{name} check failed")

    def report(self) -> str:
        status = "PASS" if self.ok else "FAIL"
        lines = [f"# Provenance verification — {status}", ""]
        lines += [f"- {c}" for c in self.checked]
        if self.errors:
            lines += ["", "## Errors", ""] + [f"- {e}" for e in self.errors]
        return "\n".join(lines) + "\n"


def verify(
    bundle: ProvenanceBundle,
    artifact: Optional[Path] = None,
    *,
    expected_commit: Optional[str] = None,
    expected_workflow_ref: Optional[str] = None,
    expected_signer: Optional[str] = None,
) -> VerificationResult:
    """Checks a provenance bundle without any network access.

    Verifies that the statement is well-formed, that it binds the artifact,
    the commit, the workflow and the build environment, and — when an
    artifact path is given — that the statement's subject digest equals a
    fresh hash of that artifact. This is the check CI runs to prove a
    published artifact matches a fresh build of the source.
    """
    res = VerificationResult(ok=True)
    st = bundle.statement

    res.add(
        "statement type is in-toto v1",
        st.get("_type") == STATEMENT_TYPE,
        f"_type {st.get('_type')!r} != {STATEMENT_TYPE}",
    )
    res.add(
        "predicate type is SLSA provenance v1",
        st.get("predicateType") == PREDICATE_TYPE,
        f"predicateType {st.get('predicateType')!r} != {PREDICATE_TYPE}",
    )

    subjects = st.get("subject") or []
    digest = ""
    if len(subjects) != 1:
        res.add("exactly one subject", False, f"expected 1 subject, got {len(subjects)}")
    else:
        digest = (subjects[0].get("digest") or {}).get("sha256", "")
        res.add(
            "subject carries a sha256 digest",
            len(digest) == 64,
            f"subject digest {digest!r} is not a 64-char sha256",
        )
        res.add(
            f"subject is {WASM_NAME}",
            subjects[0].get("name") == WASM_NAME,
            f"subject name {subjects[0].get('name')!r} != {WASM_NAME}",
        )

    build = (st.get("predicate") or {}).get("buildDefinition") or {}
    run = (st.get("predicate") or {}).get("runDetails") or {}
    external = build.get("externalParameters") or {}
    resolved = build.get("resolvedDependencies") or []
    env = ((run.get("metadata") or {}).get("environment")) or {}

    commit = external.get("commit", "")
    res.add(
        "build definition binds a commit",
        len(commit) == 40,
        f"externalParameters.commit {commit!r} is not a 40-char sha",
    )
    res.add(
        "resolved dependency pins the same commit",
        any(d.get("digest", {}).get("gitCommit") == commit for d in resolved),
        "resolvedDependencies does not pin the same commit as externalParameters",
    )
    res.add(
        "build definition records the repository",
        bool(external.get("repository")),
        "externalParameters.repository is missing",
    )
    res.add(
        "run details record a builder",
        bool((run.get("builder") or {}).get("id")),
        "runDetails.builder.id is missing",
    )
    for field_name in ("toolchain", "target", "build_command", "runner_image"):
        res.add(
            f"environment records {field_name}",
            bool(env.get(field_name)),
            f"runDetails environment is missing {field_name}",
        )

    if expected_commit is not None:
        res.add(
            "commit matches the release",
            commit == expected_commit,
            f"attested commit {commit} != release commit {expected_commit}",
        )
    if expected_workflow_ref is not None:
        res.add(
            "workflow ref matches the release",
            external.get("workflowRef") == expected_workflow_ref,
            f"attested workflowRef {external.get('workflowRef')!r} != "
            f"{expected_workflow_ref!r}",
        )
    if expected_signer is not None:
        res.add(
            "signer identity matches the release workflow",
            bundle.signer_identity == expected_signer,
            f"signer identity {bundle.signer_identity!r} != {expected_signer!r}",
        )

    if artifact is not None:
        if not artifact.exists():
            res.add("artifact exists", False, f"{artifact} does not exist")
        else:
            actual = sha256_file(artifact)
            res.add(
                "artifact matches the attested digest",
                actual == digest,
                f"fresh build digest {actual} != attested {digest}",
            )

    res.ok = not res.errors
    return res


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------


def load_bundle(path: Path) -> ProvenanceBundle:
    payload = json.loads(path.read_text())
    return ProvenanceBundle(
        statement=payload["statement"],
        signer_identity=payload.get("signerIdentity", ""),
        transparency_log=payload.get("transparencyLog", "https://rekor.sigstore.dev"),
    )


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = argparse.ArgumentParser(description="Generate or verify SLSA provenance")
    sub = p.add_subparsers(dest="cmd", required=True)

    gen = sub.add_parser("generate", help="emit a provenance bundle for a built artifact")
    gen.add_argument("--artifact", type=Path, required=True)
    gen.add_argument("--commit", required=True)
    gen.add_argument("--repository", default=UPSTREAM_REPO)
    gen.add_argument("--workflow-ref", required=True)
    gen.add_argument("--run-id", required=True)
    gen.add_argument("--tag", default="", help="git tag, e.g. v1.2.3")
    gen.add_argument("--out", type=Path, required=True)

    chk = sub.add_parser("verify", help="verify a bundle against a fresh build")
    chk.add_argument("--bundle", type=Path, required=True)
    chk.add_argument("--artifact", type=Path, help="freshly built artifact to compare")
    chk.add_argument("--commit", help="expected commit")
    chk.add_argument("--workflow-ref", help="expected workflow ref")
    chk.add_argument("--signer", help="expected keyless signer identity")
    chk.add_argument("--report", type=Path)

    args = p.parse_args(argv)

    if args.cmd == "generate":
        if not args.artifact.exists():
            print(f"generate: {args.artifact} does not exist", file=sys.stderr)
            return 1
        ref = f"refs/tags/{args.tag}" if args.tag else "refs/heads/main"
        workflow = args.workflow_ref.split("@", 1)[0].rsplit("/", 1)[-1]
        prov = Provenance(
            subject_name=WASM_NAME,
            subject_sha256=sha256_file(args.artifact),
            commit_sha=args.commit,
            repository=args.repository,
            workflow_ref=args.workflow_ref,
            run_id=args.run_id,
        )
        bundle = ProvenanceBundle(
            statement=prov.to_statement(),
            signer_identity=signer_identity(workflow, ref),
        )
        args.out.write_text(json.dumps(bundle.to_dict(), indent=2) + "\n")
        print(f"wrote {args.out} (subject sha256 {prov.subject_sha256})")
        return 0

    result = verify(
        load_bundle(args.bundle),
        args.artifact,
        expected_commit=args.commit,
        expected_workflow_ref=args.workflow_ref,
        expected_signer=args.signer,
    )
    report = result.report()
    print(report)
    if args.report:
        args.report.write_text(report)
    return 0 if result.ok else 1


if __name__ == "__main__":
    sys.exit(main())
