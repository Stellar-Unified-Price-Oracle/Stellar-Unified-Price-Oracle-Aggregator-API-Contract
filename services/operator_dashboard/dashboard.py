"""Read-only operator dashboard (#533).

Folds the indexed on-chain event stream (services/common/events.py
envelope) into one view: pending timelock operations, multisig approvals
awaiting signatures, upcoming TTL/key expirations and health state. It
never holds a key and exposes no mutating endpoint — ``DashboardServer``
answers GET only and rejects every other verb with 405.

Freshness: every snapshot carries ``lag_s = now - last_event_timestamp``;
when it exceeds ``max_lag_s`` the snapshot is marked ``stale`` and the UI
renders the banner. Items whose deadline is within ``alert_window_s``
become alerts, forwarded to any registered hook (webhook, Pager, stdout).

    python -m services.operator_dashboard.dashboard --events events.jsonl --port 8088
"""
from __future__ import annotations

import argparse
import json
import time
from dataclasses import asdict, dataclass, field
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Callable, Dict, Iterable, List, Optional

from services.common.events import EventEnvelope, EventSource, iter_envelopes

AlertHook = Callable[[dict], None]


@dataclass
class Snapshot:
    timelock_pending: Dict[str, dict] = field(default_factory=dict)
    multisig_pending: Dict[str, dict] = field(default_factory=dict)
    expirations: Dict[str, int] = field(default_factory=dict)
    health: str = "unknown"
    degraded_assets: List[str] = field(default_factory=list)
    last_event_ts: int = 0
    last_ledger: int = 0
    lag_s: int = 0
    stale: bool = True
    alerts: List[dict] = field(default_factory=list)


def fold(events: Iterable[EventEnvelope]) -> Snapshot:
    s = Snapshot()
    for e in events:
        d = e.data
        t = e.topic
        if t in ("timelock_queued", "op_proposed"):
            s.timelock_pending[str(d["op_id"])] = {"eta": int(d.get("eta", 0)), "kind": d.get("kind")}
        elif t in ("timelock_executed", "timelock_cancelled", "op_executed", "op_cancelled"):
            s.timelock_pending.pop(str(d["op_id"]), None)
        elif t == "multisig_proposed":
            s.multisig_pending[str(d["proposal_id"])] = {
                "approvals": 0, "threshold": int(d["threshold"]), "expires": int(d.get("expires", 0))}
        elif t == "multisig_approved" and str(d["proposal_id"]) in s.multisig_pending:
            s.multisig_pending[str(d["proposal_id"])]["approvals"] += 1
        elif t in ("multisig_executed", "multisig_expired"):
            s.multisig_pending.pop(str(d["proposal_id"]), None)
        elif t in ("ttl_extended", "key_registered"):
            s.expirations[str(d["key"])] = int(d["expires"])
        elif t == "health_changed":
            s.health = str(d["status"])
            s.degraded_assets = list(d.get("degraded_assets", []))
        s.last_event_ts = max(s.last_event_ts, e.timestamp)
        s.last_ledger = max(s.last_ledger, e.ledger)
    return s


def deadlines(s: Snapshot) -> List[tuple]:
    out = [("timelock", k, v["eta"]) for k, v in s.timelock_pending.items()]
    out += [("multisig", k, v["expires"]) for k, v in s.multisig_pending.items() if v["expires"]]
    out += [("expiry", k, v) for k, v in s.expirations.items()]
    return out


def build(source: EventSource, now: Optional[int] = None, max_lag_s: int = 120,
          alert_window_s: int = 3600, hooks: Iterable[AlertHook] = ()) -> Snapshot:
    now = int(time.time()) if now is None else now
    s = fold(iter_envelopes(source))
    s.lag_s = max(0, now - s.last_event_ts) if s.last_event_ts else now
    s.stale = s.last_event_ts == 0 or s.lag_s > max_lag_s
    for kind, key, due in deadlines(s):
        if due - now <= alert_window_s:
            s.alerts.append({"kind": kind, "id": key, "due": due, "in_s": due - now})
    if s.health not in ("healthy", "unknown"):
        s.alerts.append({"kind": "health", "id": s.health, "due": now, "in_s": 0})
    for alert in s.alerts:
        for hook in hooks:
            hook(alert)
    return s


def render_text(s: Snapshot) -> str:
    banner = f"STALE — last event {s.lag_s}s ago" if s.stale else f"fresh ({s.lag_s}s lag)"
    lines = [f"[{banner}] ledger {s.last_ledger} health={s.health}"]
    lines += [f"timelock {k} eta={v['eta']}" for k, v in s.timelock_pending.items()]
    lines += [f"multisig {k} {v['approvals']}/{v['threshold']}" for k, v in s.multisig_pending.items()]
    lines += [f"ALERT {a['kind']} {a['id']} in {a['in_s']}s" for a in s.alerts]
    return "\n".join(lines)


class DashboardServer(BaseHTTPRequestHandler):
    source: EventSource = ()

    def do_GET(self):  # noqa: N802
        body = json.dumps(asdict(build(self.source))).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(body)

    def _reject(self):
        self.send_response(405)
        self.end_headers()

    do_POST = do_PUT = do_PATCH = do_DELETE = _reject


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--events", required=True)
    ap.add_argument("--port", type=int)
    args = ap.parse_args(argv)
    if args.port is None:
        print(render_text(build(args.events, hooks=[lambda a: print(json.dumps(a))])))
        return 0
    DashboardServer.source = args.events
    HTTPServer(("127.0.0.1", args.port), DashboardServer).serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
