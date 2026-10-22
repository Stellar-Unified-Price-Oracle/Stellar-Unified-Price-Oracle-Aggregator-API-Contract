# Proposal Simulation & Worst-Case Impact (#545)

Module: `contracts/price-oracle/src/proposal_impact.rs`

`attach(proposal_id, base_state, deltas)` validates each delta against the parameter
registry, simulates it on a copy of `EconState`, and stores a `SimulationReport` against
the proposal. `require_cleared(proposal_id)` is the execution gate:
- no report → `SimulationRequired` (168) — applies to **every** proposal;
- touches a critical parameter (`quorum`, `fee`, `min_sources`, `max_deviation`) and is
  flagged harmful → `ProposalHarmful` (169). There is no bypass path.

## Worst-case model and assumptions
| Check | Rule | Assumption / sensitivity |
|---|---|---|
| Quorum feasibility | `5000 < quorum ≤ 10000 − WORST_CASE_DISSENT_BPS` | up to 10% of power absent/dissenting; raising this tightens the upper bound 1:1 |
| Source sufficiency | `active_sources ≥ min_sources` | current source set does not grow |
| Fee shock | fee rise ≤ `MAX_FEE_RISE_BPS` (50%) | consumer demand inelastic |
| Source incentives | per-source income drop ≤ `MAX_SOURCE_INCOME_DROP_BPS` (30%) | income = fee·queries/sources, linear in fee |

All deltas are applied together, so multi-parameter interactions (e.g. fee cut plus
`min_sources` raise) are evaluated jointly. Example harmful proposal: `quorum = 9500`
→ infeasible under worst-case dissent → flagged and blocked.
