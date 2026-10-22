# Asset Metadata Provenance and Change History (#705)

When an oracle asset's configuration changes—such as decimals, ticker symbol, authorized price feed sources, source weighting distribution, staleness timeouts, or maximum deviation thresholds—the system must preserve a permanent, tamper-evident audit trail. Without verifiable metadata provenance, disputes over historical settlement prices cannot distinguish between oracle misconfiguration, unauthorized state modification, and valid governance updates.

The asset metadata provenance subsystem is implemented in [`services/provenance/asset_metadata.py`](../services/provenance/asset_metadata.py).

---

## 1. Asset Metadata Configuration Schema

Each asset's configuration consists of the operational parameters governing price ingestion and aggregation:

| Parameter | Type | Validation Rules | Description |
|---|---|---|---|
| `decimals` | Integer | $0 \le \text{decimals} \le 18$ | Price quote fixed-point precision exponent |
| `ticker` | String | $1 \le \text{len} \le 12$ | Ticker symbol (e.g. `XLM`, `USDC`) |
| `asset_type` | String | `native` / `credit_alphanum4` / `credit_alphanum12` | Stellar asset representation type |
| `sources` | List[String] | At least 1 valid source account | Authorized oracle feed source accounts |
| `source_weights_bps` | List[Integer] | Sum must equal $10,000\text{ bps}$ ($100\%$) | Relative weights applied in weighted median/mean aggregation |
| `staleness_threshold_seconds` | Integer | $> 0$ (default $300\text{ s}$) | Max age before an incoming source quote is discarded |
| `deviation_threshold_bps` | Integer | $> 0$ (default $500\text{ bps}$) | Max allowable deviation from unweighted reference median |

---

## 2. Revision Record & Audit Trail

Every modification creates an immutable `AssetMetadataRevision` containing:

1. **Prior and New Config Diffs**: The exact field-level state transition (`prior_config` $\rightarrow$ `config`).
2. **Accountability**:
   - `proposer`: Identity or operator role proposing the update (e.g. `operator:governance-admin`).
   - `reviewer`: Reviewing multisig committee or secondary approver (e.g. `approver:multisig-signers`).
   - `evidence_url`: Verifiable reference (governance vote link, issuer announcement, RFC, or postmortem).
   - `rationale`: Human-readable justification for the parameter change.
3. **Execution Coordinates**:
   - `effective_ledger`: Specific Stellar ledger sequence number where update took effect.
   - `effective_timestamp`: Unix epoch timestamp of activation.

---

## 3. Cryptographic Tamper-Evident Hash Chain

Revisions for each asset form an append-only hash chain:

$$H_0 = \text{SHA-256}(\text{"0"}^{64} \parallel \text{CanonicalJSON}(\text{Genesis Revision}))$$
$$H_n = \text{SHA-256}(H_{n-1} \parallel \text{CanonicalJSON}(\text{Revision } n))$$

* **Domain Separator**: `ASSET_META_V1` is embedded into each canonical hash commitment to prevent cross-protocol collisions.
* **Strict Monotonicity**: Revisions require strictly increasing ledger sequences ($L_n > L_{n-1}$) and non-decreasing timestamps ($T_n \ge T_{n-1}$).
* **Tamper Verification**: Calling `verify_provenance_chain(asset_id)` recomputes every commitment and validates that $H_{n-1}$ exactly equals `previous_hash` of revision $n$. Any unauthorized edit fails immediately.

---

## 4. Point-in-Time Historical Queries

To resolve trade disputes or audit historical oracle contracts, consumers and off-chain indexers can query asset metadata at any point in history:

* `get_metadata_at_ledger(asset_id, ledger)`: Reconstructs the exact operational configuration active during that ledger block.
* `get_metadata_at_timestamp(asset_id, timestamp)`: Reconstructs configuration active at that specific UTC second.

---

## 5. CLI Usage & Audit Export

Run chain integrity verification and generate an audit report:

```bash
python -m services.provenance.asset_metadata --asset XLM --action verify
```

Export complete change history as structured JSON:

```bash
python -m services.provenance.asset_metadata --asset XLM --json
```

Generate a formatted Markdown audit log with diffs:

```bash
python -m services.provenance.asset_metadata --asset XLM --report docs/xlm-metadata-audit.md
```
