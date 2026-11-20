"""Release artifact retention and retrieval verification (#725).

Verifies that retained release artifacts (``price_oracle.wasm``,
``price_oracle.wasm.sig``, and ``provenance.json``) can be archived into an
immutable retention store by release tag and retrieved cleanly with intact
SHA-256 digests and SLSA v1.0 provenance bindings.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
import tempfile
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Sequence

from services.provenance.attest import (
    UPSTREAM_REPO,
    WASM_NAME,
    BuildEnvironment,
    Provenance,
    ProvenanceBundle,
    load_bundle,
    sha256_file,
    signer_identity,
    verify,
)

REQUIRED_FILES: tuple[str, ...] = (
    WASM_NAME,
    f"{WASM_NAME}.sig",
    "provenance.json",
)


@dataclass(frozen=True)
class RetainedReleaseEntry:
    """Metadata recorded for a retained release tag."""

    tag: str
    commit_sha: str
    wasm_sha256: str
    signature_sha256: str
    provenance_sha256: str
    workflow_ref: str
    signer: str

    def to_dict(self) -> dict:
        return asdict(self)


@dataclass
class RetrievalCheckResult:
    """Outcome of retrieving and verifying a retained release tag."""

    tag: str
    ok: bool
    checked: List[str] = field(default_factory=list)
    errors: List[str] = field(default_factory=list)

    def add(self, label: str, passed: bool, error: str = "") -> None:
        self.checked.append(label)
        if not passed:
            self.errors.append(error or f"{label} failed")
        self.ok = not self.errors

    def to_dict(self) -> dict:
        return asdict(self)


def archive_release(
    store_root: Path,
    *,
    tag: str,
    commit_sha: str,
    wasm_bytes: bytes,
    signature_bytes: bytes,
    run_id: str = "100001",
) -> RetainedReleaseEntry:
    """Write a complete release artifact triplet into ``store_root/<tag>/``."""
    tag_dir = store_root / tag
    tag_dir.mkdir(parents=True, exist_ok=True)

    wasm_path = tag_dir / WASM_NAME
    sig_path = tag_dir / f"{WASM_NAME}.sig"
    prov_path = tag_dir / "provenance.json"

    wasm_path.write_bytes(wasm_bytes)
    sig_path.write_bytes(signature_bytes)

    wasm_digest = sha256_file(wasm_path)
    workflow_ref = f".github/workflows/release.yml@refs/tags/{tag}"
    signer = signer_identity("release.yml", f"refs/tags/{tag}")

    prov = Provenance(
        subject_name=WASM_NAME,
        subject_sha256=wasm_digest,
        commit_sha=commit_sha,
        repository=UPSTREAM_REPO,
        workflow_ref=workflow_ref,
        run_id=run_id,
        environment=BuildEnvironment(),
    )
    bundle = ProvenanceBundle(
        statement=prov.to_statement(),
        signer_identity=signer,
    )
    prov_path.write_text(
        json.dumps(bundle.to_dict(), indent=2) + "\n",
        encoding="utf-8",
    )

    entry = RetainedReleaseEntry(
        tag=tag,
        commit_sha=commit_sha,
        wasm_sha256=wasm_digest,
        signature_sha256=sha256_file(sig_path),
        provenance_sha256=sha256_file(prov_path),
        workflow_ref=workflow_ref,
        signer=signer,
    )
    (tag_dir / "retention-manifest.json").write_text(
        json.dumps(entry.to_dict(), indent=2) + "\n",
        encoding="utf-8",
    )
    return entry


def retrieve_and_verify(
    store_root: Path,
    tag: str,
    *,
    expected_entry: Optional[RetainedReleaseEntry] = None,
) -> RetrievalCheckResult:
    """Retrieve a retained release by tag and verify every artifact and binding."""
    res = RetrievalCheckResult(tag=tag, ok=True)
    tag_dir = store_root / tag

    if not tag_dir.is_dir():
        res.add("release directory exists", False, f"missing release tag dir: {tag_dir}")
        return res

    for fname in REQUIRED_FILES:
        fpath = tag_dir / fname
        res.add(
            f"retained file {fname} exists and is non-empty",
            fpath.is_file() and fpath.stat().st_size > 0,
            f"missing or empty retained file: {fpath}",
        )
    if not res.ok:
        return res

    wasm_path = tag_dir / WASM_NAME
    sig_path = tag_dir / f"{WASM_NAME}.sig"
    prov_path = tag_dir / "provenance.json"

    if expected_entry is not None:
        res.add(
            "retrieved WASM digest matches retention manifest",
            sha256_file(wasm_path) == expected_entry.wasm_sha256,
            "retrieved WASM sha256 does not match retention manifest",
        )
        res.add(
            "retrieved signature digest matches retention manifest",
            sha256_file(sig_path) == expected_entry.signature_sha256,
            "retrieved signature sha256 does not match retention manifest",
        )
        res.add(
            "retrieved provenance digest matches retention manifest",
            sha256_file(prov_path) == expected_entry.provenance_sha256,
            "retrieved provenance sha256 does not match retention manifest",
        )

    bundle = load_bundle(prov_path)
    prov_check = verify(
        bundle,
        wasm_path,
        expected_commit=expected_entry.commit_sha if expected_entry else None,
        expected_workflow_ref=expected_entry.workflow_ref if expected_entry else None,
        expected_signer=expected_entry.signer if expected_entry else None,
    )
    res.add(
        "SLSA provenance verification passes on retrieved artifact",
        prov_check.ok,
        "; ".join(prov_check.errors),
    )
    return res


def run_retention_retrieval_drill(work_dir: Path) -> Dict[str, object]:
    """Archive N historical releases and verify retrieval + tamper rejection."""
    releases = [
        ("v0.1.0", "1" * 40, b"\x00asm\x01\x00\x00\x00\x01\x04\x01\x60\x00\x00"),
        ("v0.2.0", "2" * 40, b"\x00asm\x01\x00\x00\x00\x01\x05\x01\x60\x01\x7f\x00"),
    ]
    entries: Dict[str, RetainedReleaseEntry] = {}
    results: List[Dict[str, object]] = []

    for idx, (tag, commit_sha, wasm_bytes) in enumerate(releases, start=1):
        sig_bytes = hashlib.sha256(wasm_bytes + tag.encode("utf-8")).digest()
        entry = archive_release(
            work_dir,
            tag=tag,
            commit_sha=commit_sha,
            wasm_bytes=wasm_bytes,
            signature_bytes=sig_bytes,
            run_id=str(1000 + idx),
        )
        entries[tag] = entry
        chk = retrieve_and_verify(work_dir, tag, expected_entry=entry)
        results.append(chk.to_dict())

    # Negative check 1: missing artifact in a corrupted tag directory
    corrupted_tag = "v0.1.0-missing-sig"
    archive_release(
        work_dir,
        tag=corrupted_tag,
        commit_sha="3" * 40,
        wasm_bytes=b"\x00asm\x01\x00\x00\x00",
        signature_bytes=b"sig",
    )
    (work_dir / corrupted_tag / f"{WASM_NAME}.sig").unlink()
    missing_chk = retrieve_and_verify(work_dir, corrupted_tag)

    # Negative check 2: bit-rot in retained WASM binary
    bitrot_tag = "v0.2.0-bitrot"
    bitrot_entry = archive_release(
        work_dir,
        tag=bitrot_tag,
        commit_sha="4" * 40,
        wasm_bytes=b"\x00asm\x01\x00\x00\x00\xaa",
        signature_bytes=b"sig",
    )
    (work_dir / bitrot_tag / WASM_NAME).write_bytes(b"\x00asm\x01\x00\x00\x00\xbb")
    bitrot_chk = retrieve_and_verify(work_dir, bitrot_tag, expected_entry=bitrot_entry)

    all_ok = (
        all(bool(r["ok"]) for r in results)
        and not missing_chk.ok
        and not bitrot_chk.ok
    )
    return {
        "ok": all_ok,
        "retained_releases_verified": results,
        "missing_file_rejected": not missing_chk.ok,
        "bitrot_rejected": not bitrot_chk.ok,
    }


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="Release artifact retention and retrieval verification (#725)"
    )
    parser.add_argument(
        "--dir",
        type=Path,
        default=None,
        help="Directory to use for the retention store (defaults to a temp dir)",
    )
    parser.add_argument("--out", type=Path, default=None, help="Optional JSON report path")
    args = parser.parse_args(argv)

    if args.dir is not None:
        report = run_retention_retrieval_drill(args.dir)
    else:
        with tempfile.TemporaryDirectory(prefix="oracle_retention_") as tmp:
            report = run_retention_retrieval_drill(Path(tmp))

    rendered = json.dumps(report, indent=2) + "\n"
    if args.out is not None:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(rendered, encoding="utf-8")
    else:
        sys.stdout.write(rendered)
    return 0 if report["ok"] else 1


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
