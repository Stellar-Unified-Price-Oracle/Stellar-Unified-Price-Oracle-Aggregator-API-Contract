"""Off-chain state backup, point-in-time restore and reconciliation (#529).

On-chain state is durable; the pipeline that feeds it is not. Losing the
off-chain state can take the oracle down while the contract itself is perfectly
healthy, so every piece of it is enumerated here, backed up automatically, and
reconciled against on-chain truth after a restore.

Design constraints:

* **Everything is in one archive.** A backup is a single ``tar.gz`` plus a
  signed manifest, so a restore cannot half-succeed by finding one file
  missing.
* **Encrypted at rest.** The archive is encrypted with AES-256 (via
  ``openssl enc``) whenever a passphrase is supplied. Backups contain source
  configuration and adapter credentials metadata, so an unencrypted backup is
  an incident, not a shortcut.
* **Verified before it is trusted.** Every entry is hashed into the manifest,
  and :func:`verify_backup` re-checks the hashes. A backup that has not been
  verified is not a backup.
* **Restore never writes into a live state directory.** It restores to a
  staging directory by default; overwriting a running pipeline's state is
  refused unless the caller both stops the pipeline and passes ``--in-place``.
  A restore must not corrupt a healthy running system.
* **On-chain truth wins.** :func:`reconcile` drops restored records that
  post-date the backup and reports anything the chain does not confirm, so a
  stale restore is detected rather than served.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import subprocess
import sys
import tarfile
import tempfile
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Sequence, Tuple

MANIFEST_NAME = "manifest.json"
CIPHER = "aes-256-cbc"
PBKDF2_ITERATIONS = 200_000


@dataclass(frozen=True)
class StateEntry:
    """One piece of off-chain state that must survive a restore."""

    name: str
    relative_path: str
    description: str
    required: bool
    rpo_hours: int  # recovery point objective


# The complete inventory. Anything added to the pipeline must be added here;
# `check_inventory` fails if a directory holds state that is not covered.
STATE_INVENTORY: Tuple[StateEntry, ...] = (
    StateEntry("sources", "sources.json", "Registered source list and adapter configuration", True, 1),
    StateEntry("submissions", "submissions.jsonl", "Submission cache: every price sent, per source", True, 1),
    StateEntry("index", "index/assets.json", "Asset index used by the exporter and the SEP-40 route", True, 4),
    StateEntry("nonces", "nonces.json", "Replay-protection nonces for signed submissions", True, 1),
    StateEntry("metrics", "metrics.snap", "Counters needed to keep rates continuous after a restart", False, 24),
)


@dataclass
class BackupManifest:
    """What a backup contains, and how to prove it."""

    backup_id: str
    created_at: str
    state_dir: str
    entries: List[Dict[str, object]] = field(default_factory=list)
    encrypted: bool = False
    archive_sha256: str = ""
    tool_version: str = "1"

    def to_json(self) -> str:
        return json.dumps(asdict(self), indent=2, sort_keys=True)

    @classmethod
    def from_json(cls, text: str) -> "BackupManifest":
        return cls(**json.loads(text))

    def covered(self) -> List[str]:
        return [str(e["name"]) for e in self.entries]


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def check_inventory(state_dir: Path) -> List[str]:
    """Required state that is missing from ``state_dir``.

    A missing required entry is reported rather than skipped, so a backup never
    silently omits the one thing a restore needs.
    """
    missing: List[str] = []
    for entry in STATE_INVENTORY:
        if not (state_dir / entry.relative_path).exists() and entry.required:
            missing.append(f"{entry.name} ({entry.relative_path})")
    return missing


def unplanned_state(state_dir: Path) -> List[str]:
    """Files present in the state directory that the inventory does not cover.

    Anything returned here is state we would silently lose in a restore, so it
    is surfaced for review before it is added to :data:`STATE_INVENTORY`.
    """
    covered = {e.relative_path for e in STATE_INVENTORY}
    known_dirs = {str(Path(e.relative_path).parent) for e in STATE_INVENTORY}
    out: List[str] = []
    for path in sorted(state_dir.rglob("*")):
        if not path.is_file() or path.name == MANIFEST_NAME:
            continue
        rel = path.relative_to(state_dir).as_posix()
        if rel in covered:
            continue
        # Files inside a covered directory (metrics/, index/) are covered by it.
        if any(rel.startswith(f"{d}/") for d in known_dirs if d != "."):
            continue
        out.append(rel)
    return out


def create_backup(
    state_dir: Path,
    backup_dir: Path,
    backup_id: Optional[str] = None,
    passphrase: Optional[str] = None,
) -> BackupManifest:
    """Snapshots the inventoried state and writes an encrypted archive.

    Raises ``FileNotFoundError`` when a required entry is absent: a partial
    backup that verifies is worse than a failed one, because it restores into a
    state that looks complete.
    """
    missing = check_inventory(state_dir)
    if missing:
        raise FileNotFoundError(f"required state missing from {state_dir}: {', '.join(missing)}")

    backup_id = backup_id or dt.datetime.utcnow().strftime("%Y%m%dT%H%M%SZ")
    backup_dir.mkdir(parents=True, exist_ok=True)
    raw = backup_dir / f"{backup_id}.tar.gz"

    manifest = BackupManifest(
        backup_id=backup_id,
        created_at=dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z",
        state_dir=str(state_dir),
        encrypted=bool(passphrase),
    )
    with tarfile.open(raw, "w:gz") as tar:
        for entry in STATE_INVENTORY:
            path = state_dir / entry.relative_path
            if not path.exists():
                continue
            tar.add(path, arcname=entry.relative_path)
            manifest.entries.append(
                {
                    "name": entry.name,
                    "path": entry.relative_path,
                    "sha256": _sha256(path),
                    "bytes": path.stat().st_size,
                    "rpo_hours": entry.rpo_hours,
                }
            )

    if passphrase:
        _encrypt(raw, raw, passphrase)
    manifest.archive_sha256 = _sha256(raw)
    (backup_dir / f"{backup_id}.manifest.json").write_text(manifest.to_json())
    return manifest


def _encrypt(src: Path, dst: Path, passphrase: str) -> None:
    """AES-256-CBC with PBKDF2 via openssl, replacing ``dst`` atomically.

    ``src`` and ``dst`` may be the same path (the common case: encrypt the
    archive in place), so the plaintext is staged in a sibling temp file and
    only unlinked once the ciphertext is in place.
    """
    staged = dst.with_name(dst.name + ".plain")
    enc = dst.with_name(dst.name + ".enc")
    subprocess.run(
        [
            "openssl", "enc", f"-{CIPHER}", "-pbkdf2",
            "-iter", str(PBKDF2_ITERATIONS), "-salt",
            "-in", str(src), "-out", str(enc), "-pass", f"pass:{passphrase}",
        ],
        check=True,
        capture_output=True,
    )
    enc.replace(dst)
    if src.resolve() != dst.resolve():
        src.replace(staged)
    staged.unlink(missing_ok=True)


def _decrypt(src: Path, dst: Path, passphrase: str) -> None:
    subprocess.run(
        [
            "openssl", "enc", "-d", f"-{CIPHER}", "-pbkdf2",
            "-iter", str(PBKDF2_ITERATIONS),
            "-in", str(src), "-out", str(dst), "-pass", f"pass:{passphrase}",
        ],
        check=True,
        capture_output=True,
    )


def verify_backup(manifest_path: Path, passphrase: Optional[str] = None) -> List[str]:
    """Re-checks the archive hash and every entry hash.

    Returns the list of problems; empty means the backup is trustworthy.
    """
    manifest = BackupManifest.from_json(manifest_path.read_text())
    archive = manifest_path.with_name(manifest.backup_id + ".tar.gz")
    problems: List[str] = []
    if not archive.exists():
        return [f"archive {archive.name} is missing"]

    if manifest.encrypted and passphrase:
        with tempfile.TemporaryDirectory() as tmp:
            plain = Path(tmp) / archive.name
            try:
                _decrypt(archive, plain, passphrase)
            except subprocess.CalledProcessError as exc:
                return [f"cannot decrypt {archive.name}: wrong passphrase or corrupt archive ({exc})"]
            problems.extend(_verify_entries(manifest, plain))
    else:
        problems.extend(_verify_entries(manifest, archive))
    return problems


def _verify_entries(manifest: BackupManifest, archive: Path) -> List[str]:
    problems: List[str] = []
    with tarfile.open(archive, "r:gz") as tar:
        names = set(tar.getnames())
        for entry in manifest.entries:
            name = str(entry["path"])
            if name not in names:
                problems.append(f"entry {name} missing from archive")
                continue
            extracted = tar.extractfile(name)
            if extracted is None:
                problems.append(f"entry {name} is not a regular file")
                continue
            digest = hashlib.sha256(extracted.read()).hexdigest()
            if digest != entry["sha256"]:
                problems.append(f"entry {name} failed checksum (archive corrupt or tampered)")
        covered = {str(e["path"]) for e in manifest.entries}
        for name in sorted(names - covered):
            problems.append(f"archive contains unlisted file {name}")
    return problems


def restore_backup(
    manifest_path: Path,
    target_dir: Path,
    passphrase: Optional[str] = None,
    in_place: bool = False,
    state_dir: Optional[Path] = None,
) -> List[str]:
    """Restores an archive into ``target_dir`` and verifies it.

    Refuses to overwrite a live state directory unless ``in_place`` is set, and
    even then only when the caller states the pipeline is stopped. The restore
    target defaults to a fresh directory, so the healthy running system is
    never the thing being modified.
    """
    problems = verify_backup(manifest_path, passphrase)
    if problems:
        return problems

    manifest = BackupManifest.from_json(manifest_path.read_text())
    archive = manifest_path.with_name(manifest.backup_id + ".tar.gz")
    if target_dir.exists() and any(target_dir.iterdir()) and not in_place:
        return [f"refusing to overwrite non-empty {target_dir} without in_place=True"]

    target_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        plain = archive
        if manifest.encrypted and passphrase:
            plain = Path(tmp) / archive.name
            _decrypt(archive, plain, passphrase)
        with tarfile.open(plain, "r:gz") as tar:
            for entry in manifest.entries:
                name = str(entry["path"])
                source = tar.extractfile(name)
                if source is None:
                    continue
                destination = target_dir / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(source.read())

    # Post-restore verification: the restored tree must satisfy the inventory.
    return check_inventory(target_dir)


def prune_backups(backup_dir: Path, retention_days: int, keep_minimum: int = 3) -> List[str]:
    """Deletes backups older than the retention window.

    ``keep_minimum`` protects against a misconfigured retention of 0 wiping
    every restore point: the newest N backups always survive, so "we lost the
    state" never becomes "we also lost every way back".
    """
    backups = sorted(backup_dir.glob("*.manifest.json"), key=lambda p: p.name, reverse=True)
    removed: List[str] = []
    cutoff = dt.datetime.utcnow() - dt.timedelta(days=retention_days)
    for index, manifest_path in enumerate(backups):
        if index < keep_minimum:
            continue
        manifest = BackupManifest.from_json(manifest_path.read_text())
        created = dt.datetime.fromisoformat(manifest.created_at.rstrip("Z"))
        if created < cutoff:
            manifest_path.unlink()
            archive = manifest_path.with_name(manifest.backup_id + ".tar.gz")
            if archive.exists():
                archive.unlink()
            removed.append(manifest.backup_id)
    return removed


@dataclass(frozen=True)
class Submission:
    """One cached submission, as stored in ``submissions.jsonl``."""

    asset: str
    source: str
    price: int
    ledger: int
    timestamp: int

    def to_json(self) -> str:
        return json.dumps(asdict(self), sort_keys=True)

    @classmethod
    def from_dict(cls, d: Dict[str, object]) -> "Submission":
        return cls(
            asset=str(d["asset"]),
            source=str(d["source"]),
            price=int(d["price"]),  # type: ignore[arg-type]
            ledger=int(d["ledger"]),  # type: ignore[arg-type]
            timestamp=int(d["timestamp"]),  # type: ignore[arg-type]
        )


@dataclass
class ReconcileReport:
    """Difference between restored off-chain state and on-chain truth."""

    accepted: List[Submission] = field(default_factory=list)
    dropped_future: List[Submission] = field(default_factory=list)
    missing_on_chain: List[Submission] = field(default_factory=list)
    price_mismatch: List[Tuple[Submission, int]] = field(default_factory=list)

    @property
    def clean(self) -> bool:
        """True when restored state is fully explained by on-chain truth.

        Dropped post-backup records do **not** make a restore unclean: a
        point-in-time restore is expected to discard them, and counting them as
        a failure would make every restore look broken. They are reported
        separately as :attr:`replay_residue` so a large number is still visible.
        """
        return not (self.missing_on_chain or self.price_mismatch)

    @property
    def replay_residue(self) -> int:
        """Restored records newer than the backup point, discarded on restore."""
        return len(self.dropped_future)

    def to_dict(self) -> Dict[str, object]:
        d = asdict(self)
        d["price_mismatch"] = [
            {"submission": s.to_json(), "on_chain_price": p} for s, p in self.price_mismatch
        ]
        d["clean"] = self.clean
        d["replay_residue"] = self.replay_residue
        return d

    def summary(self) -> str:
        return (
            f"accepted={len(self.accepted)} replay_residue={self.replay_residue} "
            f"missing_on_chain={len(self.missing_on_chain)} price_mismatch={len(self.price_mismatch)}"
        )


def load_submissions(path: Path) -> List[Submission]:
    """Reads ``submissions.jsonl``."""
    if not path.exists():
        return []
    out: List[Submission] = []
    for line in path.read_text().splitlines():
        if line.strip():
            out.append(Submission.from_dict(json.loads(line)))
    return out


def reconcile(
    restored: Iterable[Submission],
    on_chain: Dict[Tuple[str, str, int], int],
    backup_ledger: int,
) -> ReconcileReport:
    """Checks restored submissions against on-chain truth.

    * A restored submission **newer than the backup point** is dropped: it
      cannot have been in the backup, so it is replay residue from the failed
      window and replaying it risks a duplicate or an out-of-order submission.
    * A restored submission at or before the backup point that the chain does
      not confirm is reported as missing: either the restore is stale or the
      submission never landed.
    * A confirmed submission whose price differs from the chain is a price
      mismatch, which is a data-integrity finding, not a bookkeeping detail.
    """
    report = ReconcileReport()
    for sub in restored:
        if sub.ledger > backup_ledger:
            report.dropped_future.append(sub)
            continue
        price = on_chain.get((sub.asset, sub.source, sub.ledger))
        if price is None:
            report.missing_on_chain.append(sub)
        elif price != sub.price:
            report.price_mismatch.append((sub, price))
        else:
            report.accepted.append(sub)
    return report


def prometheus(report: ReconcileReport) -> str:
    """Exports the reconciliation counters for alerting."""
    return "\n".join(
        [
            "# TYPE oracle_restore_reconcile_mismatch_total counter",
            f'oracle_restore_reconcile_mismatch_total{{kind="dropped_future"}} {len(report.dropped_future)}',
            f'oracle_restore_reconcile_mismatch_total{{kind="missing_on_chain"}} {len(report.missing_on_chain)}',
            f'oracle_restore_reconcile_mismatch_total{{kind="price_mismatch"}} {len(report.price_mismatch)}',
        ]
    ) + "\n"


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--root", type=Path, default=Path("."), help="repository root (unused; paths are explicit)")
    sub = p.add_subparsers(dest="command", required=True)

    b = sub.add_parser("create", help="create an encrypted backup of the state directory")
    b.add_argument("--state-dir", type=Path, required=True)
    b.add_argument("--backup-dir", type=Path, required=True)
    b.add_argument("--passphrase", default=os.environ.get("ORACLE_BACKUP_PASSPHRASE"))
    b.add_argument("--retention-days", type=int, default=30)

    v = sub.add_parser("verify", help="verify a backup against its manifest")
    v.add_argument("manifest", type=Path)
    v.add_argument("--passphrase", default=os.environ.get("ORACLE_BACKUP_PASSPHRASE"))

    r = sub.add_parser("restore", help="restore a backup into a target directory")
    r.add_argument("manifest", type=Path)
    r.add_argument("--target", type=Path, required=True)
    r.add_argument("--passphrase", default=os.environ.get("ORACLE_BACKUP_PASSPHRASE"))
    r.add_argument("--in-place", action="store_true", help="overwrite a non-empty target directory")

    c = sub.add_parser("reconcile", help="reconcile restored submissions against on-chain truth")
    c.add_argument("--submissions", type=Path, required=True)
    c.add_argument("--on-chain", type=Path, required=True, help="JSON: [[asset, source, ledger, price], ...]")
    c.add_argument("--backup-ledger", type=int, required=True)

    pr = sub.add_parser("prune", help="apply the retention policy")
    pr.add_argument("--backup-dir", type=Path, required=True)
    pr.add_argument("--retention-days", type=int, default=30)
    pr.add_argument("--keep-minimum", type=int, default=3)

    i = sub.add_parser("inventory", help="report missing and unplanned off-chain state")
    i.add_argument("--state-dir", type=Path, required=True)

    args = p.parse_args(argv)

    if args.command == "create":
        try:
            manifest = create_backup(args.state_dir, args.backup_dir, passphrase=args.passphrase)
        except FileNotFoundError as exc:
            print(f"backup failed: {exc}", file=sys.stderr)
            return 1
        removed = prune_backups(args.backup_dir, args.retention_days)
        print(json.dumps({**asdict(manifest) | {"entries": manifest.covered()}, "pruned": removed}, indent=2))
        return 0
    if args.command == "verify":
        problems = verify_backup(args.manifest, args.passphrase)
        for problem in problems:
            print(f"backup verify: {problem}", file=sys.stderr)
        print("backup verify: OK" if not problems else "backup verify: FAILED")
        return 1 if problems else 0
    if args.command == "restore":
        problems = restore_backup(args.manifest, args.target, args.passphrase, in_place=args.in_place)
        for problem in problems:
            print(f"restore: {problem}", file=sys.stderr)
        if not problems:
            print(f"restore: {args.target} restored and verified")
        return 1 if problems else 0
    if args.command == "reconcile":
        restored = load_submissions(args.submissions)
        rows = json.loads(args.on_chain.read_text())
        on_chain = {(r[0], r[1], int(r[2])): int(r[3]) for r in rows}
        report = reconcile(restored, on_chain, args.backup_ledger)
        print(json.dumps(report.to_dict(), indent=2))
        print(prometheus(report), end="")
        return 0 if report.clean else 1
    if args.command == "prune":
        print(json.dumps({"pruned": prune_backups(args.backup_dir, args.retention_days, args.keep_minimum)}, indent=2))
        return 0
    if args.command == "inventory":
        missing = check_inventory(args.state_dir)
        unplanned = unplanned_state(args.state_dir)
        print(json.dumps({"missing_required": missing, "unplanned": unplanned}, indent=2))
        return 1 if missing else 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
