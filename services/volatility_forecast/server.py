#!/usr/bin/env python3
"""Runs the volatility-forecasting service: an HTTP endpoint returning
realized + forecast volatility with a confidence window, plus a small
dashboard view. See docs/volatility-forecasting.md.

Usage:
    python -m services.volatility_forecast.server --demo
    python -m services.volatility_forecast.server --events-file events.jsonl --port 9102
"""
from __future__ import annotations

import argparse
import json
import logging
from dataclasses import asdict
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Dict, List, Optional
from urllib.parse import parse_qs, urlparse

from services.common.events import EventSource, TOPIC_PRICE_AGGREGATED, iter_from_postgres, iter_aggregations
from services.common.synthetic import generate_price_series
from services.volatility_forecast.forecast import DEFAULT_CONFIDENCE, VolatilityForecast, forecast

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger("volatility-forecast.server")

_DASHBOARD_HTML = """<!doctype html>
<title>Oracle Volatility Forecast</title>
<style>
  body { font-family: system-ui, sans-serif; margin: 2rem; color: #1a1a1a; }
  input, select, button { font-size: 1rem; padding: 0.25rem 0.5rem; margin-right: 0.5rem; }
  table { border-collapse: collapse; margin-top: 1.5rem; }
  td, th { border: 1px solid #ccc; padding: 0.4rem 0.8rem; text-align: left; }
  .band { color: #555; }
</style>
<h1>Oracle Volatility Forecast</h1>
<div>
  <label>Asset <input id="asset" value="DEMO"></label>
  <label>Horizon (h) <input id="horizon" type="number" value="24"></label>
  <label>Confidence
    <select id="confidence">
      <option value="0.90" selected>90%</option>
      <option value="0.95">95%</option>
      <option value="0.99">99%</option>
    </select>
  </label>
  <button onclick="load()">Forecast</button>
</div>
<table id="out" hidden>
  <tr><th>Last price</th><td id="last_price"></td></tr>
  <tr><th>Realized volatility (annualized)</th><td id="realized_volatility"></td></tr>
  <tr><th>Forecast volatility (annualized, EWMA)</th><td id="forecast_volatility"></td></tr>
  <tr><th>Confidence window</th><td class="band" id="band"></td></tr>
  <tr><th>Sample size</th><td id="sample_size"></td></tr>
</table>
<p id="err" style="color:#b00"></p>
<script>
async function load() {
  const asset = document.getElementById('asset').value;
  const horizon = document.getElementById('horizon').value;
  const confidence = document.getElementById('confidence').value;
  const err = document.getElementById('err');
  err.textContent = '';
  const res = await fetch(`/forecast?asset=${encodeURIComponent(asset)}&horizon_hours=${horizon}&confidence=${confidence}`);
  const body = await res.json();
  if (!res.ok) { err.textContent = body.error || 'request failed'; document.getElementById('out').hidden = true; return; }
  document.getElementById('last_price').textContent = body.last_price;
  document.getElementById('realized_volatility').textContent = (body.realized_volatility * 100).toFixed(2) + '%';
  document.getElementById('forecast_volatility').textContent = (body.forecast_volatility * 100).toFixed(2) + '%';
  document.getElementById('band').textContent = `${body.lower_bound.toFixed(2)} – ${body.upper_bound.toFixed(2)}`;
  document.getElementById('sample_size').textContent = body.sample_size;
  document.getElementById('out').hidden = false;
}
</script>
"""


class ForecastState:
    def __init__(self, price_history: Dict[str, List[float]], interval_secs: int):
        self.price_history = price_history
        self.interval_secs = interval_secs
        self.forecasts_total = 0


def load_price_history(source: EventSource) -> Dict[str, List[float]]:
    history: Dict[str, List[float]] = {}
    for event in iter_aggregations(source):
        history.setdefault(event.asset, []).append(float(event.price))
    return history


class ForecastHandler(BaseHTTPRequestHandler):
    state: Optional[ForecastState] = None

    def do_GET(self):  # noqa: N802 - required by BaseHTTPRequestHandler
        parsed = urlparse(self.path)
        if parsed.path == "/":
            self._respond(200, "text/html", _DASHBOARD_HTML.encode())
        elif parsed.path == "/forecast":
            self._handle_forecast(parse_qs(parsed.query))
        elif parsed.path == "/metrics":
            body = f"oracle_volatility_forecasts_total {self.state.forecasts_total}\n"
            self._respond(200, "text/plain", body.encode())
        elif parsed.path == "/health":
            self._respond(200, "application/json", b'{"status": "ok"}')
        else:
            self.send_response(404)
            self.end_headers()

    def _handle_forecast(self, query: Dict[str, List[str]]) -> None:
        asset = query.get("asset", [None])[0]
        horizon_hours = float(query.get("horizon_hours", ["24"])[0])
        confidence = float(query.get("confidence", [str(DEFAULT_CONFIDENCE)])[0])

        prices = self.state.price_history.get(asset, []) if asset else []
        if not asset or not prices:
            self._respond(404, "application/json", json.dumps({"error": f"no history for asset {asset!r}"}).encode())
            return

        result = forecast(
            asset,
            prices,
            interval_secs=self.state.interval_secs,
            horizon_secs=int(horizon_hours * 3600),
            confidence=confidence,
        )
        if result is None:
            self._respond(422, "application/json", json.dumps({"error": "insufficient price history"}).encode())
            return

        self.state.forecasts_total += 1
        self._respond(200, "application/json", json.dumps(asdict(result)).encode())

    def _respond(self, code: int, content_type: str, body: bytes) -> None:
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):  # silence default request logging
        pass


def run_server(port: int, state: ForecastState) -> None:
    ForecastHandler.state = state
    log.info("volatility forecast endpoint + dashboard listening on :%d", port)
    HTTPServer(("0.0.0.0", port), ForecastHandler).serve_forever()


def main() -> None:
    parser = argparse.ArgumentParser(description="Oracle off-chain volatility forecasting service")
    parser.add_argument("--events-file", help="JSONL file of event envelopes (price_aggregated topic)")
    parser.add_argument("--postgres-dsn", help="Postgres DSN to read from the oracle_events table")
    parser.add_argument("--demo", action="store_true", help="serve against a synthetic price series")
    parser.add_argument("--interval-secs", type=int, default=3600, help="seconds between price observations")
    parser.add_argument("--port", type=int, default=9102)
    args = parser.parse_args()

    if args.demo:
        events = generate_price_series(asset="DEMO", n_points=500, interval_secs=args.interval_secs, seed=42)
    elif args.postgres_dsn:
        events = iter_from_postgres(args.postgres_dsn, topics={TOPIC_PRICE_AGGREGATED})
    elif args.events_file:
        events = args.events_file
    else:
        parser.error("one of --events-file, --postgres-dsn, or --demo is required")
        return

    history = load_price_history(events)
    log.info("loaded price history for %d asset(s)", len(history))
    state = ForecastState(price_history=history, interval_secs=args.interval_secs)
    run_server(args.port, state)


if __name__ == "__main__":
    main()
