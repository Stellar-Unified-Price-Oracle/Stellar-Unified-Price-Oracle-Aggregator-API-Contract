"""Tests for off-chain backup, point-in-time restore and reconciliation (#529).

`test_restore_drill_reproduces_known_good_state` is the automated form of the
drill recorded in
``docs/incident-management/2026-08-12-restore-drill.md``: destroy the state,
restore from backup, and reconcile against on-chain truth.
"""

from __future__ import annotations

import json
import shutil
from pathlib import Path
from typing import Sequence, Tuple

import pytest

from services.backup.backup import (
    STATE_INVENTORY,
    BackupManifest,
    Submission,
    check_inventory,
    create_backup,
    load_submissions,
    prune_backups,
    prometheus,
    reconcile,
    restore_backup,
    unplanned_state,
    verify_backup,
)

PASSPHRASE = "correct-horse-battery-staple"
HAS_OPENSSL = shutil.which("openssl") is not None
needs_openssl = pytest.mark.skipif(not HAS_OPENSSL, reason="openssl is required for encryption")

SOURCES = {"sources": ["A", "B", "C"], "min_sources_required": 2}


def _state(
    root: Path,
    submissions: Sequence[Tuple[str, str, int]] = (("XLM/USD", "A", 100), ("XLM/USD", "B", 110)),
) -> Path:
    """Writes a complete off-chain state directory."""
    state = root / "state"
    (state / "index").mkdir(parents=True, exist_ok=True)
    (state / "sources.json").write_text(json.dumps(SOURCES))
    (state / "index" / "assets.json").write_text(json.dumps({"XLM/USD": {"decimals": 7}}))
    (state / "nonces.json").write_text(json.dumps({"A": 41, "B": 42}))
    (state / "metrics.snap").write_text("oracle_price_submissions_total 12\n")
    lines = [
        json.dumps({"asset": a, "source": s, "price": p, "ledger": 1000 + i, "timestamp": 500 + i})
        for i, (a, s, p) in enumerate(submissions)
    ]
    (state / "submissions.jsonl").write_text("\n".join(lines) + "\n")
    return state


def _manifest(backup_dir: Path) -> Path:
    return sorted(backup_dir.glob("*.manifest.json"))[0]


# ── Inventory ────────────────────────────────────────────────────────────────

def test_required_state_is_detected_as_present(tmp_path):
    assert check_inventory(_state(tmp_path)) == []


def test_missing_required_state_is_reported_not_skipped(tmp_path):
    state = _state(tmp_path)
    (state / "nonces.json").unlink()
    assert check_inventory(state) == ["nonces (nonces.json)"]


def test_backup_refuses_to_run_with_missing_required_state(tmp_path):
    state = _state(tmp_path)
    (state / "submissions.jsonl").unlink()
    with pytest.raises(FileNotFoundError, match="submissions"):
        create_backup(state, tmp_path / "backups")


def test_unplanned_state_is_surfaced(tmp_path):
    state = _state(tmp_path)
    (state / "secrets.yaml").write_text("token: abc\n")
    assert "secrets.yaml" in unplanned_state(state)
    assert "sources.json" not in unplanned_state(state)


def test_inventory_covers_every_planned_component():
    assert {e.name for e in STATE_INVENTORY} >= {
        "sources", "submissions", "index", "nonces",
    }
    assert all(e.description and e.rpo_hours > 0 for e in STATE_INVENTORY)


# ── Create / verify ──────────────────────────────────────────────────────────

def test_backup_writes_manifest_covering_all_state(tmp_path):
    state = _state(tmp_path)
    manifest = create_backup(state, tmp_path / "backups", backup_id="b1")
    assert set(manifest.covered()) == {e.name for e in STATE_INVENTORY}
    assert verify_backup(_manifest(tmp_path / "backups")) == []


@needs_openssl
def test_backup_is_encrypted_at_rest(tmp_path):
    state = _state(tmp_path)
    create_backup(state, tmp_path / "backups", backup_id="b1", passphrase=PASSPHRASE)
    archive = tmp_path / "backups" / "b1.tar.gz"
    blob = archive.read_bytes()
    assert b"XLM/USD" not in blob, "submission content must not be readable at rest"
    assert verify_backup(_manifest(tmp_path / "backups"), PASSPHRASE) == []


@needs_openssl
def test_wrong_passphrase_fails_verification_rather_than_silently_restoring(tmp_path):
    create_backup(_state(tmp_path), tmp_path / "backups", backup_id="b1", passphrase=PASSPHRASE)
    problems = verify_backup(_manifest(tmp_path / "backups"), "not-the-passphrase")
    assert problems and "decrypt" in problems[0]


def test_verify_detects_a_tampered_entry(tmp_path):
    create_backup(_state(tmp_path), tmp_path / "backups", backup_id="b1")
    manifest = _manifest(tmp_path / "backups")
    data = json.loads(manifest.read_text())
    data["entries"][0]["sha256"] = "0" * 64
    manifest.write_text(json.dumps(data))
    assert any("failed checksum" in p for p in verify_backup(manifest))


def test_verify_detects_a_missing_archive(tmp_path):
    create_backup(_state(tmp_path), tmp_path / "backups", backup_id="b1")
    manifest = _manifest(tmp_path / "backups")
    (tmp_path / "backups" / "b1.tar.gz").unlink()
    assert verify_backup(manifest) == ["archive b1.tar.gz is missing"]


# ── Restore ──────────────────────────────────────────────────────────────────

def test_restore_round_trips_every_inventoried_file(tmp_path):
    state = _state(tmp_path)
    create_backup(state, tmp_path / "backups", backup_id="b1")
    target = tmp_path / "restored"
    assert restore_backup(_manifest(tmp_path / "backups"), target) == []
    for entry in STATE_INVENTORY:
        assert (target / entry.relative_path).read_bytes() == (state / entry.relative_path).read_bytes()


@needs_openssl
def test_encrypted_backup_restores_identically(tmp_path):
    state = _state(tmp_path)
    create_backup(state, tmp_path / "backups", backup_id="b1", passphrase=PASSPHRASE)
    target = tmp_path / "restored"
    assert restore_backup(_manifest(tmp_path / "backups"), target, PASSPHRASE) == []
    assert (target / "sources.json").read_text() == json.dumps(SOURCES)


def test_restore_refuses_to_overwrite_a_running_state_directory(tmp_path):
    state = _state(tmp_path)
    create_backup(state, tmp_path / "backups", backup_id="b1")
    live = tmp_path / "live"
    live.mkdir()
    (live / "submissions.jsonl").write_text("{}\n")
    problems = restore_backup(_manifest(tmp_path / "backups"), live)
    assert any("refusing to overwrite" in p for p in problems)
    # The running state is untouched.
    assert (live / "submissions.jsonl").read_text() == "{}\n"


def test_restore_into_a_fresh_directory_leaves_the_live_state_alone(tmp_path):
    state = _state(tmp_path)
    create_backup(state, tmp_path / "backups", backup_id="b1")
    (state / "submissions.jsonl").write_text("CLOBBERED\n")
    assert restore_backup(_manifest(tmp_path / "backups"), tmp_path / "restored") == []
    assert (state / "submissions.jsonl").read_text() == "CLOBBERED\n"


def test_restore_does_not_run_when_verification_fails(tmp_path):
    state = _state(tmp_path)
    create_backup(state, tmp_path / "backups", backup_id="b1")
    manifest = _manifest(tmp_path / "backups")
    data = json.loads(manifest.read_text())
    data["entries"][0]["sha256"] = "0" * 64
    manifest.write_text(json.dumps(data))
    target = tmp_path / "restored"
    assert restore_backup(manifest, target)
    assert not target.exists(), "a failed verification must not leave a partial restore"


# ── Reconciliation against on-chain truth ────────────────────────────────────

def _chain(rows):
    return {(a, s, l): p for a, s, l, p in rows}


def test_reconcile_accepts_restored_state_that_matches_the_chain():
    restored = [
        Submission("XLM/USD", "A", 100, 1000, 500),
        Submission("XLM/USD", "B", 110, 1001, 501),
    ]
    report = reconcile(restored, _chain([("XLM/USD", "A", 1000, 100), ("XLM/USD", "B", 1001, 110)]), 1001)
    assert report.clean
    assert len(report.accepted) == 2


def test_reconcile_drops_submissions_newer_than_the_backup_point():
    restored = [
        Submission("XLM/USD", "A", 100, 1000, 500),
        Submission("XLM/USD", "A", 999, 1005, 505),  # landed after the backup
    ]
    report = reconcile(restored, _chain([("XLM/USD", "A", 1000, 100)]), 1001)
    assert [s.ledger for s in report.dropped_future] == [1005]
    assert len(report.accepted) == 1


def test_reconcile_flags_a_submission_the_chain_does_not_confirm():
    restored = [Submission("XLM/USD", "A", 100, 1000, 500)]
    report = reconcile(restored, {}, 1001)
    assert not report.clean
    assert [s.ledger for s in report.missing_on_chain] == [1000]


def test_reconcile_flags_a_price_mismatch():
    restored = [Submission("XLM/USD", "A", 100, 1000, 500)]
    report = reconcile(restored, _chain([("XLM/USD", "A", 1000, 101)]), 1001)
    assert not report.clean
    assert report.price_mismatch[0][1] == 101


def test_reconcile_exports_prometheus_counters():
    restored = [Submission("XLM/USD", "A", 100, 1000, 500)]
    text = prometheus(reconcile(restored, {}, 1001))
    assert 'oracle_restore_reconcile_mismatch_total{kind="missing_on_chain"} 1' in text


# ── Retention ────────────────────────────────────────────────────────────────

def test_prune_removes_backups_past_the_retention_window(tmp_path):
    backups = tmp_path / "backups"
    for i in range(5):
        create_backup(_state(tmp_path), backups, backup_id=f"b{i}")
    removed = prune_backups(backups, retention_days=30)
    assert removed == []
    # Age everything except the newest two past the window.
    for path in sorted(backups.glob("*.manifest.json"))[:-2]:
        m = BackupManifest.from_json(path.read_text())
        m.created_at = "2000-01-01T00:00:00Z"
        path.write_text(m.to_json())
    removed = prune_backups(backups, retention_days=30, keep_minimum=2)
    assert len(removed) == 3
    assert len(list(backups.glob("*.manifest.json"))) == 2


def test_prune_keeps_the_minimum_even_with_zero_retention(tmp_path):
    backups = tmp_path / "backups"
    for i in range(3):
        create_backup(_state(tmp_path), backups, backup_id=f"b{i}")
    for path in backups.glob("*.manifest.json"):
        m = BackupManifest.from_json(path.read_text())
        m.created_at = "2000-01-01T00:00:00Z"
        path.write_text(m.to_json())
    prune_backups(backups, retention_days=0, keep_minimum=3)
    assert len(list(backups.glob("*.manifest.json"))) == 3


# ── The drill ────────────────────────────────────────────────────────────────

@needs_openssl
def test_restore_drill_reproduces_known_good_state(tmp_path):
    """backup -> destroy -> restore -> reconcile, end to end."""
    state = _state(tmp_path)
    backups = tmp_path / "backups"
    create_backup(state, backups, backup_id="drill", passphrase=PASSPHRASE)
    known_good = {e.relative_path: (state / e.relative_path).read_bytes() for e in STATE_INVENTORY}

    # Destroy the off-chain state, as the drill did.
    shutil.rmtree(state)
    assert check_inventory(tmp_path / "state"), "the destroyed state must report as missing"

    manifest = backups / "drill.manifest.json"
    assert verify_backup(manifest, PASSPHRASE) == []
    restored = tmp_path / "restored"
    assert restore_backup(manifest, restored, PASSPHRASE) == []

    # Every byte of known-good state is back.
    for rel, blob in known_good.items():
        assert (restored / rel).read_bytes() == blob

    # Reconciling against on-chain truth: the confirmed submissions are accepted.
    submissions = load_submissions(restored / "submissions.jsonl")
    ledger = max(s.ledger for s in submissions)
    chain = {(s.asset, s.source, s.ledger): s.price for s in submissions}
    report = reconcile(submissions, chain, ledger)
    assert report.clean, report.summary()


@needs_openssl
def test_restore_drill_rejects_replay_residue_from_the_failed_window(tmp_path):
    """Post-backup records are dropped rather than replayed."""
    state = _state(tmp_path)
    backups = tmp_path / "backups"
    create_backup(state, backups, backup_id="drill", passphrase=PASSPHRASE)
    restored = tmp_path / "restored"
    restore_backup(backups / "drill.manifest.json", restored, PASSPHRASE)

    # The pipeline appended records during the outage window, after the backup.
    with (restored / "submissions.jsonl").open("a") as fh:
        fh.write(json.dumps({"asset": "XLM/USD", "source": "A", "price": 999,
                             "ledger": 9999, "timestamp": 999}) + "\n")

    submissions = load_submissions(restored / "submissions.jsonl")
    chain = {(s.asset, s.source, s.ledger): s.price for s in submissions if s.ledger <= 1001}
    report = reconcile(submissions, chain, backup_ledger=1001)
    assert [s.ledger for s in report.dropped_future] == [9999]
    assert report.clean


# ── The documentation stays in step with the inventory ───────────────────────

def test_backup_doc_documents_every_inventoried_component():
    doc = (Path(__file__).resolve().parents[3] / "docs" / "backup-restore.md").read_text()
    for entry in STATE_INVENTORY:
        assert entry.relative_path in doc, f"{entry.name} is not documented in backup-restore.md"


def test_backup_doc_states_the_recovery_objectives():
    doc = (Path(__file__).resolve().parents[3] / "docs" / "backup-restore.md").read_text()
    for token in ("RPO", "RTO", "retention", "encrypt", "Reconcile", "drill"):
        assert token.lower() in doc.lower(), f"backup-restore.md does not cover {token}"
