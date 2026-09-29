# ADR-005: Cross-Reference Design

## Status

Accepted

## Context

A single oracle source set can drift or collude. Independent external oracles provide a
cross-check without trusting the primary aggregator’s internal median.

## Decision

- Reference oracles registered as contract addresses under `ReferenceOracle(address)`.
- Deviation threshold stored in `CrossRefDeviationThreshold`.
- Cross-reference results emitted as `CrossRefDeviationEvent` when exceeded.
- DEX/AMM price readers feed the same aggregation pipeline with configurable weight.

## Consequences

- Consumers may weight primary vs cross-reference data by asset risk profile.
- Extra reads increase gas; cross-reference is configurable per asset.
- Reference oracles are permissioned by admin to avoid spam.
