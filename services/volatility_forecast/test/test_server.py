from __future__ import annotations

import json
import threading
import urllib.error
import urllib.request

from services.common.synthetic import generate_price_series
from services.volatility_forecast.server import ForecastHandler, ForecastState, load_price_history, run_server
from http.server import HTTPServer


def _start_server() -> tuple[HTTPServer, str]:
    events = generate_price_series(asset="DEMO", n_points=200, interval_secs=3600, seed=1)
    history = load_price_history(events)
    ForecastHandler.state = ForecastState(price_history=history, interval_secs=3600)

    httpd = HTTPServer(("127.0.0.1", 0), ForecastHandler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    port = httpd.server_address[1]
    return httpd, f"http://127.0.0.1:{port}"


def test_forecast_endpoint_serves_a_valid_forecast():
    httpd, base_url = _start_server()
    try:
        with urllib.request.urlopen(f"{base_url}/forecast?asset=DEMO&horizon_hours=24&confidence=0.90") as resp:
            assert resp.status == 200
            body = json.loads(resp.read())
        assert body["asset"] == "DEMO"
        assert body["lower_bound"] < body["last_price"] < body["upper_bound"]
        assert body["sample_size"] == 200
    finally:
        httpd.shutdown()


def test_forecast_endpoint_404s_on_unknown_asset():
    httpd, base_url = _start_server()
    try:
        try:
            urllib.request.urlopen(f"{base_url}/forecast?asset=NOPE")
            assert False, "expected an HTTPError"
        except urllib.error.HTTPError as exc:
            assert exc.code == 404
    finally:
        httpd.shutdown()


def test_metrics_endpoint_counts_forecasts_served():
    httpd, base_url = _start_server()
    try:
        urllib.request.urlopen(f"{base_url}/forecast?asset=DEMO").read()
        urllib.request.urlopen(f"{base_url}/forecast?asset=DEMO").read()
        metrics = urllib.request.urlopen(f"{base_url}/metrics").read().decode()
        assert "oracle_volatility_forecasts_total 2" in metrics
    finally:
        httpd.shutdown()


def test_dashboard_root_serves_html():
    httpd, base_url = _start_server()
    try:
        with urllib.request.urlopen(base_url + "/") as resp:
            assert resp.status == 200
            assert b"Volatility Forecast" in resp.read()
    finally:
        httpd.shutdown()
