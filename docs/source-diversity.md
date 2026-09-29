# Source Diversity — Effective Independence (#399)

> Diversity counts are easy to fake; this metric measures independence, not
> jurisdiction labels.

## Problem

True decentralization requires diversity. Counting distinct strings is
trivially gameable:

- ten sources on one cloud = **one** outage domain with ten names;
- ten legal entities with one common owner/funder = **one** coercion point;
- ten feeds mirroring one upstream = **one** data-compromise point;
- geographic spread is cosmetic when the honest failure is a shared
  dependency, not a shared timezone.

## What `get_source_diversity` returns

`SourceDiversityReport`:

| Field | Meaning |
|---|---|
| `raw_count` | Registered active sources (nominal diversity). |
| `effective_independent_count` | Distinct `(infra, upstream, owner)` triples. The effective count. Always `<= raw`. |
| `region_hhi` / `provider_hhi` / `jurisdiction_hhi` / `infra_hhi` / `upstream_hhi` / `owner_hhi` | Herfindahl concentration per axis, 0–10000. 10000 = full concentration. |
| `overall_score` | `10000 - avg(hhi over 6 axes)`. Higher = more diverse. |
| `largest_domain_size` | Size of the single largest failure-domain group. |
| `is_low_diversity` | `effective < min_effective` OR any axis HHI `> max_hhi_per_axis`. |

Legacy `get_decentralization_report` (#208) is preserved unchanged; new
integrations should use `get_source_diversity`.

## Effective independent count (definition + assumptions)

Two sources are **independent** iff they differ on **ALL** of
(`infra`, `upstream`, `owner`). Formally:

```text
effective = |{ (infra(s), upstream(s), owner(s)) : s in active_set }|
```

### Assumptions (explicit)

1. **Metadata is honestly reported.** The metric trusts admin-attested
   `set_source_geo` / `set_source_diversity` values. A lie in → a lie out.
2. **Active set** = registered sources NOT flagged `SrcInactive` (pure read;
   `get_source_diversity` has no heartbeat side effects and is safe for
   dashboards).
3. **Missing geo** (never set, or pre-#399 triple never re-registered) counts
   as `"unknown"` on every axis — unknowns cluster and *lower* the effective
   count instead of inflating it.
4. **HHI** is the standard sum-of-squared-shares scaled 0–10000.
5. **Defaults**: `min_effective_sources = 3`, `max_hhi_per_axis = 5000`
   (configurable via `set_diversity_thresholds`).

## Worked example (the Sybil trap)

Ten sources, cosmetic geo spread, one shared domain:

| # | region | jurisdiction | infra | upstream | owner |
|---|---|---|---|---|---|
| 1–10 | US/EU/AP/… (all distinct) | US/DE/SG/… (all distinct) | `aws-us-east-1a` (same) | `single-coinbase-feed` (same) | `single-operator-llc` (same) |

- `raw_count = 10` (looks healthy)
- `effective_independent_count = 1` (one cloud + one feed + one operator)
- `infra_hhi = upstream_hhi = owner_hhi = 10000`
- `largest_domain_size = 10`, `is_low_diversity = true`
- `check_diversity_alert()` returns `true` and emits
  `DiversityBreachEvent{raw:10, effective:1, largest:10, …}`

Six genuinely independent sources (distinct infra/upstream/owner each) score
`raw = effective = 6`, `largest_domain_size = 1`, no alert.

## Alerting

- On-chain: `check_diversity_alert() -> bool` emits
  `DiversityBreachEvent` on breach and stamps
  `DiversityLastBreachLedger` (see `get_last_diversity_breach_ledger`).
- Off-chain (Prometheus): `scripts/metrics_exporter.py` exposes
  `oracle_diversity_effective`, `oracle_diversity_raw`,
  `oracle_diversity_max_hhi`, `oracle_diversity_low{}`.
- Prometheus rules (`docs/monitoring/alerts.yml`): `LowEffectiveDiversity`
  fires when `effective < 3` even if `raw >= 5`; `DiversityConcentrationHigh`
  fires when any axis HHI `> 5000`.
- Grafana: `docs/monitoring/grafana-dashboard-v2-sources.json` gains a
  *Source Diversity* row (raw vs effective + per-axis HHI + breach annotations).
- Automated source removal is **out of scope** (see #402) — alerts never
  remove sources.

## What a given value does and does NOT prove

A high `effective_independent_count` with low HHIs proves the *registered
metadata* describes distinct failure domains. It does **NOT** prove:

1. **Metadata truthfulness** — covert shared control, shell entities, or a
   false infra/upstream label are invisible to this metric. Pair with
   verification (#226) and audits.
2. **No off-chain collusion** — distinct operators can still collude,
   copy-trade, or run identical code with identical bugs.
3. **Upstream truth** — diverse mirrors of one compromised origin still
   agree on the wrong price. Cross-reference checks (#172-family) are separate.
4. **Network / timing independence** — shared ISP, BGP, relayer, or ledger
   timing correlation is not modeled.
5. **Liveness / quality** — a diverse set can still be stale, low-reputation,
   or suspended. Combine with health, heartbeat (#186), and reputation (#171).
6. **Future stability** — the report is a point-in-time snapshot; a domain
   can consolidate after measurement (re-check before critical actions).

Do not over-trust the number. Treat `effective_independent_count` as a
*necessary but not sufficient* condition for decentralization.
