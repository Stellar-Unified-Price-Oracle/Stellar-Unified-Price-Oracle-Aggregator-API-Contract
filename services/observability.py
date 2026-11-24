"""Optional OpenTelemetry setup shared by off-chain services."""
from __future__ import annotations

import os
from contextlib import nullcontext


def configure_telemetry(service_name: str):
    """Configure OTLP export when OTEL_EXPORTER_OTLP_ENDPOINT is set."""
    endpoint = os.environ.get("OTEL_EXPORTER_OTLP_ENDPOINT")
    if not endpoint:
        return None

    from opentelemetry import trace
    from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter
    from opentelemetry.sdk.resources import Resource
    from opentelemetry.sdk.trace import TracerProvider
    from opentelemetry.sdk.trace.export import BatchSpanProcessor

    provider = TracerProvider(resource=Resource.create({"service.name": service_name}))
    provider.add_span_processor(BatchSpanProcessor(OTLPSpanExporter(endpoint=endpoint)))
    trace.set_tracer_provider(provider)
    return provider


def start_span(name: str, attributes: dict, traceparent: str | None = None):
    if not os.environ.get("OTEL_EXPORTER_OTLP_ENDPOINT"):
        return nullcontext()

    from opentelemetry import trace
    from opentelemetry.trace.propagation.tracecontext import TraceContextTextMapPropagator

    carrier = {"traceparent": traceparent} if traceparent else {}
    parent = TraceContextTextMapPropagator().extract(carrier)
    return trace.get_tracer("stellar-oracle").start_as_current_span(
        name, context=parent, attributes=attributes
    )
