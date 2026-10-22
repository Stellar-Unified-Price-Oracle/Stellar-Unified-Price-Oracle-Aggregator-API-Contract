"""Canonical form and digest of a privileged transaction payload (#686).

The guard has one job: prove that the bytes about to be signed are the bytes
that were simulated and independently reviewed. That proof needs a
canonical form, because JSON is not a canonical encoding — key order,
integer/string coercion and duplicate fields all let two different payloads
look identical to a human reviewer while a machine reads something else.

So a payload is normalised once, strictly:

* unknown top-level fields are rejected, not ignored, so a release workflow
  cannot add an operation the reviewer never saw;
* every contract argument is **typed** (`{"type": "i128", "value": "..."}`),
  so ``1``, ``"1"`` and ``1.0`` are three different payloads and an argument
  whose type changed is a mismatch, not a silent coercion;
* operations keep their order, and argument names are sorted, so the digest
  covers the exact call sequence.

The digest is SHA-256 over the canonical JSON of that normalised structure.
It is the only thing the approval record trusts.
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from enum import Enum
from typing import Any, List, Mapping, Tuple

PAYLOAD_SCHEMA = "oracle.tx_review.v1"

#: Endpoints that change who controls the contract or how prices are produced.
#: A production payload touching any of these needs the full review guard; an
#: endpoint outside this set is not accepted by the guard at all, so a release
#: workflow cannot approve a call whose effect it does not model.
PRIVILEGED_FUNCTIONS = frozenset(
    {
        "initialize",
        "upgrade",
        "set_admin",
        "pause",
        "unpause",
        "pause_asset",
        "unpause_asset",
        "set_min_sources_required",
        "set_max_history_length",
        "set_max_price_deviation",
        "set_max_sources",
        "set_max_history_per_asset",
        "set_max_events_per_call",
        "set_max_aggregation_sources",
        "set_max_assets",
        "set_max_inactive_ledgers",
        "override_price",
        "remove_price_override",
        "propose_operation",
        "propose_operation_with_priority",
        "execute_operation",
        "cancel_operation",
        "set_timelock_duration",
        "set_admin_op_daily_limit",
    }
)

TYPED_SCALARS = frozenset(
    {"address", "symbol", "string", "bytes", "i128", "i64", "u32", "u64", "u128", "bool", "void"}
)

#: Canonical payload keys; anything else is refused rather than ignored.
PAYLOAD_FIELDS = ("network", "source_account", "fee", "sequence", "max_time", "operations")

MAX_OPERATIONS = 16
MAX_ARGUMENTS = 32


class PayloadCode(str, Enum):
    MALFORMED = "malformed_payload"
    UNKNOWN_FIELD = "unknown_field"
    UNKNOWN_FUNCTION = "unknown_function"
    UNTYPED_ARGUMENT = "untyped_argument"
    UNKNOWN_ARGUMENT_TYPE = "unknown_argument_type"
    ARGUMENT_VALUE = "invalid_argument_value"


class PayloadError(ValueError):
    """A payload that cannot be normalised, so it cannot be reviewed."""

    def __init__(self, code: PayloadCode, field_name: str, detail: str) -> None:
        self.code = code
        self.field = field_name
        self.detail = detail[:160]
        super().__init__(f"{code.value} field={self.field}: {self.detail}")


@dataclass(frozen=True)
class Operation:
    contract_id: str
    function: str
    args: Tuple[Tuple[str, str, Any], ...]  # (name, type, canonical value), sorted by name

    @property
    def privileged(self) -> bool:
        return self.function in PRIVILEGED_FUNCTIONS

    def rendered(self) -> str:
        rendered_args = ", ".join(
            f"{name}={render_typed(type_name, value)}" for name, type_name, value in self.args
        )
        return f"{self.contract_id}::{self.function}({rendered_args})"


@dataclass(frozen=True)
class Payload:
    network: str
    source_account: str
    fee: int
    sequence: int
    max_time: int
    operations: Tuple[Operation, ...]

    @property
    def privileged(self) -> bool:
        return any(op.privileged for op in self.operations)

    def canonical(self) -> List[Any]:
        return [
            self.network,
            self.source_account,
            self.fee,
            self.sequence,
            self.max_time,
            [
                [op.contract_id, op.function, [[n, t, v] for n, t, v in op.args]]
                for op in self.operations
            ],
        ]

    def digest(self) -> str:
        blob = json.dumps(self.canonical(), separators=(",", ":"), ensure_ascii=False)
        return hashlib.sha256(blob.encode("utf-8")).hexdigest()


def render_typed(type_name: str, value: Any) -> str:
    """Stable, unambiguous rendering of one typed argument."""
    if type_name == "vec":
        return "[" + ", ".join(render_typed(t, v) for t, v in value) + "]"
    if type_name == "map":
        return "{" + ", ".join(f"{k}: {render_typed(t, v)}" for k, t, v in value) + "}"
    if type_name == "void":
        return "void"
    if type_name == "bytes":
        return f"bytes:{value}"
    return f"{type_name}:{value}"


def _require_str(value: Any, field_name: str, *, allow_empty: bool = False) -> str:
    if not isinstance(value, str) or (not allow_empty and not value.strip()):
        raise PayloadError(PayloadCode.MALFORMED, field_name, "expected a non-empty string")
    return value


def _require_int(value: Any, field_name: str, *, minimum: int = 0) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise PayloadError(PayloadCode.MALFORMED, field_name, "expected an integer")
    if value < minimum:
        raise PayloadError(PayloadCode.ARGUMENT_VALUE, field_name, f"must be >= {minimum}")
    return value


def _normalise_typed(value: Any, field_name: str) -> Tuple[str, Any]:
    """Normalises one typed value to `(type_name, canonical value)`."""
    if not isinstance(value, Mapping):
        raise PayloadError(
            PayloadCode.UNTYPED_ARGUMENT,
            field_name,
            "argument must be {'type': ..., 'value': ...}, not a bare value",
        )
    extra = set(value) - {"type", "value"}
    if extra:
        raise PayloadError(PayloadCode.UNKNOWN_FIELD, field_name, "argument carries unknown fields")
    type_name = value.get("type")
    if not isinstance(type_name, str):
        raise PayloadError(PayloadCode.UNTYPED_ARGUMENT, field_name, "argument has no type")
    if "value" not in value:
        raise PayloadError(PayloadCode.ARGUMENT_VALUE, field_name, "argument has no value")

    inner = value["value"]

    if type_name in ("i128", "i64", "u32", "u64", "u128"):
        minimum = 0 if type_name.startswith("u") else None
        if isinstance(inner, bool):
            raise PayloadError(PayloadCode.ARGUMENT_VALUE, field_name, f"{type_name} is not a bool")
        if isinstance(inner, str):
            # A decimal string is accepted only if it is exactly the integer it
            # claims to be; the canonical form is always the integer, so the
            # digest cannot differ between "10" and 10.
            try:
                inner = int(inner, 10)
            except ValueError:
                raise PayloadError(
                    PayloadCode.ARGUMENT_VALUE, field_name, f"{type_name} is not an integer"
                ) from None
        if isinstance(inner, bool) or not isinstance(inner, int):
            raise PayloadError(PayloadCode.ARGUMENT_VALUE, field_name, f"{type_name} is not an integer")
        if minimum is not None and inner < minimum:
            raise PayloadError(PayloadCode.ARGUMENT_VALUE, field_name, f"{type_name} must be >= 0")
        return type_name, inner

    if type_name == "bool":
        if not isinstance(inner, bool):
            raise PayloadError(PayloadCode.ARGUMENT_VALUE, field_name, "bool is not a boolean")
        return type_name, inner

    if type_name in ("address", "symbol", "string"):
        if not isinstance(inner, str) or (type_name != "string" and not inner.strip()):
            raise PayloadError(
                PayloadCode.ARGUMENT_VALUE, field_name, f"{type_name} must be a non-empty string"
            )
        return type_name, inner

    if type_name == "bytes":
        if not isinstance(inner, str):
            raise PayloadError(
                PayloadCode.ARGUMENT_VALUE, field_name, "bytes must be a base64/hex string"
            )
        return type_name, inner

    if type_name == "void":
        if inner is not None:
            raise PayloadError(PayloadCode.ARGUMENT_VALUE, field_name, "void takes no value")
        return type_name, None

    if type_name == "vec":
        if not isinstance(inner, list):
            raise PayloadError(PayloadCode.ARGUMENT_VALUE, field_name, "vec must be a list")
        return type_name, [_normalise_typed(item, field_name) for item in inner]

    if type_name == "map":
        if not isinstance(inner, Mapping):
            raise PayloadError(PayloadCode.ARGUMENT_VALUE, field_name, "map must be an object")
        pairs = [
            (str(key), *_normalise_typed(item, field_name)) for key, item in inner.items()
        ]
        return type_name, sorted(pairs, key=lambda p: p[0])

    raise PayloadError(
        PayloadCode.UNKNOWN_ARGUMENT_TYPE,
        field_name,
        "argument type is not one the guard can render",
    )


def normalise(payload: Mapping[str, Any]) -> Payload:
    """Validates and normalises one payload. Raises `PayloadError` on anything odd."""
    if not isinstance(payload, Mapping):
        raise PayloadError(PayloadCode.MALFORMED, "payload", "expected an object")

    schema = payload.get("schema", PAYLOAD_SCHEMA)
    if schema != PAYLOAD_SCHEMA:
        raise PayloadError(PayloadCode.MALFORMED, "schema", "unsupported payload schema")

    unknown = set(payload) - set(PAYLOAD_FIELDS) - {"schema"}
    if unknown:
        # Ignoring an extra field is how a payload gets an operation the
        # reviewers never saw, so this fails closed.
        raise PayloadError(
            PayloadCode.UNKNOWN_FIELD,
            sorted(unknown)[0],
            "payload carries fields the review guard does not cover",
        )

    network = _require_str(payload.get("network"), "network")
    source_account = _require_str(payload.get("source_account"), "source_account")
    fee = _require_int(payload.get("fee", 0), "fee")
    sequence = _require_int(payload.get("sequence"), "sequence", minimum=1)
    max_time = _require_int(payload.get("max_time", 0), "max_time")

    raw_operations = payload.get("operations")
    if not isinstance(raw_operations, list) or not raw_operations:
        raise PayloadError(PayloadCode.MALFORMED, "operations", "payload has no operations")
    if len(raw_operations) > MAX_OPERATIONS:
        raise PayloadError(PayloadCode.MALFORMED, "operations", "payload has too many operations")

    operations: List[Operation] = []
    for position, raw in enumerate(raw_operations):
        field_name = f"operations[{position}]"
        if not isinstance(raw, Mapping):
            raise PayloadError(PayloadCode.MALFORMED, field_name, "operation must be an object")
        extra = set(raw) - {"contract_id", "function", "args"}
        if extra:
            raise PayloadError(PayloadCode.UNKNOWN_FIELD, field_name, "operation carries unknown fields")
        contract_id = _require_str(raw.get("contract_id"), f"{field_name}.contract_id")
        function = _require_str(raw.get("function"), f"{field_name}.function")
        if function not in PRIVILEGED_FUNCTIONS:
            raise PayloadError(
                PayloadCode.UNKNOWN_FUNCTION,
                f"{field_name}.function",
                "function is not modelled by the review guard",
            )
        raw_args = raw.get("args", {})
        if not isinstance(raw_args, Mapping):
            raise PayloadError(PayloadCode.MALFORMED, f"{field_name}.args", "args must be an object")
        if len(raw_args) > MAX_ARGUMENTS:
            raise PayloadError(PayloadCode.MALFORMED, f"{field_name}.args", "too many arguments")
        args = tuple(
            sorted(
                (
                    (name, *_normalise_typed(value, f"{field_name}.args.{name}"))
                    for name, value in raw_args.items()
                ),
                key=lambda item: item[0],
            )
        )
        operations.append(Operation(contract_id=contract_id, function=function, args=args))

    return Payload(
        network=network,
        source_account=source_account,
        fee=fee,
        sequence=sequence,
        max_time=max_time,
        operations=tuple(operations),
    )


def digest_of(payload: Mapping[str, Any]) -> str:
    """Convenience: normalise then digest."""
    return normalise(payload).digest()


def load_payload(data: Any) -> Payload:
    """Accepts a JSON object or a JSON string and returns the normalised payload."""
    if isinstance(data, str):
        try:
            data = json.loads(data)
        except json.JSONDecodeError as error:
            raise PayloadError(PayloadCode.MALFORMED, "payload", "payload is not valid JSON") from error
    return normalise(data)
