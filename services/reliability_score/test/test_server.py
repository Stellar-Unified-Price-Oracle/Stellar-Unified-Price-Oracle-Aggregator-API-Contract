from __future__ import annotations

import json
import threading
import urllib.error
import urllib.request
from http.server import HTTPServer

from services.common.events import iter_submissions
from services.common.synthetic import generate_submissions
from services.reliability_score.scorer import ScoreWeights
from services.reliability_score.server import ScoreHandler, _compute_state


def _start_server() -> tuple[HTTPServer, str, str]:
    events = generate_submissions(n_rounds=50, sources=("A", "B"), seed=1)
    asset = events[0]["data"]["asset"]
    submissions = list(iter_submissions(events))
    ScoreHandler.state = _compute_state(submissions, ScoreWeights())

    httpd = HTTPServer(("127.0.0.1", 0), ScoreHandler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    port = httpd.server_address[1]
    return httpd, f"http://127.0.0.1:{port}", asset


def test_scores_endpoint_lists_every_source_for_the_asset():
    httpd, base_url, asset = _start_server()
    try:
        with urllib.request.urlopen(f"{base_url}/scores?asset={asset}") as resp:
            body = json.loads(resp.read())
        assert {row["source"] for row in body} == {"A", "B"}
    finally:
        httpd.shutdown()


def test_score_endpoint_returns_a_single_source_get_source_score():
    httpd, base_url, asset = _start_server()
    try:
        with urllib.request.urlopen(f"{base_url}/score?asset={asset}&source=A") as resp:
            body = json.loads(resp.read())
        assert body["source"] == "A"
        assert body["asset"] == asset
        assert 0.0 <= body["composite_score"] <= 100.0
    finally:
        httpd.shutdown()


def test_score_endpoint_404s_on_unknown_source():
    httpd, base_url, asset = _start_server()
    try:
        try:
            urllib.request.urlopen(f"{base_url}/score?asset={asset}&source=NOBODY")
            assert False, "expected an HTTPError"
        except urllib.error.HTTPError as exc:
            assert exc.code == 404
    finally:
        httpd.shutdown()


def test_metrics_endpoint_reports_scores_computed():
    httpd, base_url, _ = _start_server()
    try:
        metrics = urllib.request.urlopen(f"{base_url}/metrics").read().decode()
        assert "oracle_reliability_scores_computed_total 2" in metrics
    finally:
        httpd.shutdown()


def test_dashboard_root_serves_html():
    httpd, base_url, _ = _start_server()
    try:
        with urllib.request.urlopen(base_url + "/") as resp:
            assert b"Reliability Scores" in resp.read()
    finally:
        httpd.shutdown()
