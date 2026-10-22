from services.operator_dashboard.dashboard import DashboardServer, build, render_text


def ev(ts, topic, **data):
    return {"ledger": ts // 5, "timestamp": ts, "contract_id": "C1", "topic": topic, "data": data}


EVENTS = [
    ev(1000, "timelock_queued", op_id=1, eta=5000, kind="upgrade"),
    ev(1005, "timelock_queued", op_id=2, eta=90000),
    ev(1010, "timelock_executed", op_id=2),
    ev(1020, "multisig_proposed", proposal_id=7, threshold=2, expires=4000),
    ev(1030, "multisig_approved", proposal_id=7),
    ev(1040, "health_changed", status="degraded", degraded_assets=["XLM"]),
]


def test_reflects_on_chain_state():
    s = build(EVENTS, now=1100)
    assert list(s.timelock_pending) == ["1"]
    assert s.multisig_pending["7"]["approvals"] == 1
    assert s.health == "degraded" and not s.stale and s.lag_s == 60


def test_stale_is_marked():
    s = build(EVENTS, now=1040 + 121)
    assert s.stale and "STALE" in render_text(s)
    assert build([], now=10).stale


def test_deadline_alerts_hit_hooks():
    got = []
    build(EVENTS, now=1100, alert_window_s=3000, hooks=[got.append])
    kinds = {a["kind"] for a in got}
    assert {"multisig", "health"} <= kinds and "timelock" not in kinds


def test_read_only():
    assert DashboardServer.do_POST is DashboardServer._reject
    assert not hasattr(DashboardServer, "sign")
