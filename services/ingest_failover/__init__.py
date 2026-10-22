"""Multi-region RPC/ingest redundancy with health-checked failover (#526).

A single-region ingest path is a single point of failure for the whole oracle.
This module is the off-chain half of the fix — a region router that health
checks each region, fails over automatically, and guarantees that a failover
does **not** double-submit:

* **Health** — each region is probed and marked healthy/unhealthy with a
  consecutive-failure threshold and a cooldown, so one blip does not trigger a
  flapping failover and a recovering region is not hammered.
* **Failover** — the router always has a healthy region; a total outage is
  reported as such rather than silently dropped.
* **Idempotency** — every submission carries a deterministic key derived from
  ``(source, asset, ledger)``. A shared dedupe ledger, replicated to every
  region, means the same logical submission attempted in region A and retried
  in region B is submitted to chain **once**. This is the split-brain
  double-submit guard the issue calls out.
* **Observability** — metrics and alerts for region degradation and failover,
  emitted as Prometheus text (rules in ``docs/monitoring/alerts-ingest.yml``).

No on-chain contract changes: this is the off-chain ingest/RPC path only.

See ``docs/multi-region-ingest.md``.
"""
