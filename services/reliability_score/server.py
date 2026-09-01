#!/usr/bin/env python3
"""Runs the source-reliability-scoring service: computes per-source
reliability scores from indexed events and serves them over HTTP, plus a
dashboard view. See docs/source-reliability-score.md.

Usage:
    python -m services.reliability_score.server --demo
    python -m services.reliability_score.server --events-file events.jsonl --port 9103
"""
from __future__ import annotations

import argparse
import json
import logging
from dataclasses import asdict
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import List, Optional
from urllib.parse import parse_qs, urlparse

from services.common.events import TOPIC_PRICE_SUBMITTED, iter_from_postgres, iter_submissions
from services.common.synthetic import generate_submissions
from services.reliability_score.scorer import (
    ScoreWeights,
    SourceReliabilityScore,
    compute_scores,
    derive_aggregations,
    get_source_score,
)

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger("reliability-score.server")

_DASHBOARD_HTML = """<!doctype html>
<title>Oracle Source Reliability Scores</title>
<style>
  body { font-family: system-ui, sans-serif; margin: 2rem; color: #1a1a1a; }
  input, button { font-size: 1rem; padding: 0.25rem 0.5rem; margin-right: 0.5rem; }
  table { border-collapse: collapse; margin-top: 1.5rem; width: 100%; max-width: 700px; }
  td, th { border: 1px solid #ccc; padding: 0.4rem 0.8rem; text-align: right; }
  th:first-child, td:first-child { text-align: left; }
</style>
<h1>Oracle Source Reliability Scores</h1>
<div>
  <label>Asset <input id="asset" value="DEMO"></label>
  <button onclick="load()">Load</button>
</div>
<table id="out" hidden>
  <thead><tr><th>Source</th><th>Composite</th><th>Uptime</th><th>Freshness</th><th>Accuracy</th><th>Rounds</th></tr></thead>
  <tbody id="rows"></tbody>
</table>
<p id="err" style="color:#b00"></p>
<script>
async function load() {
  const asset = document.getElementById('asset').value;
  const err = document.getElementById('err');
  err.textContent = '';
  const res = await fetch(`/scores?asset=${encodeURIComponent(asset)}`);
  const body = await res.json();
  if (!res.ok) { err.textContent = body.error || 'request failed'; document.getElementById('out').hidden = true; return; }
  const rows = document.getElementById('rows');
  rows.innerHTML = '';
  for (const s of body.sort((a, b) => b.composite_score - a.composite_score)) {
    const tr = document.createElement('tr');
    tr.innerHTML = `<td>${s.source}</td><td>${s.composite_score.toFixed(1)}</td>` +
      `<td>${s.uptime_score.toFixed(1)}</td><td>${s.freshness_score.toFixed(1)}</td>` +
      `<td>${s.accuracy_score.toFixed(1)}</td><td>${s.rounds_participated}/${s.rounds_expected}</td>`;
    rows.appendChild(tr);
  }
  document.getElementById('out').hidden = false;
}
</script>
"""


class ScoreState:
    def __init__(self, scores: List[SourceReliabilityScore]):
        self.scores = scores
        self.scores_computed_total = len(scores)


def _compute_state(submissions: List, weights: ScoreWeights) -> ScoreState:
    aggregations = derive_aggregations(submissions)
    scores = compute_scores(submissions, aggregations, weights=weights)
    return ScoreState(scores)


class ScoreHandler(BaseHTTPRequestHandler):
    state: Optional[ScoreState] = None

    def do_GET(self):  # noqa: N802 - required by BaseHTTPRequestHandler
        parsed = urlparse(self.path)
        query = parse_qs(parsed.query)
        if parsed.path == "/":
            self._respond(200, "text/html", _DASHBOARD_HTML.encode())
        elif parsed.path == "/scores":
            self._handle_scores(query)
        elif parsed.path == "/score":
            self._handle_score(query)
        elif parsed.path == "/metrics":
            body = f"oracle_reliability_scores_computed_total {self.state.scores_computed_total}\n"
            self._respond(200, "text/plain", body.encode())
        elif parsed.path == "/health":
            self._respond(200, "application/json", b'{"status": "ok"}')
        else:
            self.send_response(404)
            self.end_headers()

    def _handle_scores(self, query) -> None:
        asset = query.get("asset", [None])[0]
        matching = [s for s in self.state.scores if asset is None or s.asset == asset]
        self._respond(200, "application/json", json.dumps([asdict(s) for s in matching]).encode())

    def _handle_score(self, query) -> None:
        asset = query.get("asset", [None])[0]
        source = query.get("source", [None])[0]
        if not asset or not source:
            self._respond(400, "application/json", json.dumps({"error": "asset and source are required"}).encode())
            return
        result = get_source_score(self.state.scores, asset, source)
        if result is None:
            self._respond(404, "application/json", json.dumps({"error": "no score for that asset/source"}).encode())
            return
        self._respond(200, "application/json", json.dumps(asdict(result)).encode())

    def _respond(self, code: int, content_type: str, body: bytes) -> None:
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):  # silence default request logging
        pass


def run_server(port: int, state: ScoreState) -> None:
    ScoreHandler.state = state
    log.info("reliability score endpoint + dashboard listening on :%d", port)
    HTTPServer(("0.0.0.0", port), ScoreHandler).serve_forever()


def main() -> None:
    parser = argparse.ArgumentParser(description="Oracle off-chain source reliability scoring service")
    parser.add_argument("--events-file", help="JSONL file of event envelopes (price_submitted topic)")
    parser.add_argument("--postgres-dsn", help="Postgres DSN to read from the oracle_events table")
    parser.add_argument("--demo", action="store_true", help="serve against synthetic submission history")
    parser.add_argument("--port", type=int, default=9103)
    args = parser.parse_args()

    if args.demo:
        events = generate_submissions(
            n_rounds=300,
            sources=("SOURCE_A", "SOURCE_B", "SOURCE_C"),
            noise_bps=10.0,
            seed=1,
            missing={"SOURCE_C": set(range(0, 300, 5))},
            anomalies={i: {"SOURCE_B": 300.0} for i in range(0, 300, 10)},
        )
    elif args.postgres_dsn:
        events = iter_from_postgres(args.postgres_dsn, topics={TOPIC_PRICE_SUBMITTED})
    elif args.events_file:
        events = args.events_file
    else:
        parser.error("one of --events-file, --postgres-dsn, or --demo is required")
        return

    submissions = list(iter_submissions(events))
    log.info("loaded %d submission event(s)", len(submissions))
    state = _compute_state(submissions, ScoreWeights())
    run_server(args.port, state)


if __name__ == "__main__":
    main()
