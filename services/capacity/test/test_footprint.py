"""Tests for the per-asset storage footprint forecast (#698)."""

from __future__ import annotations

import json
import re
from pathlib import Path

import pytest

from services.capacity.footprint import (
    LEDGERS_PER_DAY,
    ContractAssumptions,
    forecast,
    main,
    measure,
    observe,
    reconcile,
    render,
)
from services.capacity.model import TOLERANCE
from services.indexer.indexer import Indexer

ROOT = Path(__file__).resolve().parents[3]
SRC = ROOT / "contracts" / "price-oracle" / "src"

C = "CORACLE"
SOURCES = ("S1", "S2", "S3")
#: asset -> (sources submitting, aggregate every N ledgers)
ASSETS = {"XLM": (3, 1), "BTC": (3, 2), "ETH": (3, 3), "EURC": (2, 5)}


def workload(first, last):
    """A sample oracle workload: per-ledger submissions, per-asset cadence."""
    rows = []
    for ledger in range(first, last + 1):
        ts = 1_700_000_000 + ledger * 5
        if ledger == 1:
            rows += [{"ledger": 1, "timestamp": ts, "contract_id": C, "topic": "source_added",
                      "data": {"source": s, "name": s}} for s in SOURCES]
        for asset, (n_sources, every) in ASSETS.items():
            for i, source in enumerate(SOURCES[:n_sources]):
                rows.append({"ledger": ledger, "timestamp": ts, "contract_id": C,
                             "topic": "price_submitted",
                             "data": {"asset": asset, "source": source, "price": 1_000 + i}})
            if ledger % every == 0:
                rows.append({"ledger": ledger, "timestamp": ts, "contract_id": C,
                             "topic": "price_aggregated",
                             "data": {"asset": asset, "price": 1_001, "num_sources": n_sources}})
    return rows


def store(first=1, last=150):
    idx = Indexer()
    idx.ingest(workload(first, last))
    return idx


# ── Measurement ──────────────────────────────────────────────────────────────

def test_measurement_counts_each_supported_asset():
    m = measure(store())
    assert m.span_ledgers == 150
    by_asset = {a.asset: a for a in m.assets}
    assert set(by_asset) == set(ASSETS)
    assert by_asset["XLM"].sources == 3 and by_asset["EURC"].sources == 2
    assert by_asset["XLM"].aggregation_ledgers == 150
    assert by_asset["BTC"].aggregation_ledgers == 75
    assert m.other_events == len(SOURCES)
    assert m.bytes_per_event > 0


def test_empty_store_cannot_be_forecast():
    with pytest.raises(ValueError):
        measure(Indexer())


# ── Reconciliation: forecast from history vs. the observed store ─────────────

def test_sample_workload_reconciles_to_the_observed_footprint():
    assumptions = ContractAssumptions(max_history_length=1_000)
    idx = store(1, 150)
    fc = forecast(measure(idx), horizon_ledgers=150, assumptions=assumptions)

    idx.ingest(workload(151, 300))
    result = reconcile(fc, observe(idx, assumptions))

    assert result["within_tolerance"], result
    assert result["tolerance"] == TOLERANCE
    # Counts are linear in the measured rates, so they reconcile exactly; only
    # the store's page allocation carries a rounding error.
    assert result["metrics"]["contract_entries"] == 0
    assert result["metrics"]["indexer_events"] == 0


def test_reconciliation_flags_a_workload_that_changed_shape():
    fc = forecast(measure(store(1, 150)), horizon_ledgers=150)
    # The second half carries three times the traffic the history predicted.
    heavy = Indexer()
    heavy.ingest(workload(1, 150))
    for ledger in range(151, 301):
        rows = workload(ledger, ledger) * 3
        heavy.ingest([dict(r, event_index=i) for i, r in enumerate(rows)])
    assert not reconcile(fc, observe(heavy))["within_tolerance"]


# ── Boundaries: what grows and what saturates ────────────────────────────────

def test_contract_footprint_saturates_at_the_history_cap_while_the_indexer_grows():
    m = measure(store())
    month = forecast(m, 30 * LEDGERS_PER_DAY)
    quarter = forecast(m, 90 * LEDGERS_PER_DAY)

    assert all(r.history_entries == ContractAssumptions().history_cap for r in quarter.assets)
    assert quarter.contract_entries == month.contract_entries
    assert quarter.contract_bytes == month.contract_bytes
    assert quarter.indexer_bytes / month.indexer_bytes == pytest.approx(3.0, rel=0.01)


def test_temporary_tier_retention_bounds_history_but_persistent_tier_does_not():
    m = measure(store())
    uncapped = dict(max_history_length=10_000_000, max_history_per_asset=10_000_000)
    horizon = 30 * LEDGERS_PER_DAY

    temporary = forecast(m, horizon, ContractAssumptions(**uncapped))
    persistent = forecast(m, horizon, ContractAssumptions(retention_ledgers=None, **uncapped))

    xlm_temp = next(r for r in temporary.assets if r.asset == "XLM")
    xlm_pers = next(r for r in persistent.assets if r.asset == "XLM")
    assert xlm_temp.history_entries == 10_000
    assert xlm_pers.history_entries == 150 + horizon


def test_observed_history_respects_the_retention_window():
    idx = store(1, 300)
    obs = observe(idx, ContractAssumptions(retention_ledgers=50))
    # XLM aggregates every ledger: only the last 50 ledgers are retained.
    assert obs["contract_entries_per_asset"]["XLM"] == 3 + 2 * 3 + 50


def test_zero_horizon_forecast_is_the_measured_store():
    idx = store()
    m = measure(idx)
    fc = forecast(m, 0)
    assert fc.indexer_events == m.events
    assert fc.indexer_bytes == pytest.approx(m.store_bytes, abs=1)
    with pytest.raises(ValueError):
        forecast(m, -1)


# ── Assumptions are explicit and match the contract ──────────────────────────

def _const(path, name):
    match = re.search(rf"{name}: u32 = ([0-9_]+);", (SRC / path).read_text(encoding="utf-8"))
    assert match, name
    return int(match.group(1).replace("_", ""))


def test_default_assumptions_match_the_contract_constants():
    a = ContractAssumptions()
    assert a.max_history_length == _const("admin.rs", "DEFAULT_MAX_HISTORY")
    assert a.max_history_per_asset == _const("admin.rs", "DEFAULT_MAX_HISTORY_PER_ASSET")
    assert a.retention_ledgers == _const("types.rs", "TEMPORARY_RETENTION_LEDGERS")


# ── Report and CLI ───────────────────────────────────────────────────────────

def test_report_lists_every_asset_and_a_total():
    text = render(forecast(measure(store()), 90 * LEDGERS_PER_DAY))
    for asset in ASSETS:
        assert f"| {asset} |" in text
    assert "**total**" in text


def test_cli_forecasts_from_a_store_on_disk(tmp_path, capsys):
    db = tmp_path / "oracle-index.db"
    Indexer(str(db)).ingest(workload(1, 50))
    assert main([str(db), "--horizon-days", "7", "--json"]) == 0
    out = json.loads(capsys.readouterr().out)
    assert out["horizon_days"] == 7.0
    assert {a["asset"] for a in out["assets"]} == set(ASSETS)
