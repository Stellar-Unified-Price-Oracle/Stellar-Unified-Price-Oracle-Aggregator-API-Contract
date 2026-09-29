# Event Schema Registry

The contract emits Soroban events (see `contracts/price-oracle/src/events.rs`)
for every state-changing operation. This registry provides versioned,
structured schemas for the core events consumers most commonly subscribe to,
so off-chain indexers can validate and parse event payloads deterministically.

Schemas live under [`schemas/events/v1/`](../schemas/events/v1/) as one
JSON Schema file per event, named `<event_topic>.schema.json`. Avro
equivalents (for Kafka/streaming consumers) live alongside them as
`<event_topic>.avsc`.

## Versioning

* Schemas are versioned by directory (`schemas/events/v1/`, `v2/`, ...).
* A new version directory is added only on a **breaking** change (field
  removed, type changed, semantics changed). Additive, backward-compatible
  fields are added to the current version.
* Each event struct's Rust definition in `events.rs` is the source of
  truth; schema changes must accompany the corresponding Rust change in the
  same PR.

## Core events (v1)

| Topic | Rust struct | Schema |
|---|---|---|
| `price_submitted` | `PriceSubmittedEvent` | [schema](../schemas/events/v1/price_submitted.schema.json) |
| `price_aggregated` | `PriceAggregatedEvent` | [schema](../schemas/events/v1/price_aggregated.schema.json) |
| `source_added` | `SourceAddedEvent` | [schema](../schemas/events/v1/source_added.schema.json) |
| `source_removed` | `SourceRemovedEvent` | [schema](../schemas/events/v1/source_removed.schema.json) |
| `asset_registered` | `AssetRegisteredEvent` | [schema](../schemas/events/v1/asset_registered.schema.json) |
| `circuit_breaker_tripped` | `CircuitBreakerTrippedEvent` | [schema](../schemas/events/v1/circuit_breaker_tripped.schema.json) |

Other event types not yet listed here follow the same struct-to-schema
convention; open a PR adding a `<topic>.schema.json` / `.avsc` pair under
`schemas/events/v1/` and a row to the table above when you need one
formalized.

## Example consumer (JavaScript, JSON Schema validation)

```js
import Ajv from "ajv";
import schema from "./schemas/events/v1/price_submitted.schema.json" assert { type: "json" };

const ajv = new Ajv();
const validate = ajv.compile(schema);

function handleEvent(rawEvent) {
  if (!validate(rawEvent)) {
    throw new Error(`Invalid price_submitted event: ${ajv.errorsText(validate.errors)}`);
  }
  // rawEvent is now known to match the v1 price_submitted schema
}
```

## Example consumer (Python, Avro/streaming)

```python
from fastavro import schemaless_reader
import io

with open("schemas/events/v1/price_submitted.avsc") as f:
    import json
    schema = json.load(f)

def decode(payload: bytes):
    return schemaless_reader(io.BytesIO(payload), schema)
```
