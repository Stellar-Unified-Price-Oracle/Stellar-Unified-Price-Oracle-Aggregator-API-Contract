from http.server import BaseHTTPRequestHandler, HTTPServer
from threading import Thread

from opentelemetry import trace

from services.observability import configure_telemetry, start_span


def test_traceparent_continues_and_exports_trace(monkeypatch):
    received = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):  # noqa: N802
            received.append(self.rfile.read(int(self.headers["Content-Length"])))
            self.send_response(200)
            self.end_headers()

        def log_message(self, fmt, *args):
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    thread = Thread(target=server.serve_forever)
    thread.start()
    monkeypatch.setenv(
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        f"http://127.0.0.1:{server.server_port}/v1/traces",
    )
    provider = configure_telemetry("oracle-indexer-test")
    try:
        with start_span(
            "oracle.indexer.event",
            {"oracle.round": 42, "oracle.asset": "XLM"},
            "00-0123456789abcdef0123456789abcdef-0123456789abcdef-01",
        ):
            context = trace.get_current_span().get_span_context()
            assert context.trace_id == int("0123456789abcdef0123456789abcdef", 16)
    finally:
        provider.shutdown()
        server.shutdown()
        thread.join()
        server.server_close()

    assert received
