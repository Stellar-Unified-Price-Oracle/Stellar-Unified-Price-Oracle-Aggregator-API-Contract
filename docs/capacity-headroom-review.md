# Capacity Headroom Review from Observed Traffic (#704)

Static and synthetic capacity projections must be validated against real-world observed production telemetry. Without regular headroom reviews grounded in observed traffic, capacity limits are discovered during severe traffic spikes or network volatility, leading to service degradation.

This document defines the methodology, bottleneck categories, quantitative expansion triggers, and remediation playbooks evaluated by [`services/capacity/headroom_review.py`](../services/capacity/headroom_review.py).

---

## 1. The Four Primary Bottlenecks

Production traffic stresses four distinct resource dimensions. Each has an explicit measurement unit, capacity limit, and accountable team owner:

| Bottleneck | Monitored Unit | Default Capacity Limit | Accountable Team Owner |
|---|---|---|---|
| **RPC** | Requests per second (sustained / peak) | $500\text{ req/s}$ | `infra-ops` |
| **Compute** | Pipeline CPU cores (sustained / peak) | $8.0\text{ cores}$ | `core-contracts` |
| **Storage** | Retained volume GB (current & 90-day projection) | $250\text{ GB}$ | `infra-ops` |
| **Operator Staffing** | Paging alerts per on-call week | $10\text{ pages/week}$ | `oracle-oncall` |

---

## 2. Headroom Calculations & Expansion Triggers

For each bottleneck, headroom ratios are computed for sustained and peak traffic profiles:

$$\text{Headroom}_{\text{peak}} = \frac{\text{Capacity} - \text{Observed}_{\text{peak}}}{\text{Capacity}}$$

### Trigger Levels

* **Healthy ($\ge 30\%$ Headroom)**: Normal operating margin. No expansion required.
* **Warning Trigger ($< 30\%$ Headroom)**: Approaching threshold. Schedule review in current quarterly cycle; optimize hot spots.
* **Critical Expansion Trigger ($< 15\%$ Headroom)**: Immediate risk of exhaustion during unexpected spikes. Trigger automated remediation playbook and expedite infrastructure expansion within 14 days.

---

## 3. Remediation Playbooks

When a trigger fires, the accountable team executes the documented playbook:

### RPC Bottleneck (Owner: `infra-ops`)
1. Provision secondary and fallback RPC provider endpoints with automated failover routing.
2. Enable client-side query batching and response caching for static asset configurations.
3. Segregate high-volume read query traffic from transaction submission pipes.
4. Upgrade provider subscription tier to increase rate caps and burst allowances.

### Compute Bottleneck (Owner: `core-contracts`)
1. Horizontally scale worker execution pods and verification worker pools.
2. Enable signature verification fast-path caching for previously verified source public keys.
3. Distribute batch aggregation tasks across dedicated CPU affinity cores.
4. Profile hot spots and optimize cryptographic decoding and sorting routines.

### Storage Bottleneck (Owner: `infra-ops`)
1. Trigger automatic database compaction, VACUUM, and WAL truncation on SQLite/Postgres backends.
2. Optimize Soroban smart contract storage TTL extension frequencies.
3. Archive historical quotes older than 90 days to cold object stores (GCS/S3).
4. Resize persistent volume claims before disk saturation crosses 85%.

### Operator Staffing Bottleneck (Owner: `oracle-oncall`)
1. Conduct alert audit to silence non-actionable or flapping warnings.
2. Introduce split secondary on-call rotation to absorb daytime triage load.
3. Implement self-healing runbook automation for recurring transient errors.
4. Review on-call compensation, shift lengths, and team rotation size.

---

## 4. Operational Review Cadence

Headroom reviews are performed continuously and formally audited:
* **Weekly Automated Evaluation**: CI and monitoring scripts run `services.capacity.headroom_review --fail-on-trigger` against recent rolling telemetry.
* **Quarterly Formal Audit**: The `core-contracts` and `infra-ops` leads conduct a quarterly review to recalibrate baseline capacities, examine 90-day storage trends, and commit the updated review report.

---

## 5. Tool Usage & Automation

Generate a Markdown review report from observed traffic parameters:

```bash
python -m services.capacity.headroom_review \
  --rpc-sustained-rps 150 \
  --rpc-peak-rps 320 \
  --compute-peak-cores 5.2 \
  --storage-current-gb 80 \
  --storage-growth-gb 1.5 \
  --pages-per-week 3 \
  --report docs/capacity-review-latest.md
```

Evaluate observed telemetry from an automated metrics dump with failure on critical expansion:

```bash
python -m services.capacity.headroom_review --input telemetry.json --fail-on-trigger --json
```
