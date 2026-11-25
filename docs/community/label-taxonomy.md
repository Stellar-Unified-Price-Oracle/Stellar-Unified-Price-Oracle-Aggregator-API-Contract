# Repository Label Taxonomy

**Status:** Active | Issue: #880

Standardized taxonomy for labeling issues and pull requests across the Stellar Unified Price Oracle repositories.

## 1. Area Labels (`area/*`)

Indicates architectural subsystem or domain:
- `area/contract`: Core Soroban smart contract logic and state storage.
- `area/aggregator`: Price calculation, median selection, and statistical filters.
- `area/circuit-breaker`: Volatility detection, anomaly filters, and emergency halts.
- `area/sdk`: Python, TypeScript, and Rust client bindings.
- `area/ops`: Kubernetes manifests, GitHub Actions workflows, and DR drill scripts.
- `area/docs`: Runbooks, architectural ADRs, and roadmap documentation.

## 2. Priority Labels (`priority/*`)

Defines urgency and resolution timeframe:
- `priority/P0`: Blocker. Active outage, data corruption, or critical security flaw.
- `priority/P1`: High urgency. Scheduled for current milestone or upcoming release.
- `priority/P2`: Normal priority. Standard roadmap deliverables and improvements.
- `priority/P3`: Low priority / Nice-to-have. Backlogged ideas and refactorings.

## 3. Effort Size Labels (`size/*`)

Estimates estimated complexity and review scope:
- `size/XS`: < 50 lines changed, trivial typo or config fix.
- `size/S`: 50 - 200 lines changed, single function or test fix.
- `size/M`: 200 - 500 lines changed, discrete feature or refactoring.
- `size/L`: 500 - 1000 lines changed, major subsystem modification.
- `size/XL`: > 1000 lines changed, architectural overhaul.
