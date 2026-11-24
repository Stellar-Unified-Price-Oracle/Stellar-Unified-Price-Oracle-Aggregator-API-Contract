import { OTLPTraceExporter } from "@opentelemetry/exporter-trace-otlp-http";
import { resourceFromAttributes } from "@opentelemetry/resources";
import { NodeSDK } from "@opentelemetry/sdk-node";

export async function startTelemetry() {
  const endpoint = process.env.OTEL_EXPORTER_OTLP_ENDPOINT;
  if (!endpoint) return async () => {};

  const sdk = new NodeSDK({
    resource: resourceFromAttributes({ "service.name": "oracle-event-indexer" }),
    traceExporter: new OTLPTraceExporter({ url: endpoint }),
  });
  await sdk.start();
  return () => sdk.shutdown();
}
