"""Sustained soak / load rig for the oracle ingest path (#523).

Headline load tests measure a burst. This rig measures the *slog*: a long,
seeded run of a realistic-plus-adversarial submission mix whose latency,
memory and on/off-chain state growth are tracked per round and asserted
against ceilings. A deliberately leaky state model is shipped alongside the
real one so the "unbounded growth is detected" claim is demonstrated rather
than asserted.

See docs/soak-rig.md for the workload mix, the distributions, the ceilings and
the scheduled run.
"""
