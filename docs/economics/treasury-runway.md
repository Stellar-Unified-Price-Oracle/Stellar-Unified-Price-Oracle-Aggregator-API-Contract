# Treasury Runway and Funding Scenarios

This document models the oracle's operational spend and committed funding so that
sustainability decisions are explicit, reviewable, and verifiable. It covers the
baseline, low-usage, and high-incident scenarios required by issue #668.

## Assumptions

All figures are monthly unless stated otherwise. They are planning estimates, not
committed contracts; update them when actuals diverge by more than 20%.

| Item | Value | Notes |
| --- | --- | --- |
| Treasury balance at model start | 60,000 XLM | Committed, unencumbered funds |
| Oracle operator / on-call stipend | 2,400 XLM | 1 primary + 1 backup, part-time |
| Infrastructure (RPC, nodes, storage) | 1,800 XLM | Baseline provisioned capacity |
| Price feed / data provider fees | 900 XLM | Baseline feed subscriptions |
| Security review & audits (amortized) | 700 XLM | Annual audit spread monthly |
| Tooling, monitoring, incident response | 500 XLM | Alerting, dashboards, runbooks |
| **Baseline operational spend** | **6,300 XLM** | Sum of the above |
| Committed funding (grants/retainers) | 4,000 XLM | Recurring committed inflow |
| Net baseline burn | 2,300 XLM | Spend minus committed funding |

## Scenario definitions

- **Baseline** — normal operation at planned capacity and usage.
- **Low-usage** — demand drops; some variable costs fall but fixed costs remain.
- **High-incident** — elevated incident volume requiring surge on-call, extra
  infrastructure, and emergency response.

## Runway per scenario

Runway = treasury balance / net monthly burn.

| Scenario | Monthly spend | Committed funding | Net burn | Runway |
| --- | --- | --- | --- | --- |
| Baseline | 6,300 XLM | 4,000 XLM | 2,300 XLM | ~26.1 months |
| Low-usage | 5,400 XLM | 3,200 XLM | 2,200 XLM | ~27.3 months |
| High-incident | 11,300 XLM | 4,000 XLM | 7,300 XLM | ~8.2 months |

Low-usage reduces variable costs (data feeds, some infra) but fixed stipends and
amortized audits keep the burn close to baseline. High-incident adds surge
on-call, extra infrastructure, and emergency response, roughly doubling spend.

## Funding review threshold

**Trigger a funding review when projected runway falls below 12 months in any
scenario, or when the high-incident scenario runway falls below 9 months.**

At the baseline balance of 60,000 XLM this means:

- Baseline net burn above ~5,000 XLM/month triggers review.
- High-incident net burn above ~6,700 XLM/month triggers review.

When triggered, the operator must open a funding review issue documenting the
driver, the updated assumptions, and a remediation plan (cost reduction, new
committed funding, or scope adjustment) before the next monthly close.

## Boundary and failure/recovery paths

- **Boundary:** runway exactly at the 12-month threshold is treated as a trigger
  (fail-safe: review rather than defer).
- **Failure:** if committed funding lapses, net burn rises to full spend; baseline
  runway drops to ~9.5 months, which immediately triggers review.
- **Recovery:** restoring committed funding or cutting variable spend back to
  baseline returns runway above threshold; record the recovery in the review issue.

## Repeatable check

Recompute the tables above from the assumptions whenever spend or funding changes
and confirm the threshold logic still holds. The numbers here are the reference
values a reviewer compares against; any divergence must be explained in the
funding review issue.
