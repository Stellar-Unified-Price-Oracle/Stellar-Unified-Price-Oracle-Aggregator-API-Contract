# Price Scaling Helpers

Prices are stored as `i128` integers scaled by `10^decimals`. These helpers convert
between human-readable values and on-chain integers with round-half-up.

**JavaScript / TypeScript**

```ts
const DECIMALS = 7;
const SCALE = 10n ** BigInt(DECIMALS);
const I128_MAX = (1n << 127n) - 1n;
const I128_MIN = -(1n << 127n);

// Round-half-up on a rational (numerator / denominator).
function roundHalfUp(numerator: bigint, denominator: bigint): bigint {
  const q = numerator / denominator;
  const r = numerator % denominator;
  if (r === 0n) return q;
  const absR = r < 0n ? -r : r;
  const bump = absR * 2n >= denominator ? 1n : 0n;
  return numerator < 0n ? q - bump : q + bump;
}

function toInteger(human: number): bigint {
  // Convert via string to avoid float precision loss.
  const [whole, frac = ""] = human.toString().split(".");
  const fracPadded = (frac + "0".repeat(DECIMALS)).slice(0, DECIMALS);
  const numerator = BigInt(whole + fracPadded);
  const scaled = roundHalfUp(numerator, 1n);
  if (scaled > I128_MAX || scaled < I128_MIN) {
    throw new Error("price out of i128 range");
  }
  return scaled;
}

function toHuman(integer: bigint): string {
  const neg = integer < 0n;
  const abs = neg ? -integer : integer;
  const whole = abs / SCALE;
  const frac = (abs % SCALE).toString().padStart(DECIMALS, "0");
  return `${neg ? "-" : ""}${whole}.${frac}`;
}

// Examples (decimals = 7)
console.assert(toInteger(0.0000001) === 1n);
console.assert(toInteger(1.2345678) === 12345678n);
console.assert(toHuman(12345678n) === "1.2345678");
```

**Python**

```python
from decimal import Decimal, ROUND_HALF_UP

DECIMALS = 7
SCALE = 10 ** DECIMALS
I128_MAX = (1 << 127) - 1
I128_MIN = -(1 << 127)


def to_integer(human: Decimal) -> int:
    scaled = (human * SCALE).quantize(Decimal(1), rounding=ROUND_HALF_UP)
    value = int(scaled)
    if value > I128_MAX or value < I128_MIN:
        raise ValueError("price out of i128 range")
    return value


def to_human(integer: int) -> Decimal:
    return Decimal(integer) / SCALE


# Examples (decimals = 7)
assert to_integer(Decimal("0.0000001")) == 1
assert to_integer(Decimal("1.2345678")) == 12345678
assert to_integer(Decimal("0.00000005")) == 1  # round-half-up
assert to_human(12345678) == Decimal("1.2345678")
```
