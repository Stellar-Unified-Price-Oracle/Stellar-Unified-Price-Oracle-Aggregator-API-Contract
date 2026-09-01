#!/usr/bin/env python3
"""Runs the off-chain anomaly-detection service: streams oracle submission
events through `StreamingDetector` and serves recent alerts plus Prometheus
metrics over HTTP, so it plugs into the existing docs/monitoring stack the
same way scripts/price-submission-bot.py does.

Usage:
    python -m services.anomaly_detection.server --demo
    python -m services.anomaly_detection.server --events-file events.jsonl
    python -m services.anomaly_detection.server --postgres-dsn postgresql://... --webhook-url https://...

See docs/anomaly-detection.md for configuration and integration details.
"""
from __future__ import annotations

import argparse
import json
import logging
from dataclasses import asdict
from http.server import BaseHTTPRequestHandler, HTTPServer
from threading import Lock, Thread
from typing import List, Optional

from services.anomaly_detection.detector import (
    AlertSink,
    AnomalyAlert,
    DEFAULT_MAD_THRESHOLD,
    DetectorConfig,
    LoggingAlertSink,
    StreamingDetector,
    WebhookAlertSink,
)
from services.common.events import iter_from_postgres
from services.common.events import TOPIC_PRICE_SUBMITTED
from services.common.synthetic import generate_submissions

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger("anomaly-detector.server")

_MAX_RECENT_ALERTS = 500
_alerts_lock = Lock()
_recent_alerts: List[AnomalyAlert] = []


class RecentAlertsSink(AlertSink):
    """Keeps the most recent alerts in memory for the `/alerts` endpoint."""

    def emit(self, alert: AnomalyAlert) -> None:
        with _alerts_lock:
            _recent_alerts.append(alert)
            if len(_recent_alerts) > _MAX_RECENT_ALERTS:
                del _recent_alerts[: len(_recent_alerts) - _MAX_RECENT_ALERTS]


class MetricsHandler(BaseHTTPRequestHandler):
    detector: Optional[StreamingDetector] = None

    def do_GET(self):  # noqa: N802 - required by BaseHTTPRequestHandler
        if self.path == "/metrics":
            lines = [
                f"oracle_anomaly_alerts_total {self.detector.alerts_emitted}",
                f"oracle_anomaly_rounds_scored_total {self.detector.rounds_scored}",
            ]
            self._respond(200, "text/plain", ("\n".join(lines) + "\n").encode())
        elif self.path == "/alerts":
            with _alerts_lock:
                payload = [asdict(a) for a in _recent_alerts]
            self._respond(200, "application/json", json.dumps(payload).encode())
        elif self.path == "/health":
            self._respond(200, "application/json", b'{"status": "ok"}')
        else:
            self.send_response(404)
            self.end_headers()

    def _respond(self, code: int, content_type: str, body: bytes) -> None:
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):  # silence default request logging
        pass


def run_metrics_server(port: int, detector: StreamingDetector) -> None:
    MetricsHandler.detector = detector
    HTTPServer(("0.0.0.0", port), MetricsHandler).serve_forever()


def build_sinks(webhook_url: Optional[str]) -> List[AlertSink]:
    sinks: List[AlertSink] = [LoggingAlertSink(), RecentAlertsSink()]
    if webhook_url:
        sinks.append(WebhookAlertSink(webhook_url))
    return sinks


def main() -> None:
    parser = argparse.ArgumentParser(description="Oracle off-chain anomaly-detection service")
    parser.add_argument("--events-file", help="JSONL file of event envelopes to replay")
    parser.add_argument("--postgres-dsn", help="Postgres DSN to stream events from the oracle_events table")
    parser.add_argument("--demo", action="store_true", help="run against seeded synthetic anomalies and exit")
    parser.add_argument("--webhook-url", default=None, help="optional webhook to POST each alert to")
    parser.add_argument("--port", type=int, default=9101, help="metrics/alerts HTTP port")
    parser.add_argument("--mad-threshold", type=float, default=DEFAULT_MAD_THRESHOLD)
    args = parser.parse_args()

    config = DetectorConfig(mad_threshold=args.mad_threshold)
    detector = StreamingDetector(config=config, sinks=build_sinks(args.webhook_url))

    if args.demo:
        events = generate_submissions(
            n_rounds=100,
            sources=("SOURCE_A", "SOURCE_B", "SOURCE_C", "SOURCE_D"),
            anomalies={40: {"SOURCE_B": 900.0}, 75: {"SOURCE_D": -1200.0}},
        )
        alerts = detector.run(events)
        log.info("demo run complete: %d alert(s) from %d round(s)", len(alerts), detector.rounds_scored)
        for alert in alerts:
            print(json.dumps(asdict(alert)))
        return

    Thread(target=run_metrics_server, args=(args.port, detector), daemon=True).start()
    log.info("anomaly detector metrics/alerts endpoint listening on :%d", args.port)

    if args.postgres_dsn:
        source = iter_from_postgres(args.postgres_dsn, topics={TOPIC_PRICE_SUBMITTED})
    elif args.events_file:
        source = args.events_file
    else:
        parser.error("one of --events-file, --postgres-dsn, or --demo is required")
        return

    detector.run(source)


if __name__ == "__main__":
    main()
