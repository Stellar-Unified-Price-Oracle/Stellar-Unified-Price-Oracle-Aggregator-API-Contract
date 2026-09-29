# ADR-001: Aggregation Method Selection

## Status

Accepted

## Context

The oracle must combine multiple price submissions into a single aggregate. We evaluated
median, mean, trimmed mean, and VWAP against manipulation resistance, gas cost, and
predictability for on-chain consumers.

## Decision

Use **median** as the default aggregation method, with configurable alternatives:

- `0` median
- `1` mean
- `2` trimmed mean
- `3` VWAP

Median is selected because it tolerates up to 50 % compromised sources without
shifting, matches existing on-chain sorting utilities, and keeps gas bounded.

## Consequences

- Consumers can switch methods via `set_aggregation_method`.
- Median dominates baseline tests; alternative paths are regression-tested separately.
- VWAP requires volume data; missing volumes fall back to equal-weight median.
