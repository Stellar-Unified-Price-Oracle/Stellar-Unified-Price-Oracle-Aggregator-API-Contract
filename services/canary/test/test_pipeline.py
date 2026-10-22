import json

from services.canary.pipeline import (
    BLUE,
    GREEN,
    compare,
    decide,
    main,
    route,
    routing_table,
)

ASSETS = [f"C{i:055d}" for i in range(500)]


def snapshot(prices):
    return {a: {"price": p, "decimals": 7, "num_sources": 5} for a, p in prices.items()}


def test_each_asset_served_by_exactly_one_version_every_ledger():
    table = routing_table(ASSETS, canary_bps=2_000, salt="epoch-1")
    for _ledger in range(100):
        # The rule has no ledger/caller input: re-evaluating always agrees.
        for asset in ASSETS:
            assert route(asset, 2_000, "epoch-1") == table[asset]
    assert set(table.values()) == {BLUE, GREEN}


def test_canary_fraction_is_respected():
    table = routing_table(ASSETS, canary_bps=2_000, salt="epoch-1")
    share = sum(v == GREEN for v in table.values()) / len(ASSETS)
    assert 0.12 < share < 0.28


def test_identical_versions_await_approval():
    blue = snapshot({a: 1_000_000 for a in ASSETS})
    assert decide(compare(blue, dict(blue))) == "await_approval"


def test_minority_asset_divergence_is_caught_and_rolled_back(tmp_path):
    blue = snapshot({a: 1_000_000 for a in ASSETS})
    green = snapshot({a: 1_000_000 for a in ASSETS})
    # Deliberately divergent green: off by one unit on a single asset that is
    # routed to blue, i.e. outside the canary slice.
    victim = next(a for a in ASSETS if route(a, 1_000, "s") == BLUE)
    green[victim]["price"] += 1
    (tmp_path / "b.json").write_text(json.dumps(blue))
    (tmp_path / "g.json").write_text(json.dumps(green))
    out = tmp_path / "record.json"
    rc = main([
        "--blue-snapshot", str(tmp_path / "b.json"),
        "--green-snapshot", str(tmp_path / "g.json"),
        "--blue-contract", "CBLUE", "--green-contract", "CGREEN",
        "--canary-bps", "1000", "--salt", "s", "--out", str(out),
    ])
    record = json.loads(out.read_text())
    assert rc == 1
    assert record["decision"] == "rollback"
    assert record["divergences"][0]["asset"] == victim
    assert len(record["evidence_digest"]) == 64


def test_missing_asset_and_metadata_changes_diverge():
    blue = snapshot({"A": 1, "B": 2})
    green = snapshot({"A": 1})
    green["A"]["num_sources"] = 4
    fields = {(d.asset, d.field) for d in compare(blue, green)}
    assert fields == {("A", "num_sources"), ("B", "presence")}
