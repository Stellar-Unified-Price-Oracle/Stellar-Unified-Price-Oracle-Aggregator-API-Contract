"""Tests for asset metadata provenance and change history (#705)."""

from pathlib import Path
import pytest

from services.provenance.asset_metadata import (
    AssetMetadataConfig,
    AssetMetadataProvenanceTracker,
    AssetMetadataRevision,
    GENESIS_PREVIOUS_HASH,
    compute_revision_hash,
    main,
)


def test_asset_config_validation():
    # Valid config
    valid = AssetMetadataConfig(
        decimals=7,
        ticker="XLM",
        asset_type="native",
        sources=["S1", "S2"],
        source_weights_bps=[5000, 5000],
    )
    valid.validate()

    # Invalid decimals
    with pytest.raises(ValueError, match="Decimals must be between 0 and 18"):
        AssetMetadataConfig(
            decimals=19,
            ticker="XLM",
            asset_type="native",
            sources=["S1"],
            source_weights_bps=[10000],
        ).validate()

    # Empty sources
    with pytest.raises(ValueError, match="at least one price source"):
        AssetMetadataConfig(
            decimals=7,
            ticker="XLM",
            asset_type="native",
            sources=[],
            source_weights_bps=[],
        ).validate()

    # Weight sum mismatch
    with pytest.raises(ValueError, match="sum to 10,000 bps"):
        AssetMetadataConfig(
            decimals=7,
            ticker="XLM",
            asset_type="native",
            sources=["S1", "S2"],
            source_weights_bps=[5000, 4000],
        ).validate()


def test_genesis_revision_commits_correctly():
    tracker = AssetMetadataProvenanceTracker()
    cfg = AssetMetadataConfig(
        decimals=7,
        ticker="XLM",
        asset_type="native",
        sources=["SRC_A"],
        source_weights_bps=[10000],
    )
    rev = tracker.propose_and_commit_revision(
        asset_id="XLM",
        config=cfg,
        effective_ledger=100,
        effective_timestamp=1700000000,
        proposer="admin:operator",
        reviewer="committee:risk",
        evidence_url="https://stellar.org/gov/1",
        rationale="Genesis registration",
    )

    assert rev.revision_number == 1
    assert rev.previous_hash == GENESIS_PREVIOUS_HASH
    assert len(rev.hash) == 64
    assert rev.prior_config is None

    valid, errors = tracker.verify_provenance_chain("XLM")
    assert valid is True
    assert errors == []


def test_chain_linkage_and_monotonic_ledgers():
    tracker = AssetMetadataProvenanceTracker()
    cfg1 = AssetMetadataConfig(
        decimals=7,
        ticker="USDC",
        asset_type="credit_alphanum4",
        sources=["S1"],
        source_weights_bps=[10000],
    )
    rev1 = tracker.propose_and_commit_revision(
        asset_id="USDC",
        config=cfg1,
        effective_ledger=1000,
        effective_timestamp=1700000000,
        proposer="operator:alice",
        reviewer="approver:bob",
        evidence_url="https://stellar.org/gov/usdc-1",
        rationale="Initial USDC listing",
    )

    cfg2 = AssetMetadataConfig(
        decimals=7,
        ticker="USDC",
        asset_type="credit_alphanum4",
        sources=["S1", "S2"],
        source_weights_bps=[6000, 4000],
    )
    rev2 = tracker.propose_and_commit_revision(
        asset_id="USDC",
        config=cfg2,
        effective_ledger=2000,
        effective_timestamp=1700010000,
        proposer="operator:charlie",
        reviewer="approver:dave",
        evidence_url="https://stellar.org/gov/usdc-2",
        rationale="Add source S2",
    )

    assert rev2.revision_number == 2
    assert rev2.previous_hash == rev1.hash
    assert rev2.prior_config == cfg1

    valid, errors = tracker.verify_provenance_chain("USDC")
    assert valid is True
    assert errors == []

    # Non-monotonic ledger must be rejected
    with pytest.raises(ValueError, match="must be strictly greater than latest ledger"):
        tracker.propose_and_commit_revision(
            asset_id="USDC",
            config=cfg2,
            effective_ledger=1500,  # Less than latest 2000
            effective_timestamp=1700020000,
            proposer="op",
            reviewer="rev",
            evidence_url="url",
            rationale="invalid ledger",
        )


def test_tamper_detection_catches_modified_fields():
    tracker = AssetMetadataProvenanceTracker()
    cfg = AssetMetadataConfig(
        decimals=7,
        ticker="BTC",
        asset_type="credit_alphanum4",
        sources=["S1"],
        source_weights_bps=[10000],
    )
    rev = tracker.propose_and_commit_revision(
        asset_id="BTC",
        config=cfg,
        effective_ledger=500,
        effective_timestamp=1700000000,
        proposer="op",
        reviewer="rev",
        evidence_url="url",
        rationale="Genesis BTC",
    )

    # Tamper with the recorded config (e.g. change decimals from 7 to 8 without updating hash)
    tampered_cfg = AssetMetadataConfig(
        decimals=8,  # Modified
        ticker="BTC",
        asset_type="credit_alphanum4",
        sources=["S1"],
        source_weights_bps=[10000],
    )
    tampered_rev = AssetMetadataRevision(
        asset_id=rev.asset_id,
        revision_number=rev.revision_number,
        config=tampered_cfg,
        prior_config=rev.prior_config,
        effective_ledger=rev.effective_ledger,
        effective_timestamp=rev.effective_timestamp,
        proposer=rev.proposer,
        reviewer=rev.reviewer,
        evidence_url=rev.evidence_url,
        rationale=rev.rationale,
        previous_hash=rev.previous_hash,
        hash=rev.hash,  # Old hash retained
    )
    tracker._chains["BTC"][0] = tampered_rev

    valid, errors = tracker.verify_provenance_chain("BTC")
    assert valid is False
    assert any("hash commitment mismatch" in err for err in errors)


def test_tamper_detection_catches_broken_chain():
    tracker = AssetMetadataProvenanceTracker()
    cfg1 = AssetMetadataConfig(decimals=7, ticker="ETH", asset_type="credit_alphanum4", sources=["S1"], source_weights_bps=[10000])
    rev1 = tracker.propose_and_commit_revision("ETH", cfg1, 100, 1000, "op", "rev", "url1", "init")

    cfg2 = AssetMetadataConfig(decimals=7, ticker="ETH", asset_type="credit_alphanum4", sources=["S1"], source_weights_bps=[10000])
    rev2 = tracker.propose_and_commit_revision("ETH", cfg2, 200, 2000, "op", "rev", "url2", "update")

    # Corrupt previous_hash of rev2
    corrupted_rev2 = AssetMetadataRevision(
        asset_id=rev2.asset_id,
        revision_number=rev2.revision_number,
        config=rev2.config,
        prior_config=rev2.prior_config,
        effective_ledger=rev2.effective_ledger,
        effective_timestamp=rev2.effective_timestamp,
        proposer=rev2.proposer,
        reviewer=rev2.reviewer,
        evidence_url=rev2.evidence_url,
        rationale=rev2.rationale,
        previous_hash="f" * 64,  # Corrupted previous_hash
        hash=rev2.hash,
    )
    tracker._chains["ETH"][1] = corrupted_rev2

    valid, errors = tracker.verify_provenance_chain("ETH")
    assert valid is False
    assert any("previous_hash mismatch" in err for err in errors)


def test_point_in_time_historical_query():
    tracker = AssetMetadataProvenanceTracker()
    cfg1 = AssetMetadataConfig(decimals=6, ticker="EURC", asset_type="credit_alphanum4", sources=["S1"], source_weights_bps=[10000])
    tracker.propose_and_commit_revision("EURC", cfg1, 1000, 1700000000, "op", "rev", "url1", "v1")

    cfg2 = AssetMetadataConfig(decimals=7, ticker="EURC", asset_type="credit_alphanum4", sources=["S1"], source_weights_bps=[10000])
    tracker.propose_and_commit_revision("EURC", cfg2, 3000, 1700020000, "op", "rev", "url2", "v2")

    # Before revision 1
    assert tracker.get_metadata_at_ledger("EURC", 500) is None
    assert tracker.get_metadata_at_timestamp("EURC", 1699999999) is None

    # During revision 1
    m1 = tracker.get_metadata_at_ledger("EURC", 2000)
    assert m1 is not None
    assert m1.decimals == 6

    # After revision 2
    m2 = tracker.get_metadata_at_ledger("EURC", 4000)
    assert m2 is not None
    assert m2.decimals == 7


def test_diff_computation():
    cfg1 = AssetMetadataConfig(decimals=7, ticker="XLM", asset_type="native", sources=["S1"], source_weights_bps=[10000], staleness_threshold_seconds=300)
    cfg2 = AssetMetadataConfig(decimals=7, ticker="XLM", asset_type="native", sources=["S1", "S2"], source_weights_bps=[5000, 5000], staleness_threshold_seconds=180)

    rev = AssetMetadataRevision(
        asset_id="XLM",
        revision_number=2,
        config=cfg2,
        prior_config=cfg1,
        effective_ledger=200,
        effective_timestamp=1700001000,
        proposer="op",
        reviewer="rev",
        evidence_url="url",
        rationale="re-weight",
        previous_hash="prev",
        hash="curr",
    )

    diff = rev.compute_diff()
    assert "sources" in diff
    assert diff["sources"] == (["S1"], ["S1", "S2"])
    assert "source_weights_bps" in diff
    assert "staleness_threshold_seconds" in diff
    assert diff["staleness_threshold_seconds"] == (300, 180)
    assert "decimals" not in diff  # Decimals did not change


def test_markdown_audit_export_and_cli(tmp_path: Path):
    report_file = tmp_path / "provenance_audit.md"
    exit_code = main(["--json", "--report", str(report_file)])
    assert exit_code == 0
    assert report_file.exists()
    content = report_file.read_text(encoding="utf-8")
    assert "# Asset Metadata Provenance & Audit Trail: `XLM`" in content
    assert "Revision #1" in content
    assert "Revision #2" in content
    assert "Changes from Prior Revision" in content
