"""Minimal explorer over the indexer store (#539). Stdlib only.

    python -m services.indexer.explorer --db oracle-index.db --port 8088

Endpoints: `/` (HTML), `/api/sources`, `/api/gaps`, `/api/history?asset=A`,
`/api/provenance?asset=A&ledger=L&event_index=I`.
"""
from __future__ import annotations

import argparse
import html
import json
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlparse

from services.indexer.indexer import Indexer


def route(idx: Indexer, path: str, query: dict) -> tuple[int, str, str]:
    q = {k: v[0] for k, v in query.items()}
    if path == "/api/sources":
        return 200, "application/json", json.dumps(idx.sources())
    if path == "/api/gaps":
        return 200, "application/json", json.dumps(idx.gaps())
    if path == "/api/history" and "asset" in q:
        return 200, "application/json", json.dumps(idx.history(q["asset"], int(q.get("limit", 100))))
    if path == "/api/provenance" and {"asset", "ledger", "event_index"} <= q.keys():
        body = idx.provenance(q["asset"], int(q["ledger"]), int(q["event_index"]))
        return 200, "application/json", json.dumps(body)
    if path == "/":
        return 200, "text/html", render_index(idx, q.get("asset"))
    return 404, "application/json", json.dumps({"error": "not found"})


def render_index(idx: Indexer, asset: str | None) -> str:
    e = html.escape
    parts = ["<!doctype html><title>Oracle explorer</title><h1>Oracle explorer</h1>",
             "<form><input name=asset placeholder='asset address' value='%s'><button>Show</button></form>"
             % e(asset or "")]
    parts.append("<h2>Sources</h2><ul>")
    for s in idx.sources():
        status = "removed @%s" % s["removed_ledger"] if s["removed_ledger"] else "active"
        parts.append(f"<li>{e(s['source'])} ({e(str(s['name']))}) — {status}</li>")
    parts.append("</ul>")
    if idx.gaps():
        parts.append(f"<p><b>Gaps:</b> {e(str(idx.gaps()))}</p>")
    if asset:
        parts.append("<h2>History</h2><table border=1><tr><th>ledger</th><th>price</th>"
                     "<th>sources</th><th>recomputed</th><th>provenance</th></tr>")
        for h in idx.history(asset):
            prov = ", ".join(f"{p['source']}={p['price']}"
                             for p in idx.provenance(asset, h["ledger"], h["event_index"]))
            parts.append(f"<tr><td>{h['ledger']}</td><td>{h['price']}</td><td>{h['num_sources']}</td>"
                         f"<td>{h['recomputed']}</td><td>{e(prov)}</td></tr>")
        parts.append("</table>")
    return "".join(parts)


def serve(db: str, port: int) -> None:
    idx = Indexer(db)

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):  # noqa: N802
            u = urlparse(self.path)
            code, ctype, body = route(idx, u.path, parse_qs(u.query))
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.end_headers()
            self.wfile.write(body.encode())

    HTTPServer(("127.0.0.1", port), Handler).serve_forever()


if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("--db", default="oracle-index.db")
    p.add_argument("--port", type=int, default=8088)
    a = p.parse_args()
    serve(a.db, a.port)
