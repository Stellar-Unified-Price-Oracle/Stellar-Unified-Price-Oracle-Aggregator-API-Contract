# ADR-004: Relayer System

## Status

Accepted

## Context

Off-chain relayers can submit prices on behalf of sources, but the contract must prevent
replay, Sybil spam, and unauthorized fee claims.

## Decision

- Approved relayers stored in `ApprovedRelayer(address)`.
- Per-submission fee credited to `RelayerFeeBalance(address)`.
- Relayer nonces tracked per source for replay protection.
- Bonded relayer variants (see `relayer_bonds.rs`) require stake before approval.

## Consequences

- Relayer integration is opt-in per source.
- Bonded relayers add capital cost, improving spam resistance.
- Fee sweep and dashboard reads expose relayer economics off-chain.
